//! Copies of the window's own records.
//!
//! The window keeps conversations, workflows, settings and its add-ons in one
//! folder; a wrong delete or a broken save would take them with it. A backup is
//! a plain folder copy under `backups/`, and an export is one conversation
//! written as Markdown under `exports/`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The folders a backup carries.
///
/// The workspace is deliberately left out: it holds the files the assistant
/// worked on, which are the user's own project and can be gigabytes.
const CARRIED: [&str; 4] = ["data", "skills", "plugins", "mcpserver"];

/// Folder backups are written into, beside the program.
pub fn backup_root(home: &Path) -> PathBuf {
    home.join("backups")
}

/// Folder exported conversations are written into.
pub fn export_root(home: &Path) -> PathBuf {
    home.join("exports")
}

/// Copy the records into a timestamped folder; returns its path.
pub fn backup(home: &Path) -> Result<PathBuf, String> {
    let target = backup_root(home).join(stamp());
    std::fs::create_dir_all(&target)
        .map_err(|error| format!("无法创建 {}：{error}", target.display()))?;
    for name in CARRIED {
        let source = home.join(name);
        if source.is_dir() {
            copy_tree(&source, &target.join(name))?;
        }
    }
    let config = home.join("config.json");
    if config.is_file() {
        std::fs::copy(&config, target.join("config.json"))
            .map_err(|error| format!("无法复制 {}：{error}", config.display()))?;
    }
    Ok(target)
}

/// Write one conversation as Markdown; returns the file.
pub fn export_markdown(home: &Path, name: &str, body: &str) -> Result<PathBuf, String> {
    let folder = export_root(home);
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("无法创建 {}：{error}", folder.display()))?;
    let file = folder.join(format!("{}-{}.md", safe_name(name), stamp()));
    std::fs::write(&file, body).map_err(|error| format!("无法写入 {}：{error}", file.display()))?;
    Ok(file)
}

/// Copy `source` into `target`, recursively.
pub fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|error| format!("无法创建 {}：{error}", target.display()))?;
    let entries = std::fs::read_dir(source)
        .map_err(|error| format!("无法读取 {}：{error}", source.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let destination = target.join(entry.file_name());
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            copy_tree(&path, &destination)?;
        } else if kind.is_file() {
            std::fs::copy(&path, &destination)
                .map_err(|error| format!("无法复制 {}：{error}", path.display()))?;
        } else if kind.is_symlink() {
            // A link is saved as the text it points at, not followed: following
            // one could copy the whole disk.
            let linked = std::fs::read_link(&path)
                .map_err(|error| format!("无法读取链接 {}：{error}", path.display()))?;
            let _ = std::fs::write(&destination, linked.display().to_string());
        }
    }
    Ok(())
}

/// A folder-safe name for one conversation.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "chat".to_string()
    } else {
        trimmed.chars().take(40).collect()
    }
}

/// `YYYYMMDD-HHMMSS` in UTC, so two backups never share a folder.
fn stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since the Unix epoch → (year, month, day), UTC.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-flow-ui-backup-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_copy_takes_the_whole_tree() {
        let root = temp_dir("copy");
        let source = root.join("data");
        std::fs::create_dir_all(source.join("chats").join("chat-1")).unwrap();
        std::fs::write(source.join("chats").join("chat-1").join("chat.json"), "{}").unwrap();
        std::fs::write(source.join("sidebar.json"), "{}").unwrap();

        copy_tree(&source, &root.join("copy")).unwrap();
        let copied = root.join("copy");
        assert!(copied.join("sidebar.json").is_file());
        assert!(copied.join("chats").join("chat-1").join("chat.json").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_backup_carries_the_records_but_not_the_workspace() {
        let home = temp_dir("backup");
        std::fs::create_dir_all(home.join("data").join("chats")).unwrap();
        std::fs::write(home.join("data").join("sidebar.json"), "{}").unwrap();
        std::fs::write(home.join("config.json"), "{}").unwrap();
        std::fs::create_dir_all(home.join("workspace").join("chat-1")).unwrap();
        std::fs::write(home.join("workspace").join("chat-1").join("a.txt"), "x").unwrap();

        let target = backup(&home).unwrap();
        assert!(target.join("data").join("sidebar.json").is_file());
        assert!(target.join("config.json").is_file());
        assert!(!target.join("workspace").exists(), "the project is not copied");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_export_lands_in_the_export_folder() {
        let home = temp_dir("export");
        let file = export_markdown(&home, "我的 对话/一", "# 标题\n\n正文\n").unwrap();
        assert_eq!(file.parent(), Some(export_root(&home).as_path()));
        assert!(
            file.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("我的-对话-一-")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "# 标题\n\n正文\n");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_stamp_is_a_sortable_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-01-01 is 10957 days after the epoch.
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        let stamp = stamp();
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert_eq!(&stamp[8..9], "-");
    }
}
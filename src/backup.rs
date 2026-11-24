//! Copies of the window's own records.
//!
//! The window keeps conversations, workflows, settings and its add-ons in one
//! folder; a wrong delete or a broken save would take them with it. A backup is
//! a plain folder copy under `backups/`, and an export is one conversation
//! written as Markdown under `exports/`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The add-on folders a backup carries beside the data directory.
const CARRIED: [&str; 3] = ["skills", "plugins", "mcpserver"];

/// File that records the backup the next launch should restore.
const PENDING_RESTORE: &str = "restore-pending.txt";

/// A completed restore, for the window to report.
pub struct RestoreReport {
    /// Backup folder the records came from.
    pub source: PathBuf,
    /// Safety backup taken before the records were replaced.
    pub safety: PathBuf,
}

/// Folder backups are written into, beside the program.
pub fn backup_root(home: &Path) -> PathBuf {
    home.join("backups")
}

/// Folder exported conversations are written into.
pub fn export_root(home: &Path) -> PathBuf {
    home.join("exports")
}

/// Copy the records into a timestamped folder; returns its path.
///
/// `data` is the directory the settings currently point at, which need not be
/// under `home`; copying `home/data` blindly would miss every conversation when
/// the user moved it.
pub fn backup(home: &Path, data: &Path) -> Result<PathBuf, String> {
    let base = stamp();
    let mut target = backup_root(home).join(&base);
    // Two backups can be requested inside the same second; never reuse (and
    // overwrite) the folder a planned restore may be reading.
    let mut suffix = 0;
    while target.exists() {
        suffix += 1;
        target = backup_root(home).join(format!("{base}-{suffix}"));
    }
    std::fs::create_dir_all(&target)
        .map_err(|error| format!("无法创建 {}：{error}", target.display()))?;
    if data.is_dir() {
        copy_tree(data, &target.join("data"))?;
    }
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

/// The newest folder under `backups/`, or `None` before the first backup.
pub fn latest_backup(home: &Path) -> Option<PathBuf> {
    let root = backup_root(home);
    let entries = std::fs::read_dir(root).ok()?;
    let mut newest: Option<(String, PathBuf)> = None;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // The folder name starts with the sortable UTC stamp; a repeated second
        // appends `-1`, `-2`, … and is still newer than the base one.
        if name.len() < 15 || name.as_bytes().get(8).is_none_or(|byte| *byte != b'-') {
            continue;
        }
        if newest.as_ref().is_none_or(|(current, _)| name > *current) {
            newest = Some((name, entry.path()));
        }
    }
    newest.map(|(_, path)| path)
}

/// Write the newest backup as the one the next launch should restore.
///
/// The records are not touched now: the live window still has them in memory,
/// and saving a conversation after a live restore would overwrite the restored
/// file. Applying the plan at startup keeps that race out of the design.
pub fn plan_restore(home: &Path) -> Result<PathBuf, String> {
    let source = latest_backup(home).ok_or_else(|| "还没有可恢复的备份".to_string())?;
    let pending = home.join(PENDING_RESTORE);
    std::fs::write(&pending, source.display().to_string())
        .map_err(|error| format!("无法写入恢复计划 {}：{error}", pending.display()))?;
    Ok(source)
}

/// The backup planned for the next launch, if it still exists.
pub fn pending_restore(home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(home.join(PENDING_RESTORE)).ok()?;
    let source = PathBuf::from(text.trim());
    let root = backup_root(home);
    (source.is_dir() && source.starts_with(&root)).then_some(source)
}

/// Forget a planned restore.
pub fn cancel_restore(home: &Path) -> Result<(), String> {
    let pending = home.join(PENDING_RESTORE);
    if !pending.is_file() {
        return Ok(());
    }
    std::fs::remove_file(&pending)
        .map_err(|error| format!("无法取消恢复计划 {}：{error}", pending.display()))
}

/// Apply a planned restore, after taking a safety backup of the current state.
///
/// Only the records are replaced: `config.json` keeps the current connection and
/// path settings, so the restored data lands where the running app already looks
/// for it. The return value names both folders for the startup banner.
pub fn apply_pending_restore(home: &Path, data: &Path) -> Result<Option<RestoreReport>, String> {
    let Some(source) = pending_restore(home) else {
        let _ = cancel_restore(home);
        return Ok(None);
    };
    let safety = backup(home, data)?;
    let restored_data = source.join("data");
    if restored_data.is_dir() {
        replace_tree(&restored_data, data)?;
    }
    for name in CARRIED {
        let from = source.join(name);
        if from.is_dir() {
            replace_tree(&from, &home.join(name))?;
        }
    }
    cancel_restore(home)?;
    Ok(Some(RestoreReport { source, safety }))
}

/// Replace `target` with a copy of `source`, never merging two record sets.
fn replace_tree(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        std::fs::remove_dir_all(target)
            .map_err(|error| format!("无法替换 {}：{error}", target.display()))?;
    }
    copy_tree(source, target)
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
        let data = home.join("data");
        std::fs::create_dir_all(data.join("chats")).unwrap();
        std::fs::write(data.join("sidebar.json"), "{}").unwrap();
        std::fs::write(home.join("config.json"), "{}").unwrap();
        std::fs::create_dir_all(home.join("workspace").join("chat-1")).unwrap();
        std::fs::write(home.join("workspace").join("chat-1").join("a.txt"), "x").unwrap();

        let target = backup(&home, &data).unwrap();
        assert!(target.join("data").join("sidebar.json").is_file());
        assert!(target.join("config.json").is_file());
        assert!(!target.join("workspace").exists(), "the project is not copied");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_backup_follows_the_data_directory_outside_home() {
        let home = temp_dir("outside-home");
        let data = temp_dir("outside-data");
        std::fs::create_dir_all(data.join("chats")).unwrap();
        std::fs::write(data.join("sidebar.json"), "{}").unwrap();

        let target = backup(&home, &data).unwrap();
        assert!(target.join("data").join("sidebar.json").is_file());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_planned_restore_replaces_the_records_and_keeps_a_safety_copy() {
        let home = temp_dir("restore-home");
        let data = home.join("data");
        std::fs::create_dir_all(data.join("chats")).unwrap();
        std::fs::write(data.join("marker.txt"), "before").unwrap();
        let backup_path = backup(&home, &data).unwrap();

        // Change the live records after the backup.
        std::fs::write(data.join("marker.txt"), "after").unwrap();
        let planned = plan_restore(&home).unwrap();
        assert_eq!(planned, backup_path);
        assert!(pending_restore(&home).is_some());

        let report = apply_pending_restore(&home, &data).unwrap().expect("restore");
        assert_eq!(std::fs::read_to_string(data.join("marker.txt")).unwrap(), "before");
        assert!(report.safety.is_dir());
        assert!(pending_restore(&home).is_none());
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
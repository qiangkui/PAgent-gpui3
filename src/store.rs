//! Helpers shared by the record folders.
//!
//! Conversations and workflows are both stored as one folder per item with a
//! JSON file inside. A save runs after every small change, so a file is only
//! written when its text really changed. A folder whose item is gone is
//! removed, but only when this run has read or written it: an unreadable record
//! is not proof that nobody needs it.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Write `body` unless the file already holds it.
///
/// The comparison is against what is really on disk, not against what this run
/// last wrote: a file edited from outside must not be skipped as "unchanged",
/// or the window and its records would disagree with no way to notice.
pub fn write_if_changed(file: &Path, body: &str) -> Result<(), String> {
    let digest = digest_bytes(body.as_bytes());
    if let Ok(on_disk) = std::fs::read(file)
        && digest_bytes(&on_disk) == digest
    {
        remember(&key(file), digest);
        return Ok(());
    }
    write_atomic(file, body).map_err(|error| format!("无法保存 {}：{error}", file.display()))?;
    remember(&key(file), digest);
    Ok(())
}

/// Note what a file holds, without writing it.
///
/// A file that was read from disk counts as known from then on, so the folder
/// around it may be removed when its item is deleted, and a save that changes
/// nothing leaves it alone.
pub fn remember_file(file: &Path, body: &str) {
    remember(&key(file), digest_bytes(body.as_bytes()));
}

/// Whether this run has read or written the file.
pub fn known(file: &Path) -> bool {
    cached(&key(file)).is_some()
}

/// Drop a file that was removed.
pub fn forget(file: &Path) {
    if let Ok(mut map) = written().lock() {
        map.remove(&key(file));
    }
}

/// Replace `file` in one step: a crash mid-write leaves the old file whole.
pub fn write_atomic(file: &Path, body: &str) -> std::io::Result<()> {
    let temporary = temporary_path(file);
    std::fs::write(&temporary, body)?;
    std::fs::rename(&temporary, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// The sibling file a write goes through first.
///
/// The whole name is kept and `.tmp` appended: `with_extension` would replace
/// the extension instead, turning an extension-less file into the wrong name.
fn temporary_path(file: &Path) -> std::path::PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".tmp");
    std::path::PathBuf::from(name)
}

/// The sub-folders of `root`, sorted by name.
pub fn folder_names(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// A folder name for an item called `name`, made safe and unique.
///
/// The name is only a place to keep the file; the item's own name is stored
/// inside, so a rename here never changes what the record means.
pub fn folder_name(name: &str, used: &[String]) -> String {
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
    let base = accept_base(cleaned.trim_matches('-'));
    if !used.iter().any(|seen| seen == &base) {
        return base;
    }
    let mut number = 2;
    loop {
        let candidate = format!("{base}-{number}");
        if !used.iter().any(|seen| seen == &candidate) {
            return candidate;
        }
        number += 1;
    }
}

/// A base name the filesystem will actually create as a folder.
///
/// A trailing dot and the device names (`con`, `nul`, `com1`, …) are refused by
/// Windows, so a card named after one would never get a folder at all.
fn accept_base(name: &str) -> String {
    let trimmed = name.trim_end_matches('.');
    let base = if trimmed.is_empty() { "item" } else { trimmed };
    if crate::config::is_reserved_name(base) {
        format!("{base}-item")
    } else {
        base.to_string()
    }
}

/// The cache key for a file: its path as text.
fn key(file: &Path) -> String {
    file.display().to_string()
}

/// A cheap fingerprint of bytes.
fn digest_bytes(body: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    hasher.finish()
}

/// What each file is known to hold, by path.
fn written() -> &'static Mutex<HashMap<String, u64>> {
    static WRITTEN: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    WRITTEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The digest a file is known to hold, if this run read or wrote it.
fn cached(key: &str) -> Option<u64> {
    written().lock().ok()?.get(key).copied()
}

/// Remember what a file now holds.
fn remember(key: &str, digest: u64) {
    if let Ok(mut map) = written().lock() {
        map.insert(key.to_string(), digest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str) -> std::path::PathBuf {
        let file = std::env::temp_dir().join(format!(
            "dsh-flow-ui-store-{}-{tag}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&file);
        file
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_no_temporary() {
        let file = temp_file("write");
        write_atomic(&file, "one").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one");
        write_atomic(&file, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        assert!(!temporary_path(&file).exists(), "the temporary file is gone");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn the_temporary_name_keeps_the_whole_name() {
        use std::path::{Path, PathBuf};
        assert_eq!(temporary_path(Path::new("notes")), PathBuf::from("notes.tmp"));
        assert_eq!(
            temporary_path(Path::new("chat.json")),
            PathBuf::from("chat.json.tmp")
        );
    }

    #[test]
    fn an_unchanged_save_does_not_touch_the_file() {
        let file = temp_file("same");
        write_if_changed(&file, "same").unwrap();
        let stamp = std::fs::metadata(&file).unwrap().modified().unwrap();
        write_if_changed(&file, "same").unwrap();
        let again = std::fs::metadata(&file).unwrap().modified().unwrap();
        let _ = std::fs::remove_file(&file);
        assert_eq!(stamp, again, "the second save skipped the write");
    }

    #[test]
    fn an_external_edit_is_not_taken_for_an_unchanged_save() {
        let file = temp_file("external");
        write_if_changed(&file, "ours").unwrap();
        // Someone edits the file between two saves of the same content.
        std::fs::write(&file, "someone else").unwrap();
        write_if_changed(&file, "ours").unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        assert_eq!(text, "ours", "the save wrote over the foreign content");
    }

    #[test]
    fn a_known_file_is_forgotten_with_its_item() {
        let file = temp_file("known");
        remember_file(&file, "body");
        assert!(known(&file));
        forget(&file);
        assert!(!known(&file));
    }

    #[test]
    fn a_device_name_gets_a_folder_windows_accepts() {
        assert_eq!(folder_name("con", &[]), "con-item");
        assert_eq!(folder_name("NUL", &[]), "NUL-item");
        assert_eq!(folder_name("com1", &[]), "com1-item");
        // A trailing dot is dropped: Windows would refuse the name.
        assert_eq!(folder_name("report.", &[]), "report");
        assert_eq!(folder_name("build", &[]), "build");
        // De-duplication still applies to the adjusted name.
        let used = vec!["con-item".to_string()];
        assert_eq!(folder_name("con", &used), "con-item-2");
    }
}

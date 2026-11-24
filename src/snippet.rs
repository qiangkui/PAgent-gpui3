//! Snippets: short prompts kept as Markdown files beside the program.
//!
//! The composer has no prompt library of its own; a snippet is a `.md` file in
//! `snippets/`, and the diagnostics page lists them so one click inserts its text
//! into the composer.

use std::path::{Path, PathBuf};

use crate::config::AiSettings;

/// Most snippets the page lists.
pub const MAX_SNIPPETS: usize = 64;

/// Longest snippet text that is inserted.
pub const MAX_SNIPPET_CHARS: usize = 20_000;

/// Folder holding the snippets.
pub fn dir() -> PathBuf {
    AiSettings::home().join("snippets")
}

/// Every snippet beside the program, by display name.
pub fn installed() -> Vec<(String, PathBuf)> {
    listed(&dir())
}

/// Every snippet in an arbitrary folder.
pub fn listed(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .filter_map(|entry| {
            let stem = entry.path().file_stem()?.to_string_lossy().to_string();
            let name = stem.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), entry.path()))
        })
        .collect();
    found.sort_by(|left, right| {
        left.0
            .to_lowercase()
            .cmp(&right.0.to_lowercase())
            .then_with(|| left.0.cmp(&right.0))
    });
    found.truncate(MAX_SNIPPETS);
    found
}

/// The text one snippet holds, or `None` when it cannot be read.
pub fn read(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.trim().is_empty() {
        return None;
    }
    let mut kept: String = text.chars().take(MAX_SNIPPET_CHARS).collect();
    if text.chars().count() > MAX_SNIPPET_CHARS {
        kept.push_str("\n…（片段过长，已截断）");
    }
    Some(kept)
}

/// The example snippet's name.
pub const EXAMPLE_NAME: &str = "示例-解释代码.md";

const EXAMPLE_TEXT: &str = "请解释我选中的代码：\n\
\n\
1. 先用一句话说明它做什么。\n\
2. 再逐行说明关键语句，指出隐含假设与可能的坑。\n\
3. 如果发现 bug，先给结论，再给最小修改。\n";

/// Write the example snippet unless it is there already.
///
/// @returns the file and whether it was created.
pub fn create_example(dir: &Path) -> Result<(PathBuf, bool), String> {
    let file = dir.join(EXAMPLE_NAME);
    if file.exists() {
        return Ok((file, false));
    }
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&file, EXAMPLE_TEXT))
        .map_err(|error| format!("无法写入 {}：{error}", file.display()))?;
    Ok((file, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsh-flow-ui-snippet-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_markdown_only_in_name_order() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("b-解释.md"), "乙").unwrap();
        std::fs::write(dir.join("A-测试.md"), "甲").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("folder.md")).unwrap();

        let found = listed(&dir);
        let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(names, vec!["A-测试", "b-解释"]);
    }

    #[test]
    fn reads_a_snippet_without_its_byte_order_mark() {
        let dir = temp_dir("read");
        let file = dir.join("x.md");
        std::fs::write(&file, "\u{feff}先看 README。").unwrap();
        let text = read(&file).expect("text");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(text, "先看 README。");
    }

    #[test]
    fn the_example_is_written_once() {
        let dir = temp_dir("example");
        let (file, created) = create_example(&dir).unwrap();
        assert!(created);
        assert!(file.is_file());
        let (_, again) = create_example(&dir).unwrap();
        assert!(!again);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
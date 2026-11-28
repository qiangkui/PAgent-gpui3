//! Long-term memory the assistant keeps across conversations.
//!
//! The file lives in the data folder next to `config.json`, one entry per line.
//! The window owns it: the engine's `remember` tool only reports an entry, and
//! the window appends it here, so a tool can never write anywhere the window
//! does not manage. Every question then carries the newest entries as a prompt
//! block, which is what makes the memory long-term rather than per-chat.

use std::path::{Path, PathBuf};

/// File name inside the data folder.
pub const FILE: &str = "memory.md";

/// Longest entry the window will append, in characters.
///
/// Mirrors the engine's `MAX_MEMORY_ENTRY`: the tool refuses longer text before
/// it ever reaches the window, and this check keeps a hand-edited or older
/// engine from writing a paragraph into every later prompt.
pub const MAX_ENTRY_CHARS: usize = 1_000;

/// Largest the file may grow. Past this the oldest entries are dropped, so one
/// chatty conversation cannot keep growing every later prompt.
pub const MAX_BYTES: usize = 32 * 1024;

/// Longest text one prompt block may carry, in characters.
pub const PROMPT_CHARS: usize = 8_000;

/// Where the memory file lives.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Read the memory file; a missing or unreadable one reads as empty.
///
/// A memory file is an aid, never a precondition: a broken one must not stop a
/// question from being asked.
pub fn read(data_dir: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path(data_dir)) else {
        return String::new();
    };
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// Append one entry, dropping the oldest lines when the file is full.
///
/// One entry is one line: the list is what the prompt shows, and a multi-line
/// value would break it into entries that were never written.
pub fn append(data_dir: &Path, entry: &str) -> Result<(), String> {
    let text = entry.trim().replace(['\r', '\n'], " ");
    if text.is_empty() {
        return Err("要记的内容是空的".to_string());
    }
    if text.chars().count() > MAX_ENTRY_CHARS {
        return Err(format!(
            "这条记忆太长（{} 字符，上限 {MAX_ENTRY_CHARS}），没有写入",
            text.chars().count()
        ));
    }
    let existing = read(data_dir);
    let mut body = if existing.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", existing.trim_end())
    };
    body.push_str("- ");
    body.push_str(&text);
    body.push('\n');
    let body = trim_to_fit(&body, MAX_BYTES);
    std::fs::create_dir_all(data_dir)
        .map_err(|error| format!("无法创建 {}：{error}", data_dir.display()))?;
    crate::store::write_atomic(&path(data_dir), &body)
        .map_err(|error| format!("无法写入 {}：{error}", path(data_dir).display()))
}

/// Keep the newest whole lines that fit inside `limit` bytes.
///
/// The newest entry always survives, even when it alone is over the limit; the
/// alternative is a memory that silently stops remembering.
fn trim_to_fit(body: &str, limit: usize) -> String {
    if body.len() <= limit {
        return body.to_string();
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for line in body.lines().rev() {
        let cost = line.len() + 1;
        if used + cost > limit && !kept.is_empty() {
            break;
        }
        kept.push(line);
        used += cost;
    }
    kept.reverse();
    format!("{}\n", kept.join("\n"))
}

/// The system-prompt block for the newest entries, or an empty string.
///
/// Capped from the front: the newest memories are the ones a new question is
/// most likely to need, and an earlier entry can be re-learned.
pub fn prompt_text(data_dir: &Path) -> String {
    let text = read(data_dir);
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let mut body = text.to_string();
    let mut note = "";
    let count = body.chars().count();
    if count > PROMPT_CHARS {
        body = body.chars().skip(count - PROMPT_CHARS).collect();
        // Start at a line boundary, so an entry is never cut in half.
        if let Some(index) = body.find('\n') {
            body = body[index + 1..].to_string();
        }
        note = "（更早的记忆已省略，只载入最近的部分）\n";
    }
    format!(
        "{note}长期记忆（`remember` 写入，每次提问都会带上；与本次对话的要求冲突时以本次对话为准）：\n{body}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dsh-flow-ui-memory-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn an_entry_is_appended_as_one_line() {
        let dir = temp_dir("append");
        append(&dir, "先跑 cargo test").unwrap();
        append(&dir, "多行\n要压成一行").unwrap();
        assert_eq!(read(&dir), "- 先跑 cargo test\n- 多行 要压成一行\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_entry_is_refused() {
        let dir = temp_dir("blank");
        assert!(append(&dir, "   ").is_err());
        assert!(!path(&dir).exists(), "nothing was written");
    }

    #[test]
    fn the_prompt_block_names_the_memory_and_is_empty_without_one() {
        let dir = temp_dir("prompt");
        assert_eq!(prompt_text(&dir), "");
        append(&dir, "按 conventions 命名").unwrap();
        let text = prompt_text(&dir);
        assert!(text.contains("长期记忆"), "{text}");
        assert!(text.contains("- 按 conventions 命名"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_oldest_entries_are_dropped_when_the_file_is_full() {
        let dir = temp_dir("trim");
        append(&dir, "旧的记忆").unwrap();
        // Fill past the cap; the newest entry must survive the trimming.
        let long = "新".repeat(MAX_ENTRY_CHARS);
        for _ in 0..40 {
            append(&dir, &long).unwrap();
        }
        let text = read(&dir);
        assert!(text.len() <= MAX_BYTES, "the file stays inside its cap");
        assert!(text.contains(&long), "the newest entry survives");
        assert!(!text.contains("旧的记忆"), "the oldest entry was dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_long_file_is_capped_from_the_front() {
        let dir = temp_dir("cap");
        for index in 0..12 {
            append(&dir, &format!("第 {index} 条{}", "x".repeat(900))).unwrap();
        }
        let text = prompt_text(&dir);
        assert!(text.contains("更早的记忆已省略"), "the cut is explained");
        assert!(text.contains("第 11 条"), "the newest entry is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

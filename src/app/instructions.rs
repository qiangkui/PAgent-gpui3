//! The per-conversation instruction file: reading and writing `AGENTS.md`.
//!
//! Instructions are read with every question (see [`project_rules`]), so the
//! editor only has to keep one chat's file in step with what the settings page
//! shows. The file lives in the conversation's own folder, which is what makes
//! it per-conversation rather than workspace-wide.

use super::*;

/// Read the instructions stored in one conversation folder.
pub(crate) fn read_instructions(dir: &Path) -> String {
    let text = std::fs::read_to_string(dir.join(CHAT_RULES)).unwrap_or_default();
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_string()
}

/// Write one conversation's instructions; blank text removes the file.
///
/// Kept apart from the view so the file rules can be tested without a window.
pub(crate) fn write_instructions(dir: &Path, text: &str) -> Result<String, String> {
    let text = text.trim();
    let file = dir.join(CHAT_RULES);
    if text.is_empty() {
        return match std::fs::remove_file(&file) {
            Ok(()) => Ok("已清除这个对话的指令".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok("这个对话还没有指令".to_string())
            }
            Err(error) => Err(format!("无法删除 {}：{error}", file.display())),
        };
    }
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&file, text.as_bytes()))
        .map(|()| format!("已保存到 {}", file.display()))
        .map_err(|error| format!("无法保存 {}：{error}", file.display()))
}

impl DshFlowApp {
    /// Folder of the conversation the settings page edits.
    pub(crate) fn instructions_dir(&self) -> PathBuf {
        self.chat_dir(self.active_chat)
    }

    /// Read the active conversation's instructions into the editor.
    ///
    /// Called when the instructions tab opens; returning early when the folder
    /// has not changed keeps unsaved typing across a repaint.
    pub(crate) fn load_instructions(&mut self, cx: &mut Context<Self>) {
        let dir = self.instructions_dir();
        let key = dir.display().to_string();
        if self.instructions_chat.as_deref() == Some(key.as_str()) {
            return;
        }
        let text = read_instructions(&dir);
        self.instructions.set_text(text);
        self.instructions_chat = Some(key);
        self.instructions_status = None;
        cx.notify();
    }

    /// Open this conversation's instruction file in the system editor.
    ///
    /// Nothing is written first: opening a file that does not exist yet would
    /// open an empty editor over a path the reader has not saved.
    pub(crate) fn open_instructions_file(&mut self, cx: &mut Context<Self>) {
        let file = self.instructions_dir().join(CHAT_RULES);
        self.instructions_status = Some(if !file.is_file() {
            ("先保存一次，文件才会存在".to_string(), false)
        } else {
            match open_path(&file) {
                Ok(()) => (format!("已打开 {}", file.display()), false),
                Err(error) => (format!("无法打开：{error}"), true),
            }
        });
        cx.notify();
    }

    /// Write the editor to `AGENTS.md`; an empty editor removes the file.
    ///
    /// If the open conversation changed since the text was loaded, the editor is
    /// reloaded instead of writing one chat's rules into another.
    pub(crate) fn save_instructions(&mut self, cx: &mut Context<Self>) {
        let dir = self.instructions_dir();
        let key = dir.display().to_string();
        if self.instructions_chat.as_deref() != Some(key.as_str()) {
            let text = read_instructions(&dir);
            self.instructions.set_text(text);
            self.instructions_chat = Some(key);
            self.instructions_status = Some((
                "对话已切换，已载入当前对话的指令；确认后再保存".to_string(),
                false,
            ));
            cx.notify();
            return;
        }
        let text = self.instructions.text.trim().to_string();
        self.instructions_status = Some(match write_instructions(&dir, &text) {
            Ok(message) => (message, false),
            Err(message) => (message, true),
        });
        cx.notify();
    }
}

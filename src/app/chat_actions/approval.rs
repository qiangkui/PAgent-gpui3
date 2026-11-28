//! Approval decisions: allowing, refusing, remembering and undoing.
//!
//! Kept away from the event pump so the rules that decide a card can be read
//! (and tested) without the transcript plumbing around them.

use super::*;

impl DshFlowApp {
    pub(crate) fn answer_approval(&mut self, index: usize, allow: bool, cx: &mut Context<Self>) {
        let Some(dir) = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.dir.clone())
        else {
            return;
        };
        let Some(message) = self
            .chats
            .get(self.active_chat)
            .and_then(|chat| chat.messages.get(index))
        else {
            return;
        };
        if !matches!(message.role, Role::Approval) || !message.output.is_empty() {
            return;
        }
        let call = message.call.clone();
        if call.is_empty() {
            return;
        }
        let Some(slot) = self.pending.iter().position(|pending| pending.dir == dir) else {
            return;
        };
        if self.pending[slot].approval.as_deref() != Some(call.as_str()) {
            return;
        }
        let delivered = self.pending[slot].request.respond(&call, allow);
        self.pending[slot].approval = None;
        if let Some(message) = self
            .chats
            .get_mut(self.active_chat)
            .and_then(|chat| chat.messages.get_mut(index))
        {
            // A request that already ended cannot take the decision; claiming it
            // was allowed would be a lie the engine never heard.
            message.output = match (delivered, allow) {
                (true, true) => "已允许",
                (true, false) => "已拒绝",
                (false, _) => "已失效",
            }
            .to_string();
            // The decision folds the diff; the header brings it back.
            message.open = false;
        }
        self.save_chats();
        cx.notify();
    }

    /// Allow this command or snippet for the rest of the app run, then run it now.
    ///
    /// Only the exact text is remembered: a near miss (`npm test` vs
    /// `npm test -- --watch`, or a one-character code change) still asks,
    /// because the rule is meant to remove repetition, not to become a blanket
    /// permission.
    pub(crate) fn allow_session_call(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(message) = self
            .chats
            .get(self.active_chat)
            .and_then(|chat| chat.messages.get(index))
        else {
            return;
        };
        if !matches!(message.role, Role::Approval) {
            return;
        }
        let tool = message.tool.clone();
        // A file card remembers the path; a command card remembers its exact text.
        let file = allow_file_session_button(&tool, &message.path);
        let command = if file {
            String::new()
        } else if allow_session_button(&tool, &message.input) {
            command_from_approval(&message.input)
        } else {
            return;
        };
        if !file && command.is_empty() {
            return;
        }
        let key = if file {
            file_session_key(&message.path)
        } else {
            session_key(&tool, &command)
        };
        let path = message.path.clone();
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.allow_session.insert(key);
        }
        let note = if file {
            format!("本次会话内将自动允许改这个文件：{path}")
        } else if tool == "shell" {
            format!("本次会话内将自动允许：{command}")
        } else {
            format!("本次会话内将自动允许这段 {tool} 代码")
        };
        self.show_toast(note, false, cx);
        self.answer_approval(index, true, cx);
    }

    /// Roll back every undoable write made while answering `answer`.
    ///
    /// Newest first, because two writes to the same file must undo in reverse:
    /// the first restore only matches while the file still holds the second
    /// write's result. A file changed by someone else is left alone and named
    /// in the note, exactly like a single-write undo.
    pub(crate) fn undo_turn_writes(&mut self, answer: usize, cx: &mut Context<Self>) {
        let work_dir = self.chat_dir(self.active_chat);
        let scope = if self.chat().full_access {
            None
        } else {
            Some(self.ai.workspace_dir())
        };
        let Some(chat) = self.chats.get_mut(self.active_chat) else {
            return;
        };
        let writes = turn_writes(&chat.messages, answer);
        if writes.is_empty() {
            return;
        }
        let mut restored = 0usize;
        let mut failed: Vec<String> = Vec::new();
        for index in writes.into_iter().rev() {
            let Some(message) = chat.messages.get(index) else {
                continue;
            };
            let path = message.path.clone();
            let before = message.before.clone();
            let after = message.after.clone();
            let existed = message.existed;
            let Some(target) = resolve_scoped(&work_dir, scope.as_deref(), &path) else {
                failed.push(format!("{path}：路径不合法"));
                continue;
            };
            match restore_file(&target, &after, &before, existed) {
                Ok(()) => {
                    if let Some(message) = chat.messages.get_mut(index) {
                        message.output = "已撤销".to_string();
                        message.before.clear();
                        message.after.clear();
                        message.undoable = false;
                    }
                    restored += 1;
                }
                Err(error) => failed.push(format!("{path}：{error}")),
            }
        }
        if restored > 0 {
            chat.messages.push(ChatMessage::note(
                format!("已撤销本轮 {restored} 次写入"),
                false,
            ));
        }
        if !failed.is_empty() {
            chat.messages.push(ChatMessage::note(
                format!("有 {} 个文件没有撤销：{}", failed.len(), failed.join("；")),
                true,
            ));
        }
        chat.touch();
        self.save_chats();
        cx.notify();
    }

    /// The next verified connection this question has not tried yet.
    pub(crate) fn next_fallback(&self, slot: usize) -> Option<usize> {
        let tried = &self.pending.get(slot)?.retry.tried;
        self.ai.connections.iter().position(|connection| {
            connection.verified && !tried.contains(&connection_key(connection))
        })
    }

    /// Open one file an answer changed, inside the conversation's own scope.
    ///
    /// The path comes from the transcript, so it goes through the same scope
    /// check an undo does: a saved write must not become a way to open anything.
    pub(crate) fn open_changed_file(&mut self, path: &str, cx: &mut Context<Self>) {
        let work_dir = self.chat_dir(self.active_chat);
        let scope = if self.chat().full_access {
            None
        } else {
            Some(self.ai.workspace_dir())
        };
        let Some(target) = resolve_scoped(&work_dir, scope.as_deref(), path) else {
            self.banner = Some(Banner::error(format!("路径不合法：{path}")));
            cx.notify();
            return;
        };
        if let Err(error) = open_path(&target) {
            self.banner = Some(Banner::error(format!(
                "无法打开 {}：{error}",
                target.display()
            )));
        }
        cx.notify();
    }

    /// Put back the file a just-approved write replaced.
    ///
    /// Undo is refused when the file no longer holds what that write produced:
    /// overwriting a later change would be worse than leaving the write in
    /// place.
    pub(crate) fn undo_write(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((path, before, after, existed, undoable, state)) = self
            .chats
            .get(self.active_chat)
            .and_then(|chat| chat.messages.get(index))
            .map(|message| {
                (
                    message.path.clone(),
                    message.before.clone(),
                    message.after.clone(),
                    message.existed,
                    message.undoable,
                    message.output.clone(),
                )
            })
        else {
            return;
        };
        if state != "已写入" || !undoable || path.is_empty() {
            return;
        }
        let work_dir = self.chat_dir(self.active_chat);
        let scope = if self.chat().full_access {
            None
        } else {
            Some(self.ai.workspace_dir())
        };
        let Some(target) = resolve_scoped(&work_dir, scope.as_deref(), &path) else {
            self.finish_undo(index, "无法撤销：路径不合法".to_string(), false, cx);
            return;
        };
        match restore_file(&target, &after, &before, existed) {
            Ok(()) => self.finish_undo(index, "已撤销".to_string(), true, cx),
            Err(error) => self.finish_undo(index, error, false, cx),
        }
    }

    /// Record how an undo ended, and leave a note when the file changed.
    pub(crate) fn finish_undo(
        &mut self,
        index: usize,
        state: String,
        restored: bool,
        cx: &mut Context<Self>,
    ) {
        let mut path = String::new();
        if let Some(message) = self
            .chats
            .get_mut(self.active_chat)
            .and_then(|chat| chat.messages.get_mut(index))
        {
            path = message.path.clone();
            message.output = state;
            if restored {
                // Undo data is no longer needed and can be large.
                message.before.clear();
                message.after.clear();
                message.undoable = false;
            }
        }
        if restored && !path.is_empty() {
            // The model is not told; the note is for the person reading.
            if let Some(chat) = self.chats.get_mut(self.active_chat) {
                chat.messages
                    .push(ChatMessage::note(format!("已撤销 {path} 的写入"), false));
            }
        }
        self.save_chats();
        cx.notify();
    }
    /// Answer one approval event: decide it, or put the card before the person.
    ///
    /// The engine waits on this branch, so every path ends with a decision: an
    /// automatic allow, or a card that the person answers later.
    /// @param slot - the pending answer this event belongs to.
    /// @param chat_index - the conversation the card is written into.
    /// @param event - the `Approval` event, taken by value so its fields can move.
    pub(super) fn handle_approval(&mut self, slot: usize, chat_index: usize, event: AiEvent) {
        let AiEvent::Approval {
            call,
            tool,
            target,
            path,
            diff,
            before,
            after,
            existed,
            undoable,
            auto,
            command,
            secret,
        } = event
        else {
            return;
        };
        // Round prose is not the answer; the decision card is.
        self.pending[slot].draft.clear();
        self.pending[slot].approval = None;
        self.pending[slot].acted = true;
        // A credential leaving the machine is the one call that is
        // always put in front of the person: 完全权限 covers running
        // things, not handing a key to a URL.
        let trusted = !auto
            && !secret
            && self
                .chats
                .get(chat_index)
                .is_some_and(|chat| chat.auto_actions);
        // A command line and a code snippet are both identified by
        // the text the card shows, so one rule covers both.
        let remembered = if command.is_empty() {
            command_from_approval(&diff)
        } else {
            command.clone()
        };
        let allowed_before = !auto
            && !secret
            && self.chats.get(chat_index).is_some_and(|chat| {
                // Commands and snippets are remembered by their exact
                // text; a file is remembered by its path.
                let command_ok = !remembered.is_empty()
                    && matches!(tool.as_str(), "shell" | "python" | "node")
                    && chat
                        .allow_session
                        .contains(&session_key(&tool, &remembered));
                let file_ok = matches!(tool.as_str(), "write" | "edit")
                    && !path.trim().is_empty()
                    && chat.allow_session.contains(&file_session_key(&path));
                command_ok || file_ok
            });
        // Auto review: a conservative rule grants low-risk writes
        // and read-only commands, and everything else still asks.
        let review_extra = self.ai.review_allow.as_slice();
        let reviewed = !auto
            && !secret
            && !trusted
            && !allowed_before
            && self.chats.get(chat_index).is_some_and(|chat| {
                chat.auto_review
                    && !chat.read_only
                    && auto_review_allows(&tool, &command, &path, &diff, review_extra)
            });
        // A credential card is never answered by a rule; the helper keeps that
        // decision in one testable place.
        let auto_answered = approval_auto_answered(secret, auto, trusted, allowed_before, reviewed);
        if !auto_answered {
            self.pending[slot].approval = Some(call.clone());
        }
        if auto_answered {
            // Decided already: the call is recorded by its tool row,
            // so no approval card is drawn at all. A write still has
            // to hand its undo pair to that row.
            if matches!(tool.as_str(), "write" | "edit") && undoable && !path.is_empty() {
                self.pending[slot].writes.push(ApprovedWrite {
                    call: call.clone(),
                    path,
                    before,
                    after,
                    existed,
                    undoable,
                });
            }
            self.pending[slot].request.respond(&call, true);
            self.save_chats();
            return;
        }
        if let Some(chat) = self.chats.get_mut(chat_index) {
            let text = if target.is_empty() {
                tool.clone()
            } else {
                format!("{tool} · {target}")
            };
            let mut message = ChatMessage::approval(
                call.clone(),
                path.clone(),
                text,
                diff,
                before,
                after,
                existed,
                undoable,
            );
            message.tool = tool;
            // Kept in memory only: the card is decided while the
            // question is running, and the hint would be noise once
            // it has been answered.
            message.secret = secret;
            chat.messages.push(message);
            chat.touch();
        }
        self.save_chats();
    }
}

/// Whether a failed turn may be replayed on another connection.
///
/// A turn that already streamed text or touched a tool is never replayed: the
/// second attempt would duplicate what the reader already has, or run the same
/// writes again.
pub(crate) fn should_fallback(draft: &str, acted: bool, waiting: bool) -> bool {
    draft.trim().is_empty() && !acted && !waiting
}

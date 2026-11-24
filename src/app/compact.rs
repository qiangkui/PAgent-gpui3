//! The last compaction layer: a model-written checkpoint of the turns the
//! history window already stopped replaying.
//!
//! The deterministic layers run inside one question: a tool result is capped,
//! the tool budget drops the oldest results, and the history window drops the
//! oldest turns with a first-line recap left in the prompt. That recap keeps the
//! *shape* of what was dropped but loses the detail, so this module asks the
//! model to condense the dropped span into a fixed-structure checkpoint and
//! keeps it in the conversation folder as `compact.json`.
//!
//! It runs beside the question, never inside it: the window sends the turns it
//! wants condensed to the engine's `compact` command on a worker thread, and the
//! next question picks the checkpoint up from `prompt_blocks`. A failure writes
//! nothing, so the recap stays in charge and a question is never delayed or
//! broken by compaction.

use super::*;

/// What asking for a compaction did, so the composer can say why nothing ran.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactOutcome {
    /// The engine call is on its way; the checkpoint lands when it answers.
    Started,
    /// No turn has passed the replay window, so there is nothing to condense.
    NothingNew,
    /// No verified connection, so the summary call cannot be made.
    NoConnection,
    /// This conversation is already being compacted.
    Busy,
}

impl DshFlowApp {
    /// Condense the dropped turns of a conversation, if it is worth a call.
    ///
    /// Nothing happens when no turn was dropped yet, when the checkpoint already
    /// covers the same content, when no connection is verified, or when a run
    /// for this conversation is still in flight. A question therefore costs one
    /// extra call only when the conversation has actually grown past the window.
    pub(crate) fn compact_history(
        &mut self,
        chat_index: usize,
        cx: &mut Context<Self>,
    ) -> CompactOutcome {
        let Some(chat) = self.chats.get(chat_index) else {
            return CompactOutcome::NothingNew;
        };
        let dir = self.chat_dir(chat_index);
        let existing = util::read_checkpoint(&dir);
        let Some((span, fingerprint)) = util::compaction_span(&chat.messages, existing.as_ref())
        else {
            return CompactOutcome::NothingNew;
        };
        let Some(connection) = self.ai.active().filter(|entry| entry.verified).cloned() else {
            return CompactOutcome::NoConnection;
        };
        let folder = chat.dir.clone();
        if !self.compacting.insert(folder.clone()) {
            return CompactOutcome::Busy;
        }
        let turns = span.len();
        let engine_dir = self.engine_dir.clone();
        let system = COMPACT_INSTRUCTION.to_string();
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(crate::ai::compact_span(
                &engine_dir,
                &connection,
                &span,
                &system,
            ));
        });
        // The answer the reader is waiting for is not held up by this: the
        // checkpoint only has to be on disk before the question after it.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                match result.try_recv() {
                    Ok(answer) => {
                        this.update(cx, |view, cx| {
                            view.finish_compaction(&dir, &folder, fingerprint, turns, answer);
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    // The worker died without answering. Clearing the flag is what
                    // lets this conversation be compacted again, instead of
                    // staying busy for the rest of the session.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        this.update(cx, |view, cx| {
                            view.compacting.remove(&folder);
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
                let alive = this
                    .update(cx, |view, _| view.compacting.contains(&folder))
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
        CompactOutcome::Started
    }

    /// Run a compaction now, from the composer's attach menu.
    ///
    /// Unlike the automatic pass after a question, this one says why nothing
    /// ran: an answer in flight, no verified model, or a conversation that has
    /// not passed the replay window yet.
    pub(crate) fn compress_context(&mut self, cx: &mut Context<Self>) {
        self.attach_menu = false;
        let index = self.active_chat;
        if self.chat_busy(index) {
            self.show_toast("回答还在进行，完成后再压缩", true, cx);
            return;
        }
        match self.compact_history(index, cx) {
            CompactOutcome::Started => {
                self.show_toast("正在压缩较早的上下文，完成后会写入摘要", false, cx);
            }
            CompactOutcome::NothingNew => {
                self.show_toast("上下文还没超出回放窗口，暂时没有可压缩的内容", false, cx);
            }
            CompactOutcome::NoConnection => {
                self.show_toast("先在「设置 → AI」接好一个可用模型", true, cx);
            }
            CompactOutcome::Busy => {
                self.show_toast("这个对话已经在压缩中", false, cx);
            }
        }
    }

    /// Store the checkpoint a finished compaction produced.
    ///
    /// A failure is deliberately silent: the recap already stands in for the
    /// dropped turns, and a banner would blame the reader for something the
    /// window decided to do on its own. The one thing worth saying is when a
    /// checkpoint *was* made, because that changes what the model remembers.
    fn finish_compaction(
        &mut self,
        dir: &std::path::Path,
        folder: &str,
        fingerprint: u64,
        turns: usize,
        answer: Result<crate::ai::Compacted, String>,
    ) {
        self.compacting.remove(folder);
        let Ok(answer) = answer else {
            return;
        };
        let checkpoint = util::Checkpoint {
            summary: answer.summary,
            turns,
            model: answer.model,
            at_ms: util::now_ms(),
            span: fingerprint,
        };
        if util::write_checkpoint(dir, &checkpoint).is_err() {
            return;
        }
        // A note in the transcript, so what the model was given can be seen and
        // not just guessed at. It is a `Note`, so it is never replayed back.
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.dir == folder) {
            chat.messages.push(ChatMessage::note(
                format!("上下文压缩：最早的 {turns} 条消息已由模型压成摘要，后续提问会带上它。"),
                false,
            ));
            chat.touch();
        }
        self.save_chats();
    }
}

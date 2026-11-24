//! Sending a question, pumping the answer back, and stopping it.

use super::*;

impl DshFlowApp {
    /// Drop the caret from any text field and give the content area the keyboard.
    ///
    /// Leaving a field also files whatever an inline editor was holding: a typed
    /// category name must not be lost just because the click went elsewhere.
    pub(crate) fn leave_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_category_rename();
        self.commit_new_category();
        self.focused = None;
        window.focus(&self.content_focus, cx);
    }

    /// Rebuild the input fields from the selected manifest.
    pub(crate) fn rebuild_inputs(&mut self, cx: &mut Context<Self>) {
        let specs: Vec<InputSpec> = self
            .selected_entry()
            .map(|entry| entry.spec.inputs.clone())
            .unwrap_or_default();
        self.inputs = specs
            .into_iter()
            .map(|spec| {
                let mut field = TextField::new(cx);
                field.set_text(spec.initial_value());
                InputField { spec, field }
            })
            .collect();
    }

    /// Add files dropped onto the conversation to the composer.
    ///
    /// One unreadable file does not sink the rest: its reason is shown in the
    /// banner, and the others still go along with the question.
    pub(crate) fn attach_files(&mut self, paths: &[std::path::PathBuf], cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let mut problems: Vec<String> = Vec::new();
        for path in paths {
            if self.attachments.len() >= MAX_ATTACHMENTS {
                problems.push(format!("一次最多附加 {MAX_ATTACHMENTS} 个文件"));
                break;
            }
            match read_attachment(path) {
                Ok(attachment) => {
                    // Dropping the same file twice must not send it twice.
                    if self
                        .attachments
                        .iter()
                        .any(|item| item.name == attachment.name)
                    {
                        continue;
                    }
                    // A second file on top of the first still has to fit in one
                    // request; the endpoint would refuse the whole question.
                    let wanted = attached_bytes(std::slice::from_ref(&attachment));
                    if attached_bytes(&self.attachments) + wanted > MAX_ATTACHMENT_TOTAL_BYTES {
                        problems.push(format!(
                            "附件总量上限 {}，这个文件放不进去",
                            short_bytes(MAX_ATTACHMENT_TOTAL_BYTES as u64)
                        ));
                        continue;
                    }
                    self.attachments.push(attachment);
                }
                Err(error) => problems.push(error),
            }
        }
        if problems.is_empty() {
            cx.notify();
        } else {
            // A dropped file that cannot be read is a notice, not a state: it
            // fades out above the composer instead of holding a strip open.
            self.show_toast(problems.join("；"), true, cx);
        }
    }

    /// Attach one image taken from the clipboard.
    pub(crate) fn attach_image(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if self.attachments.len() >= MAX_ATTACHMENTS {
            self.show_toast(format!("一次最多附加 {MAX_ATTACHMENTS} 个文件"), true, cx);
            return;
        }
        let attachment = clipboard_attachment(bytes);
        let wanted = attached_bytes(std::slice::from_ref(&attachment));
        if attached_bytes(&self.attachments) + wanted > MAX_ATTACHMENT_TOTAL_BYTES {
            self.show_toast("附件总量已达上限，这张图放不进去", true, cx);
            return;
        }
        self.attachments.push(attachment);
        cx.notify();
    }

    /// Show a hint that fades out after about a second.
    ///
    /// Used for things the reader only needs to notice — a dropped file that is
    /// not text — where a banner would take up room for the rest of the session.
    pub(crate) fn show_toast(
        &mut self,
        text: impl Into<String>,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        let started = Instant::now();
        self.toast = Some(Toast {
            text: text.into(),
            started,
            failed,
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            for _ in 0..30 {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                let alive = this
                    .update(cx, |view, cx| {
                        // A newer hint owns the screen now; the old timer stops.
                        if view
                            .toast
                            .as_ref()
                            .is_none_or(|toast| toast.started != started)
                        {
                            return false;
                        }
                        if started.elapsed() >= TOAST_TOTAL {
                            view.toast = None;
                        }
                        cx.notify();
                        view.toast.is_some()
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }

    /// The floating hint's current opacity: solid, then a linear fade.
    fn toast_opacity(&self) -> f32 {
        let Some(toast) = &self.toast else {
            return 0.;
        };
        let elapsed = toast.started.elapsed();
        if elapsed <= TOAST_HOLD {
            return 1.;
        }
        let fade = TOAST_TOTAL.saturating_sub(TOAST_HOLD);
        let left = TOAST_TOTAL.saturating_sub(elapsed);
        (left.as_secs_f32() / fade.as_secs_f32()).clamp(0., 1.)
    }

    /// The hint itself, floating just above the composer.
    pub(crate) fn render_toast(&self, palette: Palette) -> Option<AnyElement> {
        let toast = self.toast.as_ref()?;
        let color = if toast.failed {
            palette.failed
        } else {
            palette.text
        };
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(COMPOSER_CLEARANCE))
                .flex()
                .flex_row()
                .justify_center()
                .px(px(24.))
                .child(
                    div()
                        .max_w(px(520.))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(palette.panel))
                        .border_1()
                        .border_color(rgb(if toast.failed {
                            palette.failed
                        } else {
                            palette.border
                        }))
                        .shadow_lg()
                        .text_xs()
                        .text_color(rgb(color))
                        .whitespace_normal()
                        .opacity(self.toast_opacity())
                        .child(toast.text.clone()),
                )
                .into_any_element(),
        )
    }

    /// Drop one attached file before sending.
    pub(crate) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            cx.notify();
        }
    }

    /// Step through the questions already asked in this conversation.
    ///
    /// `older` walks back from the newest question; going past it puts the empty
    /// composer back. False means "not recalling": the caller then lets the field
    /// move its caret as usual, so a non-empty draft keeps normal arrow keys.
    pub(crate) fn recall_question(&mut self, older: bool, cx: &mut Context<Self>) -> bool {
        let questions: Vec<String> = self
            .chat()
            .messages
            .iter()
            .filter(|message| matches!(message.role, Role::User))
            .map(|message| message.text.clone())
            .filter(|text| !text.trim().is_empty())
            .collect();
        if questions.is_empty() {
            return false;
        }
        let Some(current) = self.composer_recall else {
            if !older || !self.composer.text.trim().is_empty() {
                return false;
            }
            let newest = questions.last().cloned().unwrap_or_default();
            self.composer.set_text(newest);
            self.composer_recall = Some(0);
            cx.notify();
            return true;
        };
        if older {
            let next = current + 1;
            if next < questions.len() {
                let text = questions[questions.len() - 1 - next].clone();
                self.composer.set_text(text);
                self.composer_recall = Some(next);
                cx.notify();
            }
            return true;
        }
        if current == 0 {
            self.composer.set_text(String::new());
            self.composer_recall = None;
        } else {
            let next = current - 1;
            let text = questions[questions.len() - 1 - next].clone();
            self.composer.set_text(text);
            self.composer_recall = Some(next);
        }
        cx.notify();
        true
    }

    /// Send the composed message to the assistant, then save the transcript.
    pub(crate) fn send_message(&mut self, cx: &mut Context<Self>) {
        self.send_question(cx);
        self.save_chats();
    }

    /// Build the request for the composed message.
    pub(crate) fn send_question(&mut self, cx: &mut Context<Self>) {
        self.close_composer_menus();
        let chat_index = self.active_chat;
        let Some(dir) = self.chats.get(chat_index).map(|chat| chat.dir.clone()) else {
            return;
        };
        // Only the conversation being answered is held; another one may ask at
        // the same time and gets its own process.
        if self.pending.iter().any(|pending| pending.dir == dir) {
            return;
        }
        let mut text = self.composer.text.trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        // Files alone are a fair question; the transcript says what was asked.
        if text.is_empty() {
            text = "请看附件。".to_string();
        }
        self.composer.clear();
        self.composer_recall = None;

        // Whatever was parked for this conversation has just been sent: the card
        // must stop calling it a draft.
        self.chat_drafts.remove(&dir);
        self.attachment_drafts.remove(&dir);
        // Attached files go in before the question: the history taken below
        // carries their text, so the model reads them with this turn.
        let filed = std::mem::take(&mut self.attachments);
        for attachment in &filed {
            let name = if attachment.truncated {
                format!("{}（已截断）", attachment.name)
            } else {
                attachment.name.clone()
            };
            if let Some(chat) = self.chats.get_mut(chat_index) {
                chat.messages.push(ChatMessage::attachment_of(
                    attachment.kind,
                    name,
                    attachment.text.clone(),
                ));
                chat.touch();
            }
        }
        // The history is taken before this question goes in: the prompt carries
        // it, and sending it twice would repeat the question.
        self.ask_text(text, filed, cx);
    }

    /// Ask the assistant `text` in the open conversation.
    ///
    /// Split out of [`Self::send_question`] so "continue" can send its
    /// instruction without touching what the composer holds. `filed` holds the
    /// binary attachments, whose bytes travel with this request only.
    fn ask_text(&mut self, text: String, filed: Vec<Attachment>, cx: &mut Context<Self>) {
        // Sending starts the transcript again, even if the reader had left the
        // task or context tab open on an empty conversation.
        self.chat_tab = ChatTab::Chat;
        let chat_index = self.active_chat;
        let Some((dir, tools, read_only, full_access, max_rounds, max_calls)) =
            self.chats.get(chat_index).map(|chat| {
                (
                    chat.dir.clone(),
                    chat.tools,
                    chat.read_only,
                    chat.full_access,
                    chat.max_rounds,
                    chat.max_calls,
                )
            })
        else {
            return;
        };
        let (history, dropped) = self.chat_history(chat_index);
        // A new question makes the newest row the interesting one again.
        self.context_request = None;
        self.context_round = None;
        self.context_item = None;
        // Resolved before the question goes in: the history and the system
        // prompt both describe the conversation up to this point.
        let connection = self.ai.active().filter(|entry| entry.verified).cloned();
        let model = connection
            .as_ref()
            .map_or_else(|| self.chat_model(), |entry| entry.model.clone());
        let blocks = self.prompt_blocks(chat_index, &model);
        // Only this conversation's schemas: another answer running beside it must
        // not change what this request is recorded as carrying.
        let schemas = self
            .tool_schemas
            .get(&dir)
            .cloned()
            .unwrap_or_default();
        // Recorded with the answer, so the request can be looked at again later
        // even though only one total token figure comes back from the endpoint.
        // The question itself is not in `history` yet, so it is passed in here:
        // without it the person's own words would be missing from the record.
        let items = request_items(
            &blocks,
            &schemas,
            &self.plugin_prompts,
            &history,
            &text,
        );
        if let Some(chat) = self.chats.get_mut(chat_index) {
            if dropped > 0 {
                // Say it where the answer will be read: a cut-down context is
                // otherwise invisible, and the assistant looks forgetful.
                chat.messages.push(ChatMessage::note(
                    format!("上下文超出上限，这一轮省略了最早的 {dropped} 条消息。"),
                    false,
                ));
            }
            if chat.messages.is_empty() {
                chat.title = title_from(&text);
            }
            chat.messages.push(ChatMessage::user(text.clone()));
            chat.touch();
        }

        let Some(connection) = self
            .ai
            .active()
            .filter(|connection| connection.verified)
            .cloned()
        else {
            if let Some(chat) = self.chats.get_mut(chat_index) {
                chat.messages.push(ChatMessage::note(
                    "还没有可用的模型：打开左侧栏「设置」，在 AI 里填好 Key 并点「测试」，通过后即可对话。"
                        .to_string(),
                    true,
                ));
            }
            self.chat_scroll.scroll_to_bottom();
            cx.notify();
            return;
        };

        // Each conversation works inside its own folder under the workspace, so
        // files the assistant or a tool writes stay with that conversation.
        let workspace = self.chat_dir(chat_index);
        if let Err(error) = std::fs::create_dir_all(&workspace) {
            if let Some(chat) = self.chats.get_mut(chat_index) {
                chat.messages.push(ChatMessage::note(
                    format!("无法创建工作目录 {}：{error}", workspace.display()),
                    true,
                ));
            }
            self.chat_scroll.scroll_to_bottom();
            cx.notify();
            return;
        }

        let allowed = if tools { Some(&ALL_TOOLS[..]) } else { None };
        let system = joined_prompt(&blocks);
        // Images and PDFs travel as bytes with this request; their text is
        // already in the history above, so a retry can send the same files.
        let files: Vec<crate::ai::AskAttachment> = filed
            .iter()
            .filter(|attachment| attachment.kind != AttachmentKind::Text)
            .map(|attachment| {
                crate::ai::AskAttachment::new(
                    &attachment.name,
                    &attachment.mime,
                    attachment.kind.key(),
                    &attachment.bytes,
                )
            })
            .collect();
        // The workspace is the default scope; full access drops the jail.
        let scope_root = if full_access {
            None
        } else {
            Some(self.ai.workspace_dir())
        };
        let read_only_roots = vec![self.scan_root.clone()];
        // The command sandbox allows the workspace only; there is no extra
        // allowlist any more.
        let plugin_dir = crate::config::AiSettings::plugin_dir();
        let plugins = self.ai.plugins.clone();
        let mcp_servers = crate::mcp::enabled(&self.ai);

        match AiRequest::ask(
            &self.engine_dir,
            &workspace,
            &text,
            Some(&system),
            &history,
            &connection,
            tools,
            allowed,
            read_only,
            scope_root.as_deref(),
            &read_only_roots,
            max_rounds,
            max_calls,
            &files,
            plugin_dir.as_deref(),
            &plugins,
            &mcp_servers,
            Some(&self.mcp_dir),
        ) {
            Ok(request) => {
                let request = Arc::new(request);
                self.pending.push(PendingAnswer {
                    dir,
                    request: request.clone(),
                    draft: String::new(),
                    follow: true,
                    bottom: self.chat_scroll.offset().y,
                    viewing: false,
                    approval: None,
                    writes: Vec::new(),
                    model: connection.model.clone(),
                    acted: false,
                    round: 0,
                    calls: 0,
                    items,
                    schemas: Vec::new(),
                    retry: RetryAsk {
                        prompt: text.clone(),
                        work_dir: workspace.clone(),
                        system: system.clone(),
                        history: history.clone(),
                        tools,
                        read_only,
                        scope_root: scope_root.clone(),
                        read_only_roots,
                        max_rounds,
                        max_calls,
                        attachments: files.clone(),
                        tried: vec![connection_key(&connection)],
                    },
                });
                self.spawn_chat_pump(request, cx);
            }
            Err(error) => {
                if let Some(chat) = self.chats.get_mut(chat_index) {
                    chat.messages.push(ChatMessage::note(error, true));
                }
            }
        }
        self.chat_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Ask the model to pick up an answer that stopped mid-stream.
    ///
    /// The partial answer is already in the transcript, so the request only needs
    /// an instruction after it; the composer keeps whatever it holds.
    pub(crate) fn continue_answer(&mut self, cx: &mut Context<Self>) {
        let chat_index = self.active_chat;
        let Some(dir) = self.chats.get(chat_index).map(|chat| chat.dir.clone()) else {
            return;
        };
        if self.pending.iter().any(|pending| pending.dir == dir) {
            return;
        }
        let continuable = self
            .chats
            .get(chat_index)
            .is_some_and(|chat| continuable(&chat.messages));
        if !continuable {
            return;
        }
        self.ask_text(CONTINUE_PROMPT.to_string(), Vec::new(), cx);
        self.save_chats();
    }

    /// Switch a version from the composer's picker.
    pub(crate) fn choose_composer_model(
        &mut self,
        index: usize,
        model: &str,
        cx: &mut Context<Self>,
    ) {
        self.close_composer_menus();
        self.model_tab = index;
        self.ai.active = index;
        if let Some(fields) = self.conn_fields.get_mut(index) {
            fields.model.set_text(model.to_string());
        }
        self.set_connection_model(index, model);
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        cx.notify();
    }

    /// Poll the assistant while it answers.
    pub(crate) fn spawn_chat_pump(&self, request: Arc<AiRequest>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.pump_chat(&request, cx);
                        cx.notify();
                        view.pending
                            .iter()
                            .any(|pending| Arc::ptr_eq(&pending.request, &request))
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Move whatever the assistant produced into the conversation.
    pub(crate) fn pump_chat(&mut self, request: &Arc<AiRequest>, cx: &mut Context<Self>) {
        let events = request.poll();
        if events.is_empty() {
            return;
        }
        // The request identifies its slot: several conversations may be waiting.
        let Some(slot) = self
            .pending
            .iter()
            .position(|pending| Arc::ptr_eq(&pending.request, request))
        else {
            return;
        };
        let dir = self.pending[slot].dir.clone();
        let Some(chat_index) = self.chats.iter().position(|chat| chat.dir == dir) else {
            // The conversation is gone: stop the engine rather than let it finish
            // an answer nobody can see (and keep writing in the workspace).
            let pending = self.pending.remove(slot);
            pending.request.cancel();
            return;
        };
        // The transcript on screen is only this answer's when its conversation is
        // open: in another conversation the shared scroll handle belongs to that
        // one, and following would drag the reader to the bottom of it.
        let viewing = chat_index == self.active_chat;
        // Only scrolling moves the offset; longer text does not. Comparing with
        // where the view was left therefore tells "the user scrolled up" apart
        // from "the answer grew", which measuring the bottom never could.
        let offset = self.chat_scroll.offset().y;
        let was_viewing = std::mem::replace(&mut self.pending[slot].viewing, viewing);
        if viewing && !was_viewing {
            // Coming back to this answer: the gap belongs to the screen that
            // was open before, not to the reader scrolling this transcript.
            self.pending[slot].bottom = offset;
        } else if viewing
            && self.pending[slot].follow
            && offset - self.pending[slot].bottom > px(40.)
        {
            self.pending[slot].follow = false;
        }
        let follow = viewing && self.pending[slot].follow;
        for event in events {
            match event {
                // The loop's own counters, kept on the pending answer so the
                // status strip reads them while the answer is still arriving.
                AiEvent::Progress { round, calls } => {
                    self.pending[slot].round = round;
                    self.pending[slot].calls = calls;
                }
                AiEvent::ToolStart { call, tool, target } => {
                    if call.is_empty() {
                        continue;
                    }
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    let write = take_write(&mut self.pending[slot].writes, &call);
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        let exists = chat
                            .messages
                            .iter()
                            .any(|entry| matches!(entry.role, Role::Tool) && entry.call == call);
                        if !exists {
                            let mut message =
                                ChatMessage::tool(live_row_text(&tool, &target, None), false);
                            message.call = call;
                            // The command is the argument while it runs; the
                            // final step replaces this with the full JSON input.
                            message.input = target;
                            apply_write(&mut message, write);
                            chat.messages.push(message);
                        } else if let Some(message) =
                            chat.messages.iter_mut().rev().find(|entry| {
                                matches!(entry.role, Role::Tool) && entry.call == call
                            })
                        {
                            apply_write(message, write);
                        }
                        chat.touch();
                    }
                }
                AiEvent::ToolOutput {
                    call,
                    tool,
                    target,
                    text: chunk,
                } => {
                    if call.is_empty() {
                        continue;
                    }
                    // The model already asked for the tool; any prose in this
                    // round belongs to the round, not to the answer.
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        let line = last_output_line(&chunk);
                        let row = live_row_text(&tool, &target, line.as_deref());
                        let existing =
                            chat.messages.iter_mut().rev().find(|entry| {
                                matches!(entry.role, Role::Tool) && entry.call == call
                            });
                        if let Some(message) = existing {
                            append_live_output(&mut message.output, &chunk);
                            if line.is_some() {
                                message.text = row;
                            }
                        } else {
                            let mut message = ChatMessage::tool(row, false);
                            message.call = call;
                            append_live_output(&mut message.output, &chunk);
                            chat.messages.push(message);
                        }
                        chat.touch();
                    }
                }
                AiEvent::Delta(text) => {
                    self.pending[slot].approval = None;
                    self.pending[slot].draft.push_str(&text);
                }
                AiEvent::Tools {
                    items,
                    prompts,
                    errors,
                    mcp_errors,
                } => {
                    // The context panel shows what the next request carries, so
                    // the schemas and plugin prompts are kept until the engine
                    // sends a new set; the settings page shows the failures.
                    // This run needs its own copy: the first question of a
                    // conversation was recorded before any schema arrived.
                    self.pending[slot].schemas = items.clone();
                    self.tool_schemas.insert(dir.clone(), items);
                    self.plugin_prompts = prompts;
                    self.plugin_errors = errors;
                    self.mcp_errors = mcp_errors;
                }
                AiEvent::Step {
                    tool,
                    target,
                    ok,
                    detail,
                    input,
                    output,
                    call,
                } => {
                    // The model wrote something and then went for a tool: that
                    // text belongs to the round, not to the answer.
                    self.pending[slot].approval = None;
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    let text = if detail.is_empty() {
                        format!("{tool} · {target}")
                    } else {
                        format!("{tool} · {target} — {detail}")
                    };
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        // A live row may already exist for this call; it becomes
                        // the finished row instead of leaving a duplicate.
                        let write = take_write(&mut self.pending[slot].writes, &call);
                        let live = if call.is_empty() {
                            None
                        } else {
                            chat.messages.iter_mut().rev().find(|entry| {
                                matches!(entry.role, Role::Tool) && entry.call == call
                            })
                        };
                        if let Some(message) = live {
                            message.text = text;
                            message.failed = !ok;
                            message.input = input;
                            message.output = output;
                            apply_write(message, write);
                        } else {
                            let mut message = ChatMessage::tool(text, !ok);
                            message.input = input;
                            message.output = output;
                            message.call = call.clone();
                            apply_write(&mut message, write);
                            chat.messages.push(message);
                        }
                        // The write has now run (or failed): settle its card,
                        // which also decides whether undo can be offered.
                        if let Some(approval) = chat.messages.iter_mut().rev().find(|entry| {
                            matches!(entry.role, Role::Approval) && entry.call == call
                        }) && approval.output == "已允许"
                        {
                            approval.output = approval_state(&approval.tool, ok).to_string();
                        }
                    }
                }
                AiEvent::Approval {
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
                } => {
                    // Round prose is not the answer; the decision card is.
                    self.pending[slot].draft.clear();
                    self.pending[slot].approval = None;
                    self.pending[slot].acted = true;
                    // "本次会话都允许" and the conversation's Trust level are
                    // checked before the card is drawn: the call is answered
                    // here, so the model keeps working and no button nobody
                    // needs ever appears.
                    let trusted = !auto
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
                        && !remembered.is_empty()
                        && matches!(tool.as_str(), "shell" | "python" | "node")
                        && self.chats.get(chat_index).is_some_and(|chat| {
                            chat.allow_session
                                .contains(&session_key(&tool, &remembered))
                        });
                    let go_ahead = trusted || allowed_before;
                    if !auto && !go_ahead {
                        self.pending[slot].approval = Some(call.clone());
                    }
                    if auto || go_ahead {
                        // Decided already: the call is recorded by its tool row,
                        // so no approval card is drawn at all. A write still has
                        // to hand its undo pair to that row.
                        if matches!(tool.as_str(), "write" | "edit") && undoable && !path.is_empty()
                        {
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
                        continue;
                    }
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        let text = if target.is_empty() {
                            tool.clone()
                        } else {
                            format!("{tool} · {target}")
                        };
                        let mut message = ChatMessage::approval(
                            call.clone(),
                            path,
                            text,
                            diff,
                            before,
                            after,
                            existed,
                            undoable,
                        );
                        message.tool = tool;
                        chat.messages.push(message);
                        chat.touch();
                    }
                    self.save_chats();
                }
                AiEvent::Done {
                    text,
                    usage,
                    context,
                    cache_hits,
                    rounds,
                    round_tokens,
                    compaction,
                } => {
                    // Say what the engine's compaction did, above the answer, so
                    // a suddenly shorter context is explained rather than
                    // mysterious. An empty note means nothing happened.
                    let note = compaction.map(|report| report.note()).unwrap_or_default();
                    if !note.is_empty()
                        && let Some(chat) = self.chats.get_mut(chat_index)
                    {
                        chat.messages.push(ChatMessage::note(note, false));
                    }
                    let pending = &self.pending[slot];
                    let model = pending.model.clone();
                    let elapsed = pending.request.elapsed_ms();
                    let mut items = pending.items.clone();
                    let schemas = pending.schemas.clone();
                    // The number the panel counts requests by: this answer's
                    // place among the answers of this conversation.
                    let mut done = None;
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        // What the tools returned during this question is
                        // replayed into its next round, so it belongs to what
                        // this question cost — and `chat.json` only kept the
                        // totals, so it has to come from the transcript.
                        items.extend(posted_tool_items(&chat.messages));
                        // The engine reports its tool schemas only once a run has
                        // started, so a conversation's first question recorded
                        // none of them: add what this run actually sent.
                        add_missing_tool_schemas(&mut items, &schemas);
                        // The text this request finally wrote: no later prompt
                        // replays it, so the request's own record has to keep it
                        // or the panel would never show the answer.
                        if let Some(item) = output_item(&text) {
                            items.push(item);
                        }
                        // Every round sends the whole prompt again, so the
                        // request's figures are the sum of its rounds, not the
                        // pieces counted once.
                        let round_parts = round_estimates(&items, &round_tokens, rounds);
                        let (composition, tools_tokens) = if round_parts.is_empty() {
                            let composition = composition_of(&items);
                            let tools_tokens: usize = items
                                .iter()
                                .filter(|item| item.kind == ContextKind::ToolResults)
                                .map(|item| item.tokens)
                                .sum();
                            (composition, tools_tokens)
                        } else {
                            let totals = summed_rounds(&round_parts);
                            let tool_index = ContextKind::ALL
                                .iter()
                                .position(|kind| *kind == ContextKind::ToolResults)
                                .unwrap_or(0);
                            (
                                composition_counts(&totals),
                                totals.get(tool_index).copied().unwrap_or(0),
                            )
                        };
                        let turn = chat
                            .messages
                            .iter()
                            .filter(|message| message.role == Role::Assistant)
                            .count() as u64
                            + 1;
                        // The per-round list counts the tool calls the question
                        // made; the statistics page needs that total without
                        // opening every stored item.
                        let calls: u64 = round_tokens
                            .iter()
                            .map(|(_, _, calls)| u64::from(*calls))
                            .sum();
                        let mut message = ChatMessage::assistant(text);
                        message.model = model.clone();
                        message.elapsed_ms = elapsed;
                        message.tokens = usage.map_or(0, |usage| usage.total());
                        message.context = context;
                        message.cache_hits = cache_hits;
                        message.rounds = rounds;
                        message.round_tokens = round_tokens;
                        message.composition = composition;
                        message.tools_tokens = tools_tokens.min(u32::MAX as usize) as u32;
                        done = Some((turn, message.tokens, message.context, calls));
                        chat.messages.push(message);
                        settle_approvals(chat, "已失效");
                        chat.touch();
                    }
                    self.pending.remove(slot);
                    if let (Some((turn, tokens, context, calls)), Some(folder)) = (
                        done,
                        self.chats.get(chat_index).map(|chat| chat.dir.clone()),
                    ) {
                        // A line per request: the transcript keeps the totals and
                        // the full text lives here, so the request can be opened
                        // again later without bloating `chat.json`.
                        let request = crate::chats::StoredRequest {
                            turn,
                            at_ms: now_ms(),
                            model,
                            tokens,
                            context,
                            calls,
                            items: items
                                .iter()
                                .map(|item| crate::chats::StoredRequestItem {
                                    kind: item.kind.key().to_string(),
                                    label: item.label.clone(),
                                    text: item.text.clone(),
                                    tokens: item.tokens as u64,
                                })
                                .collect(),
                        };
                        if let Err(error) =
                            crate::chats::append_request(&self.ai.data_dir(), &folder, &request)
                        {
                            self.banner = Some(Banner::error(error));
                        }
                    }
                    self.save_chats();
                    // A connection made during this question writes its skill
                    // files; refresh the page if it is the one on screen.
                    if self.view == View::Mcp {
                        self.refresh_mcp(cx);
                    }
                    // The turns this question pushed past the window are what
                    // the last compaction layer condenses; it runs on a worker
                    // thread, so the reader's next question is not held up.
                    let _ = self.compact_history(chat_index, cx);
                    break;
                }
                AiEvent::Failed(error) => {
                    let (draft, acted, waiting, failed_model) = {
                        let pending = &self.pending[slot];
                        (
                            pending.draft.clone(),
                            pending.acted,
                            pending.approval.is_some(),
                            pending.model.clone(),
                        )
                    };
                    // Only a turn that produced nothing and touched nothing may
                    // be replayed elsewhere; otherwise it would duplicate text,
                    // tools or writes.
                    if should_fallback(&draft, acted, waiting) {
                        let reason = error
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .chars()
                            .take(160)
                            .collect::<String>();
                        // The engine is shared by every connection, so a failed
                        // spawn ends the attempt instead of trying the rest.
                        if let Some(index) = self.next_fallback(slot) {
                            let connection = self.ai.connections[index].clone();
                            let key = connection_key(&connection);
                            let request = {
                                let retry = &self.pending[slot].retry;
                                let plugin_dir = crate::config::AiSettings::plugin_dir();
                                let plugins = self.ai.plugins.clone();
                                let mcp_servers = crate::mcp::enabled(&self.ai);
                                AiRequest::ask(
                                    &self.engine_dir,
                                    &retry.work_dir,
                                    &retry.prompt,
                                    Some(&retry.system),
                                    &retry.history,
                                    &connection,
                                    retry.tools,
                                    if retry.tools {
                                        Some(&ALL_TOOLS[..])
                                    } else {
                                        None
                                    },
                                    retry.read_only,
                                    retry.scope_root.as_deref(),
                                    &retry.read_only_roots,
                                    retry.max_rounds,
                                    retry.max_calls,
                                    &retry.attachments,
                                    plugin_dir.as_deref(),
                                    &plugins,
                                    &mcp_servers,
                                    Some(&self.mcp_dir),
                                )
                            };
                            match request {
                                Ok(request) => {
                                    let request = Arc::new(request);
                                    let old = std::mem::replace(
                                        &mut self.pending[slot].request,
                                        request.clone(),
                                    );
                                    self.pending[slot].model = connection.model.clone();
                                    self.pending[slot].retry.tried.push(key);
                                    drop(old);
                                    if let Some(chat) = self.chats.get_mut(chat_index) {
                                        let why = if reason.is_empty() {
                                            String::new()
                                        } else {
                                            format!("（{reason}）")
                                        };
                                        chat.messages.push(ChatMessage::note(
                                            format!(
                                                "自动切换模型：「{failed_model}」调用失败{why}，改用「{}」重试。",
                                                connection.model
                                            ),
                                            false,
                                        ));
                                        chat.touch();
                                    }
                                    self.save_chats();
                                    self.spawn_chat_pump(request, cx);
                                    return;
                                }
                                Err(start_error) => {
                                    // Every connection shares the engine; a
                                    // spawn failure will not heal by switching.
                                    if let Some(pending) = self.pending.get_mut(slot) {
                                        pending.retry.tried.push(key);
                                    }
                                    if let Some(chat) = self.chats.get_mut(chat_index) {
                                        chat.messages.push(ChatMessage::note(
                                            format!(
                                                "「{}」无法启动：{start_error}",
                                                connection.model
                                            ),
                                            true,
                                        ));
                                        chat.touch();
                                    }
                                }
                            }
                        }
                    }
                    // A failure can arrive after the model has already written
                    // part of the answer; that text is still the user's, so it
                    // stays above the error the same way a stopped answer does.
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        chat.messages.extend(stopped_messages(&draft));
                        chat.messages.push(ChatMessage::note(error, true));
                        settle_approvals(chat, "已取消");
                        chat.touch();
                    }
                    self.pending.remove(slot);
                    // The turns this question pushed past the window are what the
                    // last compaction layer condenses; it runs on a worker thread
                    // so the reader's next question is not held up by it.
                    let _ = self.compact_history(chat_index, cx);
                    self.save_chats();
                    break;
                }
            }
        }
        // The transcript grows as the text arrives, and the bottom keeps moving
        // after a scroll is asked for, so the request is repeated on every batch.
        if follow {
            self.chat_scroll.scroll_to_bottom();
            let target = -self.chat_scroll.max_offset().y;
            if let Some(pending) = self.pending.get_mut(slot) {
                // The slot may now hold a later answer if this one just finished.
                if Arc::ptr_eq(&pending.request, request) {
                    pending.bottom = target;
                }
            }
        }
    }

    /// Stop the answer to the conversation on screen.
    pub(crate) fn cancel_answer(&mut self, cx: &mut Context<Self>) {
        self.cancel_chat(self.active_chat, cx);
    }

    /// Stop every answer and keep what each had already written; used as the
    /// window closes so a half-finished answer is not lost with the process.
    pub(crate) fn stop_all_answers(&mut self, cx: &mut Context<Self>) {
        let dirs: Vec<String> = self
            .pending
            .iter()
            .map(|pending| pending.dir.clone())
            .collect();
        for dir in dirs {
            if let Some(index) = self.chats.iter().position(|chat| chat.dir == dir) {
                self.cancel_chat(index, cx);
            }
        }
    }

    /// Stop waiting for one conversation's answer.
    ///
    /// Whatever the model had already written stays in the transcript: the user
    /// stopped the stream, not the conversation, and throwing away visible text
    /// would lose it for good.
    pub(crate) fn cancel_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(dir) = self.chats.get(index).map(|chat| chat.dir.clone()) else {
            return;
        };
        let Some(slot) = self.pending.iter().position(|pending| pending.dir == dir) else {
            return;
        };
        let pending = self.pending.remove(slot);
        pending.request.cancel();
        let draft = pending.draft.trim().to_string();
        if let Some(chat) = self.chats.get_mut(index) {
            chat.messages.extend(stopped_messages(&draft));
            settle_approvals(chat, "已取消");
            chat.touch();
        }
        self.save_chats();
        cx.notify();
    }

    /// Allow or refuse the write whose card sits at `index`.
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
        if !matches!(message.role, Role::Approval)
            || !allow_session_button(&message.tool, &message.input)
        {
            return;
        }
        let tool = message.tool.clone();
        let command = command_from_approval(&message.input);
        if command.is_empty() {
            return;
        }
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.allow_session.insert(session_key(&tool, &command));
        }
        let note = if tool == "shell" {
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
    fn next_fallback(&self, slot: usize) -> Option<usize> {
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
    fn finish_undo(&mut self, index: usize, state: String, restored: bool, cx: &mut Context<Self>) {
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
}

/// Whether a failed turn may be replayed on another connection.
///
/// A turn that already streamed text or touched a tool is never replayed: the
/// second attempt would duplicate what the reader already has, or run the same
/// writes again.
pub(crate) fn should_fallback(draft: &str, acted: bool, waiting: bool) -> bool {
    draft.trim().is_empty() && !acted && !waiting
}

/// A stable identity for a connection, so one question never retries it twice.
fn connection_key(connection: &Connection) -> String {
    format!(
        "{}|{}|{}",
        connection.provider, connection.base_url, connection.model
    )
}

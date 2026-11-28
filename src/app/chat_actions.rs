//! Sending a question, pumping the answer back, and stopping it.

use super::*;

mod answer;
mod approval;
pub(crate) use approval::should_fallback;

impl DshFlowApp {
    /// Drop the caret from any text field and give the content area the keyboard.
    ///
    /// Leaving a field also files whatever an inline editor was holding: a typed
    /// category name must not be lost just because the click went elsewhere.
    pub(crate) fn leave_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_category_rename();
        self.commit_new_category();
        // A revealed password goes back behind dots the moment the fields lose
        // the keyboard; holding the button is the only way to see it.
        self.revealed_input = None;
        self.focused = None;
        window.focus(&self.content_focus, cx);
    }

    /// Rebuild the input fields from the selected manifest.
    ///
    /// A secret this machine saved earlier comes back filled in, so
    /// "保存到本机" still means something after a restart. Everything else
    /// starts at the manifest's default.
    pub(crate) fn rebuild_inputs(&mut self, cx: &mut Context<Self>) {
        let Some((workflow, spec)) = self
            .selected_entry()
            .map(|entry| (entry.name.clone(), entry.spec.clone()))
        else {
            self.inputs.clear();
            self.revealed_input = None;
            return;
        };
        let active_workspace = self.chat_dir(self.active_chat).display().to_string();
        self.inputs = spec
            .inputs
            .iter()
            .map(|input_spec| {
                let mut field = TextField::new(cx);
                let stored = input_spec
                    .secret
                    .then(|| crate::secrets::read_input(&workflow, &input_spec.name))
                    .flatten();
                field.set_text(match &stored {
                    Some(value) => value.clone(),
                    // A secret never starts from a manifest default, even when a
                    // malformed manifest has one: the engine rejects that file,
                    // and the window must not put a password on screen anyway.
                    None if input_spec.secret => String::new(),
                    None => {
                        let default = input_spec.initial_value();
                        // Running a workflow from this window should reuse the
                        // conversation's browser by default: a login already done
                        // in chat then skips the captcha/ai branch entirely.
                        if matches!(input_spec.name.as_str(), "workspace" | "browserDir")
                            && matches!(default.trim(), "" | ".")
                        {
                            active_workspace.clone()
                        } else {
                            default
                        }
                    }
                });
                InputField {
                    uses: spec.consumers(&input_spec.name),
                    step: spec.owner(&input_spec.name),
                    spec: input_spec.clone(),
                    field,
                    stored,
                }
            })
            .collect();
        self.revealed_input = None;
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
        let workflow_command = text.trim() == super::distill::WORKFLOW_COMMAND;
        if workflow_command && !self.ai.active().is_some_and(|entry| entry.verified) {
            self.show_toast(
                "没有可用模型：先在「设置 → AI」接好模型，再发送这条指令",
                true,
                cx,
            );
            return;
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
        if workflow_command {
            // The transcript keeps `/固化为工作流`; the model gets the paths and
            // the Skill rules. Nothing is distiled by the window itself.
            let prompt = self.workflow_command_prompt();
            self.ask_text_with_display(
                prompt,
                super::distill::WORKFLOW_COMMAND.to_string(),
                filed,
                cx,
            );
            self.save_chats();
            return;
        }
        self.ask_text(text, filed, cx);
    }

    /// Ask the assistant `text` in the open conversation.
    ///
    /// Split out of [`Self::send_question`] so "continue" can send its
    /// instruction without touching what the composer holds. `filed` holds the
    /// binary attachments, whose bytes travel with this request only.
    fn ask_text(&mut self, text: String, filed: Vec<Attachment>, cx: &mut Context<Self>) {
        let display = text.clone();
        self.ask_text_with_display(text, display, filed, cx);
    }

    /// Ask the assistant `text`, showing `display` in the transcript.
    ///
    /// The two differ only for the workflow command: the reader sees the short
    /// command, while the model receives the paths and the Skill rules. The
    /// request and its record use `text`; the transcript and a fresh title use
    /// `display`.
    fn ask_text_with_display(
        &mut self,
        text: String,
        display: String,
        filed: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) {
        // Sending starts the transcript again, even if the reader had left the
        // task or context tab open on an empty conversation.
        self.chat_tab = ChatTab::Chat;
        let chat_index = self.active_chat;
        let Some((dir, tools, read_only, full_access, max_rounds, max_calls, mode, plan)) =
            self.chats.get(chat_index).map(|chat| {
                (
                    chat.dir.clone(),
                    chat.tools,
                    chat.read_only,
                    chat.full_access,
                    chat.max_rounds,
                    chat.max_calls,
                    chat.mode.clone(),
                    chat.plan,
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
        // What this request carries, resolved before the record is written: the
        // engine's last tool list can name servers or plugins that are switched
        // off now, and the record must not claim tools this request does not send.
        let selection = self.ai.mode_selection(&mode);
        let plugins = selection.plugins.clone();
        let mcp_servers = crate::mcp::enabled_for(&self.ai, &selection.mcp_servers);
        let mcp_ids: Vec<String> = mcp_servers.iter().map(|server| server.id.clone()).collect();
        // A text-only endpoint is not offered the two image tools: sending one
        // would turn a valid call into a rejected request.
        let allowed_list = mode_tools_for(
            &selection,
            connection.as_ref().is_none_or(|entry| entry.vision),
        );
        // Only this conversation's schemas: another answer running beside it must
        // not change what this request is recorded as carrying.
        let schemas = request_schemas(
            self.tool_schemas.get(&dir).map_or(&[][..], Vec::as_slice),
            tools,
            &allowed_list,
            &plugins,
            &mcp_ids,
            &self.plugin_tools,
            &self.mcp_tools,
        );
        // Recorded with the answer, so the request can be looked at again later
        // even though only one total token figure comes back from the endpoint.
        // The question itself is not in `history` yet, so it is passed in here:
        // without it the person's own words would be missing from the record.
        // The plugin prompts get the same treatment as the schemas: a plugin
        // switched off since the last answer keeps none of its text here.
        let prompts = request_prompts(&self.plugin_prompts, &plugins);
        let items = request_items(&blocks, &schemas, &prompts, &history, &text);
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
                chat.title = title_from(&display);
            }
            chat.messages.push(ChatMessage::user(display.clone()));
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

        // A conversation may keep searching on or off; `None` leaves it on, the
        // default everywhere the endpoint offers the tools.
        let web_search = self
            .chats
            .get(chat_index)
            .and_then(|chat| chat.web_search)
            .unwrap_or(true);

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

        let allowed = if tools { Some(&allowed_list[..]) } else { None };
        let system = joined_prompt(&blocks);
        // The conversation folder is the writable scope, the same folder the OS
        // sandbox jails commands to: a sibling conversation's files are then
        // unreachable from both sides. Full access drops both limits.
        let scope_root = if full_access {
            None
        } else {
            Some(workspace.clone())
        };
        // Images and PDFs travel as bytes with this request; their text is
        // already in the history above, so a retry can send the same files.
        let mut files: Vec<crate::ai::AskAttachment> = filed
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
        // A picture the previous answer showed comes back with this question:
        // "把它再往右挪一点" has no subject otherwise. The person's own
        // attachment wins when there is one.
        let mut carried = 0usize;
        // A text-only endpoint would reject the picture, so the follow-up does not
        // carry last answer's screenshot there.
        if files.is_empty() && !read_only && connection.vision {
            files = self.recent_image_attachments(chat_index, &workspace, scope_root.as_deref());
            carried = files.len();
        }
        if carried > 0
            && let Some(chat) = self.chats.get_mut(chat_index)
        {
            chat.messages.push(ChatMessage::note(
                format!("已把上一轮的 {carried} 张截图一并带上，这一问仍能看到它们。"),
                false,
            ));
            chat.touch();
        }
        let read_only_roots = vec![self.scan_root.clone()];
        // The command sandbox allows the workspace only; there is no extra
        // allowlist any more.
        let plugin_dir = crate::config::AiSettings::plugin_dir();
        // Searching the workspace must not walk into another conversation's
        // folder: those are siblings under the workspace root, and they hold the
        // files that conversation's assistant wrote. The current one stays, of
        // course — it is where this conversation's own files are.
        let own_dir = self
            .chats
            .get(chat_index)
            .map(|chat| chat.dir.clone())
            .unwrap_or_default();
        let skip_dirs: Vec<String> = self
            .chats
            .iter()
            .map(|chat| chat.dir.clone())
            .filter(|dir| *dir != own_dir)
            .collect();

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
            plan,
            tools,
            web_search,
            &files,
            plugin_dir.as_deref(),
            &plugins,
            &mcp_servers,
            Some(&self.mcp_dir),
            &skip_dirs,
        ) {
            Ok(request) => {
                let request = Arc::new(request);
                // Panels are rebuilt with every request. Dropping the previous
                // set here keeps a request that reports none — tools switched
                // off, or a crash before the first round — from showing the last
                // request's cards as its own.
                self.plugin_panels.remove(&dir);
                self.pending.push(PendingAnswer {
                    dir,
                    request: request.clone(),
                    draft: String::new(),
                    reasoning: String::new(),
                    thinking_row: None,
                    thinking_rows: 0,
                    follow: true,
                    bottom: self.chat_scroll.offset().y,
                    viewing: false,
                    approval: None,
                    question: None,
                    plan: None,
                    last_event: Instant::now(),
                    writes: Vec::new(),
                    model: connection.model.clone(),
                    acted: false,
                    round: 0,
                    calls: 0,
                    items,
                    schemas: Vec::new(),
                    tool_names: Vec::new(),
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
                        mode: mode.clone(),
                        plan,
                        web_search,
                        attachments: files.clone(),
                        skip_dirs: skip_dirs.clone(),
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
                        // A request that has gone quiet is stopped here rather than
                        // left spinning: the pump loop is the only timer that runs
                        // while an answer is in flight.
                        view.abandon_stalled(&request);
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

    /// Stop an answer that has produced nothing for a long time.
    ///
    /// A hung engine process, a provider connection that never closes, or a tool
    /// that never returns would otherwise leave 「正在思考」 running until the person
    /// presses stop. A card waiting for a decision pauses the clock: that is the
    /// engine waiting for a person, not a hang.
    /// @param request - the answer to check.
    /// @returns true when it was stopped.
    pub(crate) fn abandon_stalled(&mut self, request: &Arc<AiRequest>) -> bool {
        let Some(slot) = self
            .pending
            .iter()
            .position(|pending| Arc::ptr_eq(&pending.request, request))
        else {
            return false;
        };
        let idle = self.pending[slot].last_event.elapsed();
        let waiting = self.pending[slot].approval.is_some()
            || self.pending[slot].question.is_some()
            || self.pending[slot].plan.is_some();
        if !answer_stalled(idle, waiting) {
            return false;
        }
        let dir = self.pending[slot].dir.clone();
        let minutes = STALL_LIMIT.as_secs() / 60;
        self.pending.remove(slot);
        request.cancel();
        if let Some(chat) = self.chats.iter_mut().find(|chat| chat.dir == dir) {
            chat.messages.push(ChatMessage::note(
                format!(
                    "这次回答已经 {minutes} 分钟没有任何进展，已停止。可以重试，或把问题拆小一点。"
                ),
                true,
            ));
            chat.touch();
        }
        self.banner = Some(Banner::error(format!(
            "助手 {minutes} 分钟没有响应，已停止这次请求"
        )));
        self.save_chats();
        true
    }

    /// Move whatever the assistant produced into the conversation.
    pub(crate) fn pump_chat(&mut self, request: &Arc<AiRequest>, cx: &mut Context<Self>) {
        let events = request.poll();
        if events.is_empty() {
            return;
        }
        // Any event counts as progress: the stall watchdog reads this instead of
        // guessing from the transcript.
        self.pending
            .iter_mut()
            .filter(|pending| Arc::ptr_eq(&pending.request, request))
            .for_each(|pending| pending.last_event = Instant::now());
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
                    // A new round starts with its own thinking row; the previous
                    // round's text is already stored in its own message.
                    self.pending[slot].thinking_row = None;
                }
                AiEvent::Reasoning(text) => {
                    self.pending[slot].reasoning.push_str(&text);
                    // One row per continuous stretch: a tool call or a new round
                    // closes it, so thinking and tools stay in the order they
                    // happened.
                    let round = self.pending[slot].round.max(1);
                    let open = self.pending[slot].thinking_row;
                    let mut created = false;
                    let mut row_index = open;
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        let usable = open.filter(|index| {
                            chat.messages
                                .get(*index)
                                .is_some_and(|row| row.role == Role::Thinking && row.round == round)
                        });
                        match usable {
                            Some(index) => {
                                chat.messages[index].text.push_str(&text);
                                chat.touch();
                            }
                            None => {
                                let mut row = ChatMessage::thinking(String::new(), round);
                                row.text.push_str(&text);
                                chat.messages.push(row);
                                row_index = Some(chat.messages.len() - 1);
                                created = true;
                                chat.touch();
                            }
                        }
                    }
                    if created {
                        self.pending[slot].thinking_rows += 1;
                    }
                    if row_index.is_some() {
                        self.pending[slot].thinking_row = row_index;
                    }
                }
                AiEvent::ToolStart { call, tool, target } => {
                    if call.is_empty() {
                        continue;
                    }
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    // The tool's row comes after the thinking that asked for it.
                    self.pending[slot].thinking_row = None;
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
                    // The answer started: the next thinking fragment belongs to
                    // a new row, not to the one that led into this text.
                    self.pending[slot].thinking_row = None;
                    self.pending[slot].draft.push_str(&text);
                }
                AiEvent::Tools {
                    items,
                    prompts,
                    errors,
                    mcp_errors,
                    plugin_tools,
                    mcp_tools,
                    panels,
                } => {
                    // The context panel shows what the next request carries, so
                    // the schemas and plugin prompts are kept until the engine
                    // sends a new set; the settings page shows the failures, and
                    // the plugin/MCP cards count what each one contributed. The
                    // panels belong to this request, so they stay with it.
                    // This run needs its own copy: the first question of a
                    // conversation was recorded before any schema arrived.
                    self.pending[slot].schemas = items.clone();
                    // The tools page draws the engine's own description of each
                    // tool, so it is read out of the schema once per run here
                    // instead of parsed again on every frame.
                    for (name, schema) in &items {
                        if let Some(text) = super::tool_catalog::engine_description(schema) {
                            self.tool_descriptions.insert(name.clone(), text);
                        }
                    }
                    self.tool_schemas.insert(dir.clone(), items);
                    self.plugin_prompts = prompts;
                    self.plugin_errors = errors;
                    self.mcp_errors = mcp_errors;
                    self.plugin_tools = plugin_tools;
                    self.mcp_tools = mcp_tools;
                    self.plugin_panels.insert(dir.clone(), panels);
                }
                AiEvent::Step {
                    tool,
                    target,
                    ok,
                    detail,
                    input,
                    output,
                    call,
                    image_mime,
                    image_path,
                    files,
                } => {
                    // The model wrote something and then went for a tool: that
                    // text belongs to the round, not to the answer.
                    self.pending[slot].approval = None;
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    // The statistics ledger counts calls per tool, not only the
                    // total, so the name is kept as the step finishes.
                    if !tool.is_empty()
                        && self.pending[slot].tool_names.len() < super::TOOL_NAME_LOG_MAX
                    {
                        self.pending[slot].tool_names.push(tool.clone());
                    }
                    // The thinking that led here is complete; a later fragment
                    // starts a row of its own.
                    self.pending[slot].thinking_row = None;
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
                            apply_image(message, &image_mime, &image_path);
                            message.files = files;
                            apply_write(message, write);
                        } else {
                            let mut message = ChatMessage::tool(text, !ok);
                            message.input = input;
                            message.output = output;
                            message.call = call.clone();
                            apply_image(&mut message, &image_mime, &image_path);
                            message.files = files;
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
                event @ AiEvent::Approval { .. } => {
                    self.handle_approval(slot, chat_index, event);
                }
                AiEvent::Question {
                    call,
                    question,
                    options,
                    allow_custom,
                } => {
                    if call.is_empty() {
                        continue;
                    }
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    self.pending[slot].thinking_row = None;
                    self.pending[slot].question = Some(call.clone());
                    // Only one interaction can be live at a time.
                    self.pending[slot].plan = None;
                    let payload = serde_json::json!({
                        "options": options,
                        "allowCustom": allow_custom,
                    })
                    .to_string();
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        if let Some(existing) = chat
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.role == Role::Question && entry.call == call)
                        {
                            existing.text = question;
                            existing.input = payload;
                        } else {
                            chat.messages
                                .push(ChatMessage::question(call, question, payload));
                        }
                        chat.touch();
                    }
                    self.save_chats();
                }
                AiEvent::Plan { call, plan } => {
                    if call.is_empty() {
                        continue;
                    }
                    self.pending[slot].draft.clear();
                    self.pending[slot].acted = true;
                    self.pending[slot].thinking_row = None;
                    self.pending[slot].plan = Some(call.clone());
                    self.pending[slot].question = None;
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        if let Some(existing) = chat
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|entry| entry.role == Role::Plan && entry.call == call)
                        {
                            existing.text = plan;
                        } else {
                            chat.messages.push(ChatMessage::plan_review(call, plan));
                        }
                        chat.touch();
                    }
                    self.save_chats();
                }
                AiEvent::Todo { items } => {
                    if let Some(chat) = self.chats.get_mut(chat_index) {
                        chat.todos = items
                            .into_iter()
                            .map(|(text, status)| TodoItem {
                                text,
                                status: TodoStatus::from_key(&status),
                            })
                            .collect();
                        chat.touch();
                    }
                    self.save_chats();
                }
                AiEvent::Memory { text } => {
                    // The engine only reports the entry; the file belongs to the
                    // window. A failure is shown, because a memory the model
                    // believes it kept would otherwise be lost silently.
                    if let Err(error) = crate::memory::append(&self.ai.data_dir(), &text) {
                        if let Some(chat) = self.chats.get_mut(chat_index) {
                            chat.messages.push(ChatMessage::note(error, true));
                            chat.touch();
                        }
                        self.save_chats();
                    }
                }
                event @ AiEvent::Done { .. } => {
                    self.finish_answer(slot, chat_index, event, cx);
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
                                let selection = self.ai.mode_selection(&retry.mode);
                                let allowed_list = mode_tools_for(&selection, connection.vision);
                                let plugins = selection.plugins.clone();
                                let mcp_servers =
                                    crate::mcp::enabled_for(&self.ai, &selection.mcp_servers);
                                let plugin_dir = crate::config::AiSettings::plugin_dir();
                                AiRequest::ask(
                                    &self.engine_dir,
                                    &retry.work_dir,
                                    &retry.prompt,
                                    Some(&retry.system),
                                    &retry.history,
                                    &connection,
                                    retry.tools,
                                    if retry.tools {
                                        Some(&allowed_list[..])
                                    } else {
                                        None
                                    },
                                    retry.read_only,
                                    retry.scope_root.as_deref(),
                                    &retry.read_only_roots,
                                    retry.max_rounds,
                                    retry.max_calls,
                                    retry.plan,
                                    retry.tools,
                                    retry.web_search,
                                    &retry.attachments,
                                    plugin_dir.as_deref(),
                                    &plugins,
                                    &mcp_servers,
                                    Some(&self.mcp_dir),
                                    &retry.skip_dirs,
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
                        settle_interactions(chat, "已取消");
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

    /// Pictures the previous answer showed, ready to send with this question.
    ///
    /// Only files that a tool saved are considered, only files still inside the
    /// conversation's own scope are read, and the total is capped: the same
    /// picture rides on every round of the question, so two large files would
    /// cost more than the context they are worth.
    /// @param chat_index - the conversation being asked.
    /// @param work_dir - its folder, which relative paths resolve from.
    /// @param scope - the writable root, or `None` in full access.
    /// @returns the attachments to send, newest last.
    fn recent_image_attachments(
        &self,
        chat_index: usize,
        work_dir: &Path,
        scope: Option<&Path>,
    ) -> Vec<crate::ai::AskAttachment> {
        /// Most pictures carried into the next question.
        const MAX_IMAGES: usize = 2;
        /// Largest single file that is read.
        const MAX_FILE: u64 = 3 * 1024 * 1024;
        /// Total bytes the carried pictures may add up to.
        const MAX_TOTAL: usize = 4 * 1024 * 1024;
        let Some(chat) = self.chats.get(chat_index) else {
            return Vec::new();
        };
        let mut out: Vec<crate::ai::AskAttachment> = Vec::new();
        let mut total = 0usize;
        for (mime, path) in recent_images(&chat.messages, MAX_IMAGES) {
            let Some(target) = resolve_scoped(work_dir, scope, &path) else {
                continue;
            };
            let Ok(info) = std::fs::metadata(&target) else {
                continue;
            };
            if !info.is_file() || info.len() == 0 || info.len() > MAX_FILE {
                continue;
            }
            let Ok(bytes) = std::fs::read(&target) else {
                continue;
            };
            if total + bytes.len() > MAX_TOTAL {
                continue;
            }
            total += bytes.len();
            let name = target
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "screenshot".to_string());
            out.push(crate::ai::AskAttachment::new(&name, &mime, "image", &bytes));
        }
        out
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
            settle_interactions(chat, "已取消");
            chat.touch();
        }
        self.save_chats();
        cx.notify();
    }

    /// Answer the structured question whose card sits at `index`.
    pub(crate) fn answer_question(&mut self, index: usize, choice: String, cx: &mut Context<Self>) {
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
        if message.role != Role::Question || !message.output.is_empty() {
            return;
        }
        let call = message.call.clone();
        if call.is_empty() {
            return;
        }
        let Some(slot) = self.pending.iter().position(|pending| pending.dir == dir) else {
            return;
        };
        if self.pending[slot].question.as_deref() != Some(call.as_str()) {
            return;
        }
        let delivered = self.pending[slot].request.respond_question(&call, &choice);
        self.pending[slot].question = None;
        if let Some(message) = self
            .chats
            .get_mut(self.active_chat)
            .and_then(|chat| chat.messages.get_mut(index))
        {
            message.output = if delivered {
                format!("已选择：{choice}")
            } else {
                "已失效".to_string()
            };
        }
        self.save_chats();
        cx.notify();
    }

    /// Approve or reject the plan whose card sits at `index`.
    pub(crate) fn answer_plan(&mut self, index: usize, approved: bool, cx: &mut Context<Self>) {
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
        if message.role != Role::Plan || !message.output.is_empty() {
            return;
        }
        let call = message.call.clone();
        if call.is_empty() {
            return;
        }
        let Some(slot) = self.pending.iter().position(|pending| pending.dir == dir) else {
            return;
        };
        if self.pending[slot].plan.as_deref() != Some(call.as_str()) {
            return;
        }
        let delivered = self.pending[slot].request.respond_plan(&call, approved);
        self.pending[slot].plan = None;
        if approved && delivered {
            // A fallback retry of this question must not re-enter plan mode.
            self.pending[slot].retry.plan = false;
        }
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            // An approved plan ends plan mode for the next question; the current
            // run keeps its guidance so the model can start executing.
            if approved && delivered {
                chat.plan = false;
            }
            if let Some(message) = chat.messages.get_mut(index) {
                message.output = match (delivered, approved) {
                    (true, true) => "已批准".to_string(),
                    (true, false) => "已拒绝，继续修改".to_string(),
                    (false, _) => "已失效".to_string(),
                };
            }
            chat.touch();
        }
        self.save_chats();
        cx.notify();
    }
}

/// A stable identity for a connection, so one question never retries it twice.
fn connection_key(connection: &Connection) -> String {
    format!(
        "{}|{}|{}",
        connection.provider, connection.base_url, connection.model
    )
}

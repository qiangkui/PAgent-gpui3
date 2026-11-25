//! Connecting to model providers: listing models, testing a key, and saving.

use super::*;

/// Where the active connection lands after `removed` was taken out of the list.
///
/// Removing a card *before* the active one shifts it down; only a card at or
/// after it can leave the index pointing past the end.
pub(crate) fn active_after_removal(active: usize, removed: usize, remaining: usize) -> usize {
    if remaining == 0 {
        return 0;
    }
    let active = if removed < active { active - 1 } else { active };
    active.min(remaining - 1)
}

impl DshFlowApp {
    /// Copy the editable connection fields back into the settings.
    ///
    /// Pick the temperature for one connection; `None` leaves it to the endpoint.
    ///
    /// It rides along with every request instead of being proven by a test, so
    /// moving the slider does not invalidate the endpoint.
    pub(crate) fn set_temperature(
        &mut self,
        index: usize,
        temperature: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(connection) = self.ai.connections.get_mut(index) {
            connection.temperature = temperature;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        cx.notify();
    }

    /// Pick whether the model thinks first, and how hard.
    ///
    /// Like the temperature it rides along with every request instead of being
    /// proven by a test, so switching it keeps the endpoint selectable.
    pub(crate) fn set_reasoning(
        &mut self,
        index: usize,
        reasoning: Reasoning,
        cx: &mut Context<Self>,
    ) {
        if let Some(connection) = self.ai.connections.get_mut(index) {
            connection.reasoning = reasoning;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        cx.notify();
    }

    /// Cap how many tokens one answer may generate; zero sends nothing.
    pub(crate) fn set_max_tokens(&mut self, index: usize, value: u32, cx: &mut Context<Self>) {
        if let Some(connection) = self.ai.connections.get_mut(index) {
            connection.max_tokens = value;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        cx.notify();
    }

    /// Say how large this endpoint's context window is; zero asks the table.
    ///
    /// The engine compacts at 80% of this figure and the gauge reads it, so a
    /// local model smaller than the 128k default no longer overflows before
    /// anything is compressed.
    pub(crate) fn set_window(&mut self, index: usize, value: u64, cx: &mut Context<Self>) {
        if let Some(connection) = self.ai.connections.get_mut(index) {
            connection.window = value;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        cx.notify();
    }

    /// Editing a field invalidates an earlier test: the model has to be proven
    /// again before it shows up in the picker.
    pub(crate) fn sync_connections(&mut self) {
        let Self {
            ai, conn_fields, ..
        } = self;
        for (index, fields) in conn_fields.iter().enumerate() {
            let Some(connection) = ai.connections.get_mut(index) else {
                continue;
            };
            let edited = Connection {
                id: connection.id.clone(),
                provider: connection.provider.clone(),
                model: fields.model.text.clone(),
                api_key: fields.api_key.text.clone(),
                base_url: fields.base_url.text.clone(),
                verified: connection.verified,
                models: connection.models.clone(),
                temperature: connection.temperature,
                reasoning: connection.reasoning,
                max_tokens: connection.max_tokens,
                window: connection.window,
            }
            .normalized();
            if edited.model != connection.model
                || edited.api_key != connection.api_key
                || edited.base_url != connection.base_url
            {
                connection.verified = false;
            }
            connection.model = edited.model;
            connection.api_key = edited.api_key;
            connection.base_url = edited.base_url;
        }
    }

    /// Rebuild the editable fields from the stored connections.
    pub(crate) fn rebuild_conn_fields(&mut self, cx: &mut Context<Self>) {
        self.conn_fields = build_conn_fields(&self.ai, cx);
        self.conn_status = vec![None; self.ai.connections.len()];
        self.listing = None;
    }

    /// Read the model list an endpoint reports, through the engine.
    ///
    /// The window cannot reach the network itself, and any list it shipped with
    /// would be out of date, so the provider is asked what it currently offers.
    pub(crate) fn list_models(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.listing.is_some() {
            return;
        }
        self.sync_connections();
        let Some(connection) = self.ai.connections.get(index).cloned() else {
            return;
        };
        // Remembered by id: adding or removing another card shifts indices, and
        // a shifted index would make a forgotten endpoint look already asked.
        if !connection.id.is_empty() {
            self.list_attempted.insert(connection.id.clone());
        }
        if connection.api_key.trim().is_empty() || connection.base_url.trim().is_empty() {
            if let Some(slot) = self.conn_status.get_mut(index) {
                *slot = Some(("先填好 API Key 与 Base URL".to_string(), true));
            }
            cx.notify();
            return;
        }
        if let Some(slot) = self.conn_status.get_mut(index) {
            *slot = Some(("正在读取模型列表…".to_string(), false));
        }
        let engine_dir = self.engine_dir.clone();
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(crate::ai::list_models(&engine_dir, &connection));
        });
        self.listing = Some(ListState { index, result });
        self.spawn_list_pump(cx);
        cx.notify();
    }

    /// Poll a model-list read while it runs.
    pub(crate) fn spawn_list_pump(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.pump_list(cx);
                        cx.notify();
                        view.listing.is_some()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Record the result of a finished model-list read.
    pub(crate) fn pump_list(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.listing.as_ref() else {
            return;
        };
        let index = state.index;
        let outcome = match state.result.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("读取模型列表的线程意外退出".to_string())
            }
        };
        self.listing = None;
        match outcome {
            Ok(list) => {
                if let Some(connection) = self.ai.connections.get_mut(index) {
                    connection.models = list.models;
                }
                // The versions appear as buttons in the card, so success needs no
                // text; a degraded listing explains why it holds one name.
                match self.ai.save() {
                    Ok(()) => {
                        if let Some(slot) = self.conn_status.get_mut(index) {
                            *slot = list.note.map(|note| (note, false));
                        }
                    }
                    Err(error) => {
                        if let Some(slot) = self.conn_status.get_mut(index) {
                            *slot = Some((error, true));
                        }
                    }
                }
            }
            Err(error) => {
                if let Some(slot) = self.conn_status.get_mut(index) {
                    *slot = Some((error, true));
                }
            }
        }
        self.refresh_model_lists(cx);
    }

    /// Ask endpoints for their model list, once per session.
    ///
    /// Opening Settings is the moment the versions matter, so a connection that
    /// has never reported its models asks for them then; the answer is cached in
    /// the config, so this only happens when there is nothing to show.
    pub(crate) fn refresh_model_lists(&mut self, cx: &mut Context<Self>) {
        if self.listing.is_some() {
            return;
        }
        let candidate = self
            .ai
            .connections
            .iter()
            .enumerate()
            .position(|(_, connection)| {
                connection.models.is_empty()
                    && !self.list_attempted.contains(&connection.id)
                    && !connection.api_key.trim().is_empty()
                    && !connection.base_url.trim().is_empty()
            });
        if let Some(index) = candidate {
            self.list_models(index, cx);
        }
    }

    /// 用量上限: remember the reminder limit for new questions.
    pub(crate) fn set_budget(&mut self, tokens: u64, cx: &mut Context<Self>) {
        if self.ai.budget_tokens == tokens {
            return;
        }
        self.ai.budget_tokens = tokens;
        self.ai_status = Some(match self.ai.save() {
            Ok(()) => ("已保存用量上限".to_string(), false),
            Err(error) => (error, true),
        });
        cx.notify();
    }

    /// Persist an edited connection field, once the typing pauses.
    ///
    /// The AI page has no save button: typing a model, key or base URL writes the
    /// settings file as soon as the text changes. The in-memory value is updated
    /// immediately, but the file (and the credential store with it) is written
    /// after [`AI_SAVE_DEBOUNCE`] of quiet, so a key is one save rather than one
    /// per keystroke. Nothing is written when the text did not actually change
    /// (arrow keys, selection, a refused paste).
    pub(crate) fn commit_edit(&mut self, slot: Field, cx: &mut Context<Self>) {
        if let Field::ColorHex(index) = slot {
            self.commit_color_edit(index, cx);
            return;
        }
        if slot == Field::ModeName {
            self.commit_mode_name(cx);
            return;
        }
        if slot == Field::Proxy {
            let proxy = self.proxy_field.text.trim().to_string();
            if proxy != self.ai.proxy {
                self.ai.proxy = proxy;
                // The children read this the next time one is started.
                crate::engine::set_proxy(&self.ai.proxy);
                self.ai_status = Some(("正在保存…".to_string(), false));
                self.schedule_settings_save(cx);
                cx.notify();
            }
            return;
        }
        if !matches!(
            slot,
            Field::ConnModel(_) | Field::ConnKey(_) | Field::ConnUrl(_)
        ) {
            return;
        }
        let before = self.ai.connections.clone();
        self.sync_connections();
        if self.ai.connections == before {
            return;
        }
        self.ai_status = Some(("正在保存…".to_string(), false));
        self.schedule_settings_save(cx);
        cx.notify();
    }

    /// Write the settings once the typing has settled.
    fn schedule_settings_save(&mut self, cx: &mut Context<Self>) {
        let generation = self.ai_save_generation.wrapping_add(1);
        self.ai_save_generation = generation;
        self.ai_save_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AI_SAVE_DEBOUNCE).await;
            this.update(cx, |view, cx| {
                // A later keystroke owns the save now; it will write the newest
                // text when its own timer fires.
                if view.ai_save_generation != generation {
                    return;
                }
                view.flush_settings();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Write the settings now, whatever the debounce timer says.
    ///
    /// Used as the window closes: a change typed a moment ago must not be lost
    /// with the process.
    pub(crate) fn flush_settings(&mut self) {
        if !self.ai_save_pending {
            return;
        }
        self.ai_save_pending = false;
        self.sync_connections();
        self.ai_status = Some(match self.ai.save() {
            Ok(()) => ("已自动保存".to_string(), false),
            Err(error) => (error, true),
        });
    }

    /// Add a connection from a preset, ready for a key.
    pub(crate) fn add_connection(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(mut connection) = Connection::from_preset(provider) else {
            return;
        };
        self.sync_connections();
        connection.id = AiSettings::fresh_connection_id(&self.ai.connections);
        self.ai.connections.push(connection);
        self.rebuild_conn_fields(cx);
        self.ai_status = Some(match self.ai.save() {
            Ok(()) => ("已添加，填好 Key 后点「测试」".to_string(), false),
            Err(error) => (error, true),
        });
        cx.notify();
    }

    /// Drop a connection.
    pub(crate) fn remove_connection(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.ai.connections.len() {
            return;
        }
        // The key goes with the card: an orphan in the credential store would be
        // a secret nobody can see or revoke from this window any more.
        if let Some(gone) = self.ai.connections.get(index)
            && !gone.id.is_empty()
        {
            crate::secrets::forget_connection(&gone.id);
        }
        self.ai.connections.remove(index);
        self.ai.active = active_after_removal(self.ai.active, index, self.ai.connections.len());
        self.rebuild_conn_fields(cx);
        self.ai_status = Some(match self.ai.save() {
            Ok(()) => ("已删除".to_string(), false),
            Err(error) => (error, true),
        });
        cx.notify();
    }

    /// Send a tiny prompt through one connection to prove it works.
    ///
    /// Only connections that answered are offered in the composer, so this is what
    /// turns a stored key into a selectable model.
    pub(crate) fn test_connection(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sync_connections();
        let Some(connection) = self.ai.connections.get(index).cloned() else {
            return;
        };
        let status = |self_: &mut Self, text: String, error: bool| {
            if let Some(slot) = self_.conn_status.get_mut(index) {
                *slot = Some((text, error));
            }
        };
        if !connection.ready() {
            status(self, "先填写模型、Key 与 Base URL".to_string(), true);
            cx.notify();
            return;
        }
        match AiRequest::ask(
            &self.engine_dir,
            &self.engine_dir,
            "ping",
            Some("这是一次连通性测试，只回复 ok。"),
            &[],
            &connection,
            false,
            None,
            false,
            None,
            &[],
            crate::config::DEFAULT_MAX_ROUNDS,
            crate::config::DEFAULT_MAX_CALLS,
            false,
            true,
            &[],
            // A connectivity test never carries plugins: it is about the key.
            None,
            &[],
            &[],
            None,
        ) {
            Ok(request) => {
                status(self, format!("测试中 · {}", connection.model), false);
                self.testing = Some(TestState {
                    index,
                    request: Arc::new(request),
                });
                self.spawn_test_pump(cx);
            }
            Err(error) => status(self, error, true),
        }
        cx.notify();
    }

    /// Poll a connection test while it runs.
    pub(crate) fn spawn_test_pump(&self, cx: &mut Context<Self>) {
        let Some(request) = self.testing.as_ref().map(|test| test.request.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.pump_test(&request);
                        cx.notify();
                        view.testing.is_some()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Record the result of a finished connection test.
    pub(crate) fn pump_test(&mut self, request: &Arc<AiRequest>) {
        let events = request.poll();
        if events.is_empty() {
            return;
        }
        let Some(test) = self.testing.as_ref() else {
            return;
        };
        let index = test.index;
        let mut result: Option<(String, bool)> = None;
        for event in events {
            result = Some(match event {
                AiEvent::Done { text, .. } => {
                    if let Some(connection) = self.ai.connections.get_mut(index) {
                        connection.verified = true;
                        let model = connection.model.clone();
                        let _ = self.ai.save();
                        (format!("已接入 · {model}（回复：{}）", text.trim()), false)
                    } else {
                        ("这条连接已经不存在".to_string(), true)
                    }
                }
                AiEvent::Failed(error) => {
                    if let Some(connection) = self.ai.connections.get_mut(index) {
                        connection.verified = false;
                    }
                    (error, true)
                }
                // The ping never declares tools, so this cannot happen.
                AiEvent::Step { .. }
                | AiEvent::ToolStart { .. }
                | AiEvent::ToolOutput { .. }
                | AiEvent::Progress { .. }
                | AiEvent::Approval { .. }
                | AiEvent::Question { .. }
                | AiEvent::Plan { .. }
                | AiEvent::Todo { .. } => continue,
                // Fragments only matter while an answer is being written; a
                // test keeps the finished text from `Done`.
                AiEvent::Delta(_) => continue,
                // Thinking is not the answer either; a test reads `Done`.
                AiEvent::Reasoning(_) => continue,
                // Tool schemas belong to the context panel, not to a test.
                AiEvent::Tools { .. } => continue,
            });
        }
        if let Some(slot) = self.conn_status.get_mut(index) {
            *slot = result;
        }
        self.testing = None;
    }

    /// Switch a connection to one of its versions.
    ///
    /// Versions the endpoint reported (or the preset ships with) are known to be
    /// callable, so a connection that already passed a test stays usable; a
    /// hand-typed model has not been proven and goes back to 未测试.
    pub(crate) fn set_connection_model(&mut self, index: usize, model: &str) {
        let Some(connection) = self.ai.connections.get_mut(index) else {
            return;
        };
        if connection.model == model {
            return;
        }
        let known = connection.models.iter().any(|name| name == model)
            || model_versions(&connection.provider).contains(&model);
        connection.model = model.to_string();
        if !known {
            connection.verified = false;
        }
    }

    /// Switch a version from the settings card.
    pub(crate) fn choose_model(&mut self, index: usize, model: &str, cx: &mut Context<Self>) {
        if let Some(fields) = self.conn_fields.get_mut(index) {
            fields.model.set_text(model.to_string());
        }
        self.sync_connections();
        self.set_connection_model(index, model);
        let verified = self
            .ai
            .connections
            .get(index)
            .is_some_and(|connection| connection.verified);
        self.ai_status = Some(match self.ai.save() {
            Ok(()) if verified => (format!("已切换到 {model}"), false),
            Ok(()) => (format!("已切换到 {model}，点「测试」重新接入"), false),
            Err(error) => (error, true),
        });
        cx.notify();
    }
}

/// Build the editable fields for every stored connection.
pub(crate) fn build_conn_fields(ai: &AiSettings, cx: &mut Context<DshFlowApp>) -> Vec<ConnFields> {
    ai.connections
        .iter()
        .map(|connection| {
            let mut model = TextField::new(cx);
            model.set_text(connection.model.clone());
            let mut api_key = TextField::new(cx);
            api_key.set_text(connection.api_key.clone());
            let mut base_url = TextField::new(cx);
            base_url.set_text(connection.base_url.clone());
            ConnFields {
                model,
                api_key,
                base_url,
            }
        })
        .collect()
}

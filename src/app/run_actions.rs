//! Starting, pumping, and stopping a workflow run, plus the status-bar summary.

use super::*;
use super::run_error::RunErrorKind;

impl DshFlowApp {
    /// Start the selected workflow on a background process.
    pub(crate) fn start_run(&mut self, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        let Some(entry) = self.selected_entry().cloned() else {
            self.banner = Some(Banner::error(RunErrorKind::Workflow.prefixed(
                "还没有选中工作流：在左侧选择一张工作流卡片，或把 DSH_FLOW_WORKFLOWS 指向工作流目录。",
            )));
            cx.notify();
            return;
        };
        // A run always shows on the current tab, whatever the reader was looking at.
        self.flow_tab = FlowTab::Current;
        self.history_open = None;
        self.selected_run = None;

        // A local format check first: an empty required input can only end as a
        // failure inside the engine, a few seconds later. Blocking it here keeps
        // the window from starting a process it already knows must fail; the
        // engine stays the authority for everything else (defaults, types,
        // secrets, paths).
        let missing: Vec<String> = self
            .inputs
            .iter()
            .filter(|input| input.spec.required && input.field.text.trim().is_empty())
            .map(|input| input.spec.name.clone())
            .collect();
        if !missing.is_empty() {
            self.banner = Some(Banner::error(RunErrorKind::Input.prefixed(&format!(
                "缺少必填输入：{}。在右侧输入卡里填好后再运行。",
                missing.join("、")
            ))));
            cx.notify();
            return;
        }
        // A value revealed with "按住显示" goes back behind dots the moment the
        // run starts.
        self.revealed_input = None;

        // A secret input never becomes `--input`: it would land on the command
        // line, in the process list, and in the log pane's first line. It travels
        // on the child's stdin with the endpoint credentials, and the pane shows
        // `***`.
        let mut inputs: Vec<(String, String)> = Vec::new();
        let mut secrets: Vec<(String, String)> = Vec::new();
        let mut shown: Vec<(String, String)> = Vec::new();
        for input in &self.inputs {
            let value = input.field.text.trim().to_string();
            if value.is_empty() {
                continue;
            }
            if input.spec.secret {
                secrets.push((input.spec.name.clone(), value));
                shown.push((input.spec.name.clone(), "***".to_string()));
                continue;
            }
            inputs.push((input.spec.name.clone(), value.clone()));
            shown.push((input.spec.name.clone(), value));
        }

        // `ai` steps ask the same verified endpoint the chat uses; the key goes
        // to the child on stdin so it never reaches argv or the log pane.
        let credentials = self
            .ai
            .active()
            .filter(|connection| connection.verified && !connection.api_key.trim().is_empty())
            .map(|connection| RunCredentials {
                api_key: connection.api_key.clone(),
                base_url: connection.base_url.clone(),
                model: connection.model.clone(),
            });

        match RunSession::start(
            &self.engine_dir,
            &entry.path,
            &inputs,
            credentials.as_ref(),
            &secrets,
        ) {
            Ok(session) => {
                let mut log = vec![command_line(&entry, &inputs, &self.scan_root)];
                log.push("正在启动引擎".to_string());
                if credentials.is_none()
                    && entry
                        .spec
                        .steps
                        .iter()
                        .any(|step| step.kind == crate::manifest::StepKind::Ai)
                {
                    log.push(
                        RunErrorKind::Credential.prefixed(
                            "接入点没有可用 API Key，需要验证码/ai 的步骤会失败；\
                             已在对话里登录时，把 workspace 指向对话目录可跳过。",
                        ),
                    );
                }
                self.active = Some(ActiveRun {
                    session: Arc::new(session),
                    workflow_name: entry.name.clone(),
                    manifest: entry.path.clone(),
                    manifest_label: entry.label(&self.scan_root),
                    spec: entry.spec.clone(),
                    inputs: shown,
                    steps: Vec::new(),
                    log,
                });
                self.banner = None;
                self.selected_run = None;
                self.selected_step = None;
                // The transcript starts at the run's header, not wherever the
                // previous run was left scrolled to.
                self.transcript_scroll.set_offset(point(px(0.), px(0.)));
                self.spawn_run_pump(cx);
            }
            Err(error) => {
                self.active = None;
                self.banner = Some(Banner::error(
                    RunErrorKind::from_message(&error).prefixed(&error),
                ));
            }
        }
        cx.notify();
    }

    /// Poll the engine while it runs, so the log and timer stay live.
    pub(crate) fn spawn_run_pump(&self, cx: &mut Context<Self>) {
        let Some(session) = self.active.as_ref().map(|active| active.session.clone()) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.pump_run(&session);
                        cx.notify();
                        view.active.is_some()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Move whatever the engine produced into the view.
    pub(crate) fn pump_run(&mut self, session: &Arc<RunSession>) {
        let messages = session.poll();
        if messages.is_empty() {
            return;
        }
        for message in messages {
            match message {
                RunMessage::Line(line) => {
                    if let Some(active) = self.active.as_mut() {
                        push_log(&mut active.log, line);
                    } else if let Some(record) = self.history.first_mut() {
                        // Lines written just before the engine finished arrive
                        // after the summary; they belong to the run that just
                        // became the newest record.
                        push_log(&mut record.log, line);
                    }
                }
                RunMessage::Event(event) => {
                    // Live step states: a card turns green or red while the run
                    // is still going, instead of only when the summary lands.
                    if let Some(active) = self.active.as_mut() {
                        active.apply(&event);
                    }
                }
                RunMessage::Done(outcome) => self.finish(*outcome),
                RunMessage::Failed(error) => self.fail(error),
            }
        }
    }

    /// Stop the run in flight.
    pub(crate) fn cancel_run(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.session.is_cancelled() {
            return;
        }
        active.session.cancel();
        push_log(
            &mut active.log,
            "正在取消：已向引擎发送结束信号".to_string(),
        );
        cx.notify();
    }

    /// Stop a workflow run without going through the view, for the close path.
    ///
    /// The engine is a separate process: leaving it running once the window is
    /// gone means it keeps writing files with nobody watching. `RunSession` also
    /// kills its child when the last handle drops; doing it here makes the
    /// moment deterministic instead of waiting for the executor to shut down.
    pub(crate) fn stop_active_run(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.session.cancel();
        }
    }

    /// Turn the finished run into a history record.
    pub(crate) fn finish(&mut self, outcome: RunOutcome) {
        let Some(active) = self.active.take() else {
            return;
        };
        let cancelled = active.session.is_cancelled();
        let finished = Instant::now();
        self.push_record(RunRecord {
            workflow_name: active.workflow_name,
            manifest: active.manifest,
            manifest_label: active.manifest_label,
            spec: active.spec,
            inputs: active.inputs,
            outcome,
            log: active.log,
            cancelled,
            finished,
            finished_ms: epoch_ms(finished),
        });
    }

    /// Record a run the engine could not report on.
    pub(crate) fn fail(&mut self, error: String) {
        let Some(active) = self.active.take() else {
            return;
        };
        let cancelled = active.session.is_cancelled();
        let mut log = active.log;
        let error = if cancelled {
            push_log(&mut log, "已取消".to_string());
            "已取消".to_string()
        } else {
            let classified = RunErrorKind::from_message(&error).prefixed(&error);
            push_log(&mut log, format!("错误：{classified}"));
            error
        };
        let outcome = RunOutcome {
            ok: false,
            run_id: "-".to_string(),
            duration_ms: active.session.elapsed_ms(),
            steps: Vec::new(),
            error: Some(error),
            report: None,
            exit_code: None,
        };
        self.push_record(RunRecord {
            workflow_name: active.workflow_name,
            manifest: active.manifest,
            manifest_label: active.manifest_label,
            spec: active.spec,
            inputs: active.inputs,
            outcome,
            log,
            cancelled,
            finished: Instant::now(),
            finished_ms: now_ms(),
        });
    }

    /// Send the shown run to history and clear the current transcript.
    ///
    /// The record was already saved when the run finished; this only forgets the
    /// selection, so the left column goes back to "not run yet" and the right
    /// inputs become editable again. Until this is pressed, the current tab keeps
    /// showing that finished run.
    pub(crate) fn new_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        let name = self
            .selected_entry()
            .map(|entry| entry.name.clone())
            .or_else(|| {
                self.selected_record()
                    .map(|record| record.workflow_name.clone())
            });
        if let Some(name) = name {
            self.flow_cleared.insert(name);
            self.save_categories();
        }
        self.flow_tab = FlowTab::Current;
        self.history_open = None;
        self.selected_run = None;
        self.transcript_scroll.set_offset(point(px(0.), px(0.)));
        self.leave_fields(window, cx);
        cx.notify();
    }

    /// Store a finished run and show it.
    pub(crate) fn push_record(&mut self, record: RunRecord) {
        let workflow = record.workflow_name.clone();
        // The snapshot is what a later post-mortem reads; a failed write costs
        // that record, never the run itself.
        if let Err(error) = crate::run_store::save(&self.ai.data_dir(), &record.to_stored()) {
            crate::logging::error(&format!("保存运行快照失败：{error}"));
        }
        self.history.insert(0, record);
        self.history.truncate(MAX_HISTORY);
        self.flow_cleared.remove(&workflow);
        self.remember_run();
        if self.flow_tab == FlowTab::History {
            // A new run arrives at the top; an open record moves down one slot.
            if let Some(index) = self.history_open {
                self.history_open = Some(index + 1);
                self.selected_run = Some(index + 1);
            }
        } else {
            self.selected_run = Some(0);
        }
        // The static board keeps whatever card the reader opened; a run does not
        // move its selection.
        self.transcript_scroll.set_offset(point(px(0.), px(0.)));
    }

    /// Add the newest run to the saved history, so the card keeps its status
    /// after a restart. A failed write only costs that history, never the run.
    pub(crate) fn remember_run(&mut self) {
        let Some(record) = self.history.first() else {
            return;
        };
        self.past_runs.insert(
            0,
            crate::workflow_store::RunSummary {
                workflow: record.workflow_name.clone(),
                manifest: record.manifest_label.clone(),
                ok: record.outcome.ok,
                cancelled: record.cancelled,
                duration_ms: record.outcome.duration_ms,
                steps: record.outcome.steps.len(),
                failed_steps: record
                    .outcome
                    .steps
                    .iter()
                    .filter(|step| step.status != "ok")
                    .count(),
                finished_ms: epoch_ms(record.finished),
            },
        );
        // Every workflow keeps its own newest runs, in memory as on disk.
        let mut kept: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        self.past_runs.retain(|run| {
            let count = kept.entry(run.workflow.clone()).or_insert(0);
            *count += 1;
            *count <= crate::workflow_store::MAX_RUNS
        });
        self.save_categories();
    }

    /// Gather the workflow pane's view model.
    pub(crate) fn pane(&self) -> Option<Pane<'_>> {
        // A record opened from the history tab wins over a run in flight: the
        // reader asked for that one explicitly.
        if self.flow_tab == FlowTab::History
            && self.history_open.is_some()
            && let Some(record) = self.selected_record()
        {
            return Some(self.record_pane(record));
        }
        if let Some(active) = &self.active {
            return Some(Pane {
                title: active.workflow_name.clone(),
                description: active.spec.description.clone().unwrap_or_default(),
                manifest: Some(&active.manifest),
                rows: build_rows(&active.spec, &active.steps),
                outcome: None,
                inputs: active.inputs.clone(),
                running: true,
                cancelled: false,
                history: false,
                elapsed_ms: active.session.elapsed_ms(),
            });
        }
        if let Some(record) = self.selected_record() {
            return Some(self.record_pane(record));
        }
        self.selected_entry().map(|entry| Pane {
            title: entry.name.clone(),
            description: entry.description.clone(),
            manifest: Some(&entry.path),
            rows: build_rows(&entry.spec, &[]),
            outcome: None,
            inputs: Vec::new(),
            running: false,
            cancelled: false,
            history: false,
            elapsed_ms: 0,
        })
    }

    /// The view model of one finished run.
    fn record_pane<'a>(&'a self, record: &'a RunRecord) -> Pane<'a> {
        Pane {
            title: record.workflow_name.clone(),
            description: record.spec.description.clone().unwrap_or_default(),
            manifest: Some(&record.manifest),
            rows: build_rows(&record.spec, &record.outcome.steps),
            outcome: Some(&record.outcome),
            inputs: record.inputs.clone(),
            running: false,
            cancelled: record.cancelled,
            history: true,
            elapsed_ms: record.outcome.duration_ms,
        }
    }
}

/// Append a log line, dropping the oldest when the buffer is full.
pub(crate) fn push_log(log: &mut Vec<String>, line: String) {
    log.push(line);
    if log.len() > MAX_LOG_LINES {
        let excess = log.len() - MAX_LOG_LINES;
        log.drain(..excess);
    }
}

/// The command the view is about to run, for the log header.
pub(crate) fn command_line(
    entry: &WorkflowEntry,
    inputs: &[(String, String)],
    root: &Path,
) -> String {
    let mut line = format!("$ node src/cli.ts run {} --json", entry.label(root));
    for (name, value) in inputs {
        line.push_str(&format!(" --input {name}={value}"));
    }
    line
}

//! Starting, pumping, and stopping a workflow run, plus the status-bar summary.

use super::*;

impl DshFlowApp {
    /// Start the selected workflow on a background process.
    pub(crate) fn start_run(&mut self, cx: &mut Context<Self>) {
        if self.active.is_some() {
            return;
        }
        let Some(entry) = self.selected_entry().cloned() else {
            self.banner = Some(Banner::error(
                "还没有选中工作流：在左侧选择一张工作流卡片，或把 DSH_FLOW_WORKFLOWS 指向工作流目录。",
            ));
            cx.notify();
            return;
        };
        let inputs: Vec<(String, String)> = self
            .inputs
            .iter()
            .map(|input| (input.spec.name.clone(), input.field.text.trim().to_string()))
            .filter(|(_, value)| !value.is_empty())
            .collect();

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

        match RunSession::start(&self.engine_dir, &entry.path, &inputs, credentials.as_ref()) {
            Ok(session) => {
                let mut log = vec![command_line(&entry, &inputs, &self.scan_root)];
                log.push("正在启动引擎".to_string());
                self.active = Some(ActiveRun {
                    session: Arc::new(session),
                    workflow_name: entry.name.clone(),
                    manifest: entry.path.clone(),
                    manifest_label: entry.label(&self.scan_root),
                    spec: entry.spec.clone(),
                    inputs,
                    log,
                });
                self.banner = None;
                self.selected_run = None;
                self.selected_step = None;
                self.log_scroll.scroll_to_bottom();
                self.spawn_run_pump(cx);
            }
            Err(error) => {
                self.active = None;
                self.banner = Some(Banner::error(error));
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
        let follow = self.log_at_bottom();
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
                RunMessage::Done(outcome) => self.finish(*outcome),
                RunMessage::Failed(error) => self.fail(error),
            }
        }
        if follow {
            self.log_scroll.scroll_to_bottom();
        }
    }

    /// True when the log pane is scrolled to its end.
    pub(crate) fn log_at_bottom(&self) -> bool {
        let offset = self.log_scroll.offset();
        let max = self.log_scroll.max_offset();
        max.y + offset.y <= px(24.)
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
        self.push_record(RunRecord {
            workflow_name: active.workflow_name,
            manifest: active.manifest,
            manifest_label: active.manifest_label,
            spec: active.spec,
            inputs: active.inputs,
            outcome,
            log: active.log,
            cancelled,
            finished: Instant::now(),
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
            push_log(&mut log, format!("错误：{error}"));
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
        });
    }

    /// Store a finished run and show it.
    pub(crate) fn push_record(&mut self, record: RunRecord) {
        self.history.insert(0, record);
        self.history.truncate(MAX_HISTORY);
        self.remember_run();
        self.selected_run = Some(0);
        self.selected_step = None;
        self.log_scroll.scroll_to_bottom();
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

    /// Drop the log of whatever pane is visible.
    pub(crate) fn clear_log(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.log.clear();
            return;
        }
        if let Some(record) = self
            .selected_run
            .and_then(|index| self.history.get_mut(index))
        {
            record.log.clear();
        }
    }

    /// Open a report the engine wrote, without going through a shell.
    ///
    /// The address comes from the engine's own output, so it is treated as
    /// untrusted: an `http(s)` link goes to the browser through the checked
    /// opener, and a local path is only opened when it really is a file.
    pub(crate) fn open_report(
        &mut self,
        path: &str,
        manifest: Option<&Path>,
        cx: &mut Context<Self>,
    ) {
        let report = path.trim();
        if report.starts_with("http://") || report.starts_with("https://") {
            if let Err(error) = open_url(report) {
                self.banner = Some(Banner::error(format!("无法打开报告：{error}")));
            }
            cx.notify();
            return;
        }
        let report = PathBuf::from(report);
        let candidate = if report.is_absolute() {
            report
        } else {
            let from_engine = self.engine_dir.join(&report);
            let from_manifest = manifest
                .and_then(Path::parent)
                .map(|folder| folder.join(&report))
                .filter(|candidate| candidate.exists());
            if from_engine.exists() {
                from_engine
            } else if let Some(from_manifest) = from_manifest {
                from_manifest
            } else {
                report
            }
        };
        if !candidate.is_file() {
            self.banner = Some(Banner::error(format!(
                "找不到报告 {}",
                candidate.display()
            )));
            cx.notify();
            return;
        }
        if let Err(error) = open_path(&candidate) {
            self.banner = Some(Banner::error(format!("无法打开报告：{error}")));
        }
        cx.notify();
    }

    /// Gather the workflow pane's view model.
    pub(crate) fn pane(&self) -> Option<Pane<'_>> {
        if let Some(active) = &self.active {
            return Some(Pane {
                title: active.workflow_name.clone(),
                description: active.spec.description.clone().unwrap_or_default(),
                manifest_label: active.manifest_label.clone(),
                manifest: Some(&active.manifest),
                rows: build_rows(&active.spec, &[]),
                log: &active.log,
                outcome: None,
                inputs: active.inputs.clone(),
                running: true,
                cancelled: false,
                history: false,
                elapsed_ms: active.session.elapsed_ms(),
            });
        }
        if let Some(record) = self.selected_record() {
            return Some(Pane {
                title: record.workflow_name.clone(),
                description: record.spec.description.clone().unwrap_or_default(),
                manifest_label: record.manifest_label.clone(),
                manifest: Some(&record.manifest),
                rows: build_rows(&record.spec, &record.outcome.steps),
                log: &record.log,
                outcome: Some(&record.outcome),
                inputs: record.inputs.clone(),
                running: false,
                cancelled: record.cancelled,
                history: true,
                elapsed_ms: record.outcome.duration_ms,
            });
        }
        self.selected_entry().map(|entry| Pane {
            title: entry.name.clone(),
            description: entry.description.clone(),
            manifest_label: entry.label(&self.scan_root),
            manifest: Some(&entry.path),
            rows: build_rows(&entry.spec, &[]),
            log: &[],
            outcome: None,
            inputs: Vec::new(),
            running: false,
            cancelled: false,
            history: false,
            elapsed_ms: 0,
        })
    }
}

/// Colour a log line by what it seems to say.
pub(crate) fn log_color(line: &str, palette: Palette) -> u32 {
    let lower = line.to_lowercase();
    if line.starts_with('$') {
        return palette.accent;
    }
    if lower.contains("error")
        || lower.contains("错误")
        || lower.contains("失败")
        || lower.contains("failed")
    {
        return palette.failed;
    }
    if lower.contains("warn") || lower.contains("cancel") || lower.contains("取消") {
        return palette.warn;
    }
    if lower.contains("done") || lower.contains("完成") || lower.contains(" ok") {
        return palette.ok;
    }
    palette.muted
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

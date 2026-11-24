//! Step cards, the expanded step panel, the log, and the status line.

use super::*;

impl DshFlowApp {
    /// The plan, with results merged in when the pane shows a run.
    /// The workflow body: step cards on the left, the chosen step on the right.
    pub(crate) fn render_steps(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .child(self.render_step_cards(pane, palette, cx))
            .child(self.render_step_panel(pane, palette, cx))
    }

    /// One card per step, joined by a vertical line, newest state on each.
    pub(crate) fn render_step_cards(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let last = pane.rows.len().saturating_sub(1);
        let mut cards: Vec<AnyElement> = Vec::new();
        for (order, row) in pane.rows.iter().enumerate() {
            let selected = self.selected_step.as_deref() == Some(row.id.as_str());
            let color = status_color(&row.status, palette);
            let step_id = row.id.clone();
            let status = if row.status == "pending" {
                String::new()
            } else {
                row.status.clone()
            };
            let timing = if row.status == "pending" || row.status == "running" {
                String::new()
            } else {
                format_duration(row.duration_ms)
            };

            cards.push(
                div()
                    .id(("step-card", order))
                    .flex()
                    .flex_row()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected_step = Some(step_id.clone());
                        cx.notify();
                    }))
                    // The rail: a segment above the dot (except for the first
                    // step) and one below (except for the last), so consecutive
                    // cards read as one connected flow.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .w(px(12.))
                            .flex_shrink_0()
                            .when(order > 0, |this| {
                                this.child(div().w(px(2.)).h(px(8.)).bg(rgb(palette.border)))
                            })
                            .child(status_dot(color))
                            .when(order < last, |this| {
                                this.child(div().w(px(2.)).flex_1().bg(rgb(palette.border)))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .min_w(px(0.))
                            .mb_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(if selected { palette.sky } else { palette.panel }))
                            .shadow_sm()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(format!("{}", order + 1)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_sm()
                                            .text_color(rgb(palette.text))
                                            .child(row.id.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgb(palette.background))
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(row.kind.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(if row.summary.is_empty() {
                                                row.detail.clone()
                                            } else {
                                                row.summary.clone()
                                            }),
                                    )
                                    .when(!status.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(rgb(color))
                                                .child(status.clone()),
                                        )
                                    })
                                    .when(!timing.is_empty(), |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(rgb(palette.faint))
                                                .child(timing.clone()),
                                        )
                                    }),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if cards.is_empty() {
            cards.push(
                div()
                    .px_1()
                    .py_3()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("这个清单里没有可显示的步骤。")
                    .into_any_element(),
            );
        }

        div()
            .id("steps")
            .flex()
            .flex_col()
            .w(px(264.))
            .flex_shrink_0()
            .min_h(px(0.))
            .px_4()
            .py_3()
            .overflow_y_scroll()
            .track_scroll(&self.steps_scroll)
            .children(cards)
    }

    /// Pick one combined permission level for this conversation.
    pub(crate) fn set_tool_mode(&mut self, mode: ToolMode, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            mode.apply(chat);
        }
        self.close_composer_menus();
        self.save_chats();
        cx.notify();
    }

    /// How many model round-trips this conversation's questions may take.
    pub(crate) fn set_max_rounds(&mut self, value: u32, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.max_rounds = value.clamp(1, crate::config::MAX_ROUNDS_CHOICE);
        }
        self.close_composer_menus();
        self.save_chats();
        cx.notify();
    }

    /// How many tool calls this conversation's questions may make.
    pub(crate) fn set_max_calls(&mut self, value: u32, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.max_calls = value.clamp(1, crate::config::MAX_CALLS_CHOICE);
        }
        self.close_composer_menus();
        self.save_chats();
        cx.notify();
    }

    /// Close every menu that floats over the composer.
    pub(crate) fn close_composer_menus(&mut self) {
        self.model_menu = false;
        self.tools_menu = false;
        self.budget_menu = false;
        self.mode_menu = false;
        self.attach_menu = false;
    }

    /// The right half: the chosen step's content, then the run inputs.
    pub(crate) fn render_step_panel(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self
            .selected_step
            .as_ref()
            .and_then(|id| pane.rows.iter().find(|row| &row.id == id));
        let inputs = self.input_fields(pane, palette, cx);
        let inputs_section = (!inputs.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child("运行参数"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_start()
                        .gap_3()
                        .children(inputs),
                )
                .into_any_element()
        });

        let mut panel = div()
            .id("step-content")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .gap_3()
            .px_5()
            .py_3()
            .overflow_y_scroll();

        match selected {
            Some(row) => {
                panel = panel.child(self.render_step_content(row, palette));
                if let Some(section) = inputs_section {
                    panel = panel.child(section);
                }
            }
            None => {
                if let Some(section) = inputs_section {
                    panel = panel.child(section);
                }
                panel = panel
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child("步骤内容"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .whitespace_normal()
                            .child("点左侧任意一张步骤卡片，这里会显示它的命令、守卫与输出。"),
                    );
            }
        }
        panel
    }

    /// The content of one step: its guards, command and captured output.
    pub(crate) fn render_step_content(&self, row: &StepRow, palette: Palette) -> AnyElement {
        let color = status_color(&row.status, palette);
        let mut lines: Vec<AnyElement> = Vec::new();
        if !row.summary.is_empty() {
            lines.push(detail_line("概要", &row.summary, palette));
        }
        if let Some(when) = &row.when {
            lines.push(detail_line("条件", when, palette));
        }
        if let Some(save) = &row.save {
            lines.push(detail_line("输出键", save, palette));
        }
        if !row.notes.is_empty() {
            lines.push(detail_line("标记", &row.notes.join(", "), palette));
        }
        if let Some(command) = &row.command {
            lines.push(detail_line("命令", command, palette));
        }
        if let Some(model) = &row.model {
            let label = if row.cached {
                "模型 · 复用记录"
            } else {
                "模型"
            };
            lines.push(detail_line(label, model, palette));
        }
        if !row.detail.is_empty() {
            lines.push(detail_line("说明", &row.detail, palette));
        }

        let empty =
            lines.is_empty() && row.stderr.is_none() && row.stdout.is_none() && row.text.is_none();
        let mut detail = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_md()
            .bg(rgb(palette.panel))
            .shadow_sm()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(row.id.clone()),
                    )
                    .child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .bg(rgb(palette.background))
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(row.kind.clone()),
                    )
                    .child(div().text_xs().text_color(rgb(color)).child(
                        if row.status == "pending" {
                            "待运行".to_string()
                        } else {
                            row.status.clone()
                        },
                    ))
                    .when(row.status != "pending" && row.status != "running", |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(format_duration(row.duration_ms)),
                        )
                    }),
            )
            .children(lines);
        if let Some(stderr) = &row.stderr {
            detail = detail.child(detail_block("stderr", stderr, palette));
        }
        if let Some(stdout) = &row.stdout {
            detail = detail.child(detail_block("stdout", stdout, palette));
        }
        if let Some(text) = &row.text {
            detail = detail.child(detail_block("回答", text, palette));
        }
        if empty {
            detail = detail.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("这个步骤还没有更多内容：运行后会显示命令、耗时与输出。"),
            );
        }
        detail.into_any_element()
    }

    /// Engine output for the visible pane.
    pub(crate) fn render_log(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let total = pane.log.len();
        let start = total.saturating_sub(VISIBLE_LOG_LINES);
        let mut lines: Vec<AnyElement> = Vec::new();
        if start > 0 {
            lines.push(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(format!("另有 {start} 行较早的输出未显示"))
                    .into_any_element(),
            );
        }
        for line in &pane.log[start..] {
            lines.push(
                div()
                    .text_xs()
                    .text_color(rgb(log_color(line, palette)))
                    .whitespace_normal()
                    .child(line.clone())
                    .into_any_element(),
            );
        }
        if lines.is_empty() {
            lines.push(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("引擎输出会显示在这里。")
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .h(px(150.))
            .min_w(px(0.))
            .border_t_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.panel))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .px_4()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().text_color(rgb(palette.faint)).child("日志"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(format!("{total} 步")),
                            ),
                    )
                    .child(
                        div()
                            .id("clear-log")
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.clear_log();
                                cx.notify();
                            }))
                            .child("清空"),
                    ),
            )
            .child(
                div()
                    .id("log-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .gap_0p5()
                    .px_4()
                    .py_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.log_scroll)
                    .children(lines),
            )
    }

    /// Engine location, manifest count, and the run summary.
    pub(crate) fn render_status(&self, pane: &Pane<'_>, palette: Palette) -> impl IntoElement {
        let engine_color = if self.engine_ready {
            palette.ok
        } else {
            palette.failed
        };
        let summary = if pane.running {
            format!(
                "运行中 {} · 共 {} 步",
                format_duration(pane.elapsed_ms),
                pane.rows.len()
            )
        } else if let Some(outcome) = pane.outcome {
            let failed = outcome.count("failed");
            let skipped = outcome.count("skipped");
            let mut summary = format!(
                "{} · {} · {} 步{} · run {}",
                if pane.cancelled {
                    "已取消"
                } else if outcome.ok {
                    "成功"
                } else {
                    "失败"
                },
                format_duration(outcome.duration_ms),
                outcome.steps.len(),
                if failed + skipped > 0 {
                    format!("（{failed} 失败 / {skipped} 跳过）")
                } else {
                    String::new()
                },
                outcome.run_id,
            );
            if let Some(error) = &outcome.error {
                summary.push_str(" · ");
                summary.push_str(error);
            }
            if let Some(code) = outcome.exit_code.filter(|code| *code != 0) {
                summary.push_str(&format!(" · exit {code}"));
            }
            summary
        } else {
            format!("共 {} 步 · 按「运行」或 Ctrl+R", pane.rows.len())
        };

        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .gap_4()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .border_t_1()
            .border_color(rgb(palette.border))
            .bg(rgb(palette.background))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .child(status_dot(engine_color))
                    .child(
                        div()
                            .w(px(320.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(format!(
                                "{} · {} 个工作流",
                                self.engine_dir.display(),
                                self.workflows.len()
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(summary),
            )
    }
}

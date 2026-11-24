//! The workflow view: header, run history, and the input form.

use super::*;

impl DshFlowApp {
    /// The workflow pane: banner, header, inputs, steps, log, status bar.
    pub(crate) fn render_flow(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut root = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .text_color(rgb(palette.text));

        if self.banner.is_some() {
            root = root.children(self.render_banner(palette, cx));
        }

        let Some(pane) = self.pane() else {
            return root.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .p_8()
                    .child(
                        div()
                            .text_xl()
                            .text_color(rgb(palette.muted))
                            .child("还没有选中工作流"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.faint))
                            .whitespace_normal()
                            .child(
                                "用 DSH_FLOW_WORKFLOWS 指向包含 flow.yml 的目录，然后重启界面。",
                            ),
                    ),
            );
        };

        root.child(self.render_flow_header(&pane, palette, cx))
            .child(self.render_run_history(palette, cx))
            .child(self.render_steps(&pane, palette, cx))
            .child(self.render_log(&pane, palette, cx))
            .child(self.render_status(&pane, palette))
    }

    /// Recent runs of the visible workflow, as small chips.
    pub(crate) fn render_run_history(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let name = self
            .selected_entry()
            .map(|entry| entry.name.clone())
            .or_else(|| {
                self.selected_record()
                    .map(|record| record.workflow_name.clone())
            });
        let mut chips: Vec<AnyElement> = Vec::new();
        for (index, record) in self.history.iter().enumerate() {
            if let Some(name) = &name
                && &record.workflow_name != name
            {
                continue;
            }
            if chips.len() >= 4 {
                break;
            }
            let selected = self.selected_run == Some(index);
            chips.push(
                div()
                    .id(("run-history", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(if selected {
                        palette.accent
                    } else {
                        palette.border
                    }))
                    .when(selected, |this| this.bg(rgb(palette.panel_soft)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.selected_run = Some(index);
                        view.selected_step = None;
                        view.leave_fields(window, cx);
                        cx.notify();
                    }))
                    .child(status_dot(record.color(palette)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format_duration(record.outcome.duration_ms)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(ago(epoch_ms(record.finished))),
                    )
                    .into_any_element(),
            );
        }
        if chips.is_empty() {
            return div().h(px(0.));
        }
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("最近运行"),
            )
            .children(chips)
    }

    /// Workflow identity, run state, and the action buttons.
    pub(crate) fn render_flow_header(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let (label, color) = pane_status(pane, palette);
        let running = pane.running;
        let report = pane.outcome.and_then(|outcome| outcome.report.clone());
        let report_manifest = pane.manifest.map(Path::to_path_buf);

        let mut actions: Vec<AnyElement> = Vec::new();
        if let Some(report) = report {
            actions.push(
                div()
                    .id("report")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(palette.panel_soft))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.open_report(&report, report_manifest.as_deref(), cx);
                    }))
                    .child("报告")
                    .into_any_element(),
            );
        }
        if pane.history {
            actions.push(
                div()
                    .id("back-to-plan")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(palette.panel_soft))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.selected_run = None;
                        view.selected_step = None;
                        view.leave_fields(window, cx);
                        cx.notify();
                    }))
                    .child("回到计划")
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_start()
            .gap_4()
            .flex_shrink_0()
            .px_5()
            .py_4()
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xl()
                                    .truncate()
                                    .text_color(rgb(palette.text))
                                    .child(pane.title.clone()),
                            )
                            .child(status_chip(&label, color, palette)),
                    )
                    .when(!pane.description.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .whitespace_normal()
                                .child(pane.description.clone()),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .truncate()
                            .child(pane.manifest_label.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .children(actions)
                    .child(
                        div()
                            .id("run")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(if running {
                                palette.panel_soft
                            } else {
                                palette.accent
                            }))
                            .border_1()
                            .border_color(rgb(if running {
                                palette.failed
                            } else {
                                palette.accent
                            }))
                            .text_sm()
                            .text_color(rgb(if running {
                                palette.failed
                            } else {
                                palette.accent_text
                            }))
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, window, cx| {
                                if view.active.is_some() {
                                    view.cancel_run(cx);
                                } else {
                                    view.start_run(cx);
                                }
                                view.leave_fields(window, cx);
                                cx.notify();
                            }))
                            .child(if running { "取消" } else { "运行" }),
                    ),
            )
    }

    /// Declared inputs: editable for a plan, read-only for a run.
    pub(crate) fn input_fields(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        if pane.history || pane.running {
            return pane
                .inputs
                .iter()
                .map(|(name, value)| {
                    div()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(rgb(palette.panel_soft))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{name} = {value}"))
                        .into_any_element()
                })
                .collect();
        }

        self.inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let slot = Field::Input(index);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w(px(240.))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(input.spec.name.clone()),
                            )
                            .when(input.spec.required, |this| {
                                this.child(div().text_xs().text_color(rgb(palette.warn)).child("*"))
                            })
                            .when(!input.spec.description.is_empty(), |this| {
                                this.child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(input.spec.description.clone()),
                                )
                            }),
                    )
                    .child(self.render_field(slot, &input.spec.placeholder(), palette, cx))
                    .into_any_element()
            })
            .collect()
    }
}

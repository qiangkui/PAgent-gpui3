//! The left column of the workflow page: a run shown the way a conversation is.
//!
//! The rows reuse the conversation's rendering (markdown, code blocks, pictures,
//! file cards, error blocks) but not its persona: a step header is a system row
//! — index, id, kind, status — and nothing here is editable or sent to a model.
//! The pane is read-only on purpose: editing lives on the step cards, and one
//! value has exactly one editable widget.

use super::*;
use super::run_error::RunErrorKind;

/// Message ids for transcript markdown start far above any chat index, so a
/// step's `<details>` toggle, code-block scroll handle and copy highlight can
/// never collide with a conversation message's.
const TRANSCRIPT_MESSAGE_BASE: usize = 2_000_000;

/// One level of indentation for everything a step produced.
const STEP_INDENT: f32 = 20.;

/// A further indent for an expanded output box under its label.
const EXPANDED_INDENT: f32 = 18.;

/// Text longer than this (or with more than one line) folds by default.
const FOLD_CHARS: usize = 40;

impl DshFlowApp {
    /// The text lane of the left column: the usual split, or the centred column
    /// once the history tab hides the board.
    fn transcript_content_width(&self) -> f32 {
        if self.flow_tab == FlowTab::History {
            let lane = self.flow_lane_width();
            (lane - 2. * self.history_gutter()).max(120.)
        } else {
            self.transcript_width
        }
    }

    /// The whole lower area, in logical pixels.
    fn flow_lane_width(&self) -> f32 {
        self.transcript_width + self.board_width + FLOW_TRANSCRIPT_PAD
    }

    /// History keeps the conversation's reading column: at most 900px, with the
    /// spare width split into equal margins, like the chat transcript.
    fn history_gutter(&self) -> f32 {
        ((self.flow_lane_width() - message::ANSWER_MAX_WIDTH) / 2.)
            .max(message::TRANSCRIPT_PAD)
            .round()
    }

    /// The left column: the run as a chat-like stream.
    ///
    /// One action is one row below a tab bar: "当前" shows the run in flight or the
    /// newest finished one, "历史" lists every saved run of this workflow. No card
    /// wraps a step; a step starts with one prominent label line.
    pub(crate) fn render_flow_transcript(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = self.transcript_content_width();
        let width = (content + FLOW_TRANSCRIPT_PAD).max(0.);
        let history = self.flow_tab == FlowTab::History;
        let history_list = self.flow_tab == FlowTab::History && self.history_open.is_none();
        let empty = self.flow_tab == FlowTab::Current && !pane.running && pane.outcome.is_none();
        let mut rows: Vec<AnyElement> = Vec::new();
        if history_list {
            rows.extend(self.render_history_list(pane, palette, cx));
        } else {
            if self.flow_tab == FlowTab::History {
                rows.push(self.render_history_back(palette, cx));
            }
            rows.extend(self.render_run_rows(pane, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .when(history, |this| this.flex_1().w_full())
            .when(!history, |this| this.w(px(width)).flex_shrink_0())
            .min_w(px(0.))
            .min_h(px(0.))
            .bg(rgb(palette.canvas))
            .when(empty, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .w_full()
                        .min_h(px(0.))
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .font_family("Microsoft YaHei")
                                .text_color(rgb(palette.faint))
                                .child("请点击运行开始"),
                        ),
                )
            })
            .when(!empty, |this| {
                this.child(
                    div()
                        .id("run-transcript")
                        .flex()
                        .flex_col()
                        .gap_1()
                        .flex_1()
                        .min_h(px(0.))
                        .min_w(px(0.))
                        .when(history, |this| this.px(px(self.history_gutter())))
                        .when(!history, |this| this.px_4())
                        .py_3()
                        .overflow_y_scroll()
                        .contained_wheel()
                        .track_scroll(&self.transcript_scroll)
                        .children(rows),
                )
            })
            .into_any_element()
    }

    /// The two tabs of the run transcript, drawn under the page title.
    pub(crate) fn render_flow_tabs(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let current = self.flow_tab == FlowTab::Current;
        let mut tabs = div()
            .flex()
            .flex_row()
            .items_end()
            .gap_3()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .px_5()
            .pt_1();
        for (label, active, slot) in [("当前", current, true), ("历史", !current, false)] {
            tabs = tabs.child(
                div()
                    .id(if slot {
                        "flow-tab-current"
                    } else {
                        "flow-tab-history"
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(24.))
                    .px_1()
                    .border_b_2()
                    .border_color(rgb(if active {
                        palette.muted
                    } else {
                        palette.chrome
                    }))
                    .text_xs()
                    .text_color(rgb(if active { palette.muted } else { palette.faint }))
                    .hover(move |style| style.text_color(rgb(palette.muted)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if slot {
                            view.show_current_run(cx);
                        } else {
                            view.show_run_history(cx);
                        }
                    }))
                    .child(label),
            );
        }
        tabs
    }

    /// Every saved run of this workflow, newest first.
    fn render_history_list(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let name = self
            .selected_entry()
            .map(|entry| entry.name.clone())
            .or_else(|| {
                self.selected_record()
                    .map(|record| record.workflow_name.clone())
            })
            .or_else(|| {
                self.active
                    .as_ref()
                    .map(|active| active.workflow_name.clone())
            })
            .unwrap_or_else(|| pane.title.clone());
        let indices = self.flow_history(&name);
        if indices.is_empty() {
            return vec![
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("还没有历史运行")
                    .into_any_element(),
            ];
        }
        indices
            .into_iter()
            .map(|index| {
                let record = &self.history[index];
                let (label, color) = if record.cancelled {
                    ("已取消".to_string(), palette.skipped)
                } else if record.outcome.ok {
                    ("成功".to_string(), palette.ok)
                } else {
                    ("失败".to_string(), palette.failed)
                };
                let failed = record.outcome.count("failed");
                let skipped = record.outcome.count("skipped");
                let inputs: String = record
                    .inputs
                    .iter()
                    .map(|(name, value)| format!("{name} = {value}"))
                    .collect::<Vec<_>>()
                    .join(" · ");
                div()
                    .id(("history-run", index))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .min_w(px(0.))
                    .px_3()
                    .py_2()
                    .mb_2()
                    .rounded_md()
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .shadow_sm()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.open_run_history(index, window, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(status_chip(&label, color, palette))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(ago(epoch_ms(record.finished))),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(format_duration(record.outcome.duration_ms)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(format!(
                                        "{} 步{}",
                                        record.outcome.steps.len(),
                                        if failed + skipped > 0 {
                                            format!("（{failed} 失败 / {skipped} 跳过）")
                                        } else {
                                            String::new()
                                        }
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(format!("run {}", record.outcome.run_id)),
                            ),
                    )
                    .when(!inputs.is_empty(), |this| {
                        this.child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(inputs),
                        )
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// The current run as rows; no rows when nothing ran yet.
    fn render_run_rows(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let ran = pane.running || pane.outcome.is_some();
        if !ran {
            if self.flow_tab == FlowTab::History {
                return vec![
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("还没有历史运行")
                        .into_any_element(),
                ];
            }
            return Vec::new();
        }
        let mut rows = vec![self.transcript_divider(
            &format!("开始运行 · {}", pane.title),
            "",
            palette.faint,
            palette,
        )];
        let dir = pane
            .manifest
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.chat_dir(self.active_chat));
        for (order, row) in pane.rows.iter().enumerate() {
            if row.status == "pending" && row.summary.is_empty() && row.text.is_none() {
                continue;
            }
            rows.push(self.render_transcript_step(order, row, &dir, palette, cx));
        }
        rows.push(self.transcript_run_end(pane, palette));
        rows
    }

    /// The "back to the list" row above an opened history record.
    fn render_history_back(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("history-back")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .pb_1()
            .text_xs()
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .on_click(cx.listener(|view, _, window, cx| {
                view.close_run_history(window, cx);
            }))
            .child("‹ 历史记录")
            .into_any_element()
    }

    /// One step of the run: a label line, then its output rows.
    ///
    /// The divider is the only emphasis a step gets; everything under it is a
    /// plain chat-like row, so a long run reads as a stream rather than a stack
    /// of cards.
    fn render_transcript_step(
        &self,
        order: usize,
        row: &StepRow,
        dir: &Path,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = status_color(&row.status, palette);
        let detail = if row.status == "running" {
            "运行中…".to_string()
        } else if row.status == "pending" {
            "待运行".to_string()
        } else if row.duration_ms > 0 {
            format!("{} · {}", row.status, format_duration(row.duration_ms))
        } else {
            row.status.clone()
        };
        let body = self.render_transcript_step_body(order, row, dir, palette, cx);
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_1()
            .pb_0()
            .child(self.transcript_divider(
                &format!("{}  {} · {}", order + 1, row.id, row.kind),
                &detail,
                color,
                palette,
            ))
            .when(!body.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .w_full()
                        .min_w(px(0.))
                        .pl(px(STEP_INDENT))
                        .children(body),
                )
            })
            .into_any_element()
    }

    /// A prominent label line: what step (or run) the rows below belong to.
    ///
    /// Deliberately no rules on either side — a dot and coloured text carry the
    /// emphasis, and the transcript keeps reading as one stream.
    fn transcript_divider(
        &self,
        label: &str,
        detail: &str,
        color: u32,
        palette: Palette,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .py_1()
            .child(status_dot(color))
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .text_color(rgb(color))
                    .child(label.to_string()),
            )
            .when(!detail.is_empty(), |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(detail.to_string()),
                )
            })
            .into_any_element()
    }

    /// The closing line of the run: verdict, time, and step count.
    fn transcript_run_end(&self, pane: &Pane<'_>, palette: Palette) -> AnyElement {
        let (label, color) = if pane.running {
            ("运行中…".to_string(), palette.running)
        } else if pane.cancelled {
            ("运行已取消".to_string(), palette.warn)
        } else if pane.outcome.is_some_and(|outcome| outcome.ok) {
            ("运行结束 · 成功".to_string(), palette.ok)
        } else {
            ("运行结束 · 失败".to_string(), palette.failed)
        };
        let detail = if let Some(outcome) = pane.outcome {
            let failed = outcome.count("failed");
            let skipped = outcome.count("skipped");
            format!(
                "{} · {} 步{} · run {}",
                format_duration(outcome.duration_ms),
                outcome.steps.len(),
                if failed + skipped > 0 {
                    format!("（{failed} 失败 / {skipped} 跳过）")
                } else {
                    String::new()
                },
                outcome.run_id
            )
        } else {
            format!(
                "{} · 共 {} 步",
                format_duration(pane.elapsed_ms),
                pane.rows.len()
            )
        };
        let mut block = div()
            .flex()
            .flex_col()
            .gap_2()
            .py_2()
            .child(self.transcript_divider(&label, &detail, color, palette));
        if let Some(error) = pane
            .outcome
            .and_then(|outcome| outcome.error.clone())
            .filter(|error| !error.trim().is_empty())
        {
            let kind = RunErrorKind::from_message(&error);
            block = block.child(detail_line("原因", &kind.prefixed(&error), palette));
            block = block.child(detail_line("建议", kind.hint(), palette));
        }
        block.into_any_element()
    }

    /// The content rows under one step's header, indented one level.
    fn render_transcript_step_body(
        &self,
        order: usize,
        row: &StepRow,
        dir: &Path,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let width = (self.transcript_content_width() - STEP_INDENT).max(120.);
        let mut rows: Vec<AnyElement> = Vec::new();
        if !row.summary.is_empty() {
            let color = status_color(&row.status, palette);
            rows.push(self.render_transcript_text(
                order,
                1,
                "结果",
                color,
                &row.summary,
                false,
                width,
                palette,
                cx,
            ));
        }
        if let Some(text) = row.text.as_deref().filter(|text| !text.trim().is_empty()) {
            rows.extend(self.render_transcript_markdown(order, text, dir, palette, cx));
        }
        if let Some(stdout) = row.stdout.as_deref().filter(|text| !text.trim().is_empty()) {
            rows.push(self.render_transcript_text(
                order,
                2,
                "输出",
                palette.sky,
                stdout,
                true,
                width,
                palette,
                cx,
            ));
        }
        if let Some(stderr) = row.stderr.as_deref().filter(|text| !text.trim().is_empty()) {
            rows.push(self.render_transcript_text(
                order,
                3,
                "错误输出",
                palette.failed,
                stderr,
                true,
                width,
                palette,
                cx,
            ));
        }
        if let Some(error) = &row.error {
            let text = error_text(error);
            if !text.is_empty() {
                rows.push(self.render_transcript_text(
                    order,
                    4,
                    "错误",
                    palette.failed,
                    &text,
                    false,
                    width,
                    palette,
                    cx,
                ));
            }
        }
        if !row.artifacts.is_empty() {
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .bg(rgb(palette.panel_soft))
                            .text_xs()
                            .text_color(rgb(palette.accent))
                            .child("产物"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(format!("{} 个", row.artifacts.len())),
                    )
                    .into_any_element(),
            );
            let mut cards: Vec<AnyElement> = Vec::new();
            for artifact in &row.artifacts {
                cards.push(self.render_transcript_artifact(
                    artifact,
                    (width - STEP_INDENT).max(120.),
                    palette,
                    cx,
                ));
            }
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pl(px(STEP_INDENT))
                    .w_full()
                    .min_w(px(0.))
                    .children(cards)
                    .into_any_element(),
            );
        }
        if row.status == "running" && row.stdout.is_none() && row.text.is_none() {
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .bg(rgb(palette.panel_soft))
                            .text_xs()
                            .text_color(rgb(palette.running))
                            .child("运行"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.running))
                            .child("运行中…"),
                    )
                    .into_any_element(),
            );
        }
        rows
    }

    /// One labelled outcome row, folded to a single preview line until asked for.
    ///
    /// This is the transcript's version of a chat tool row: a coloured badge,
    /// one line of the body, and an expand chip. The open state lives in the
    /// same per-transcript set as markdown `<details>`, under a section offset
    /// far above their block numbers.
    #[allow(clippy::too_many_arguments)]
    fn render_transcript_text(
        &self,
        order: usize,
        section: usize,
        label: &str,
        color: u32,
        body: &str,
        mono: bool,
        width: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = 10_000 + section;
        let expanded = self.transcript_details.contains(&(order, key));
        let foldable = body.lines().count() > 1 || body.chars().count() > FOLD_CHARS;
        let preview = body
            .lines()
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_string();
        let mut preview_text = div()
            .flex_1()
            .min_w(px(0.))
            .when(mono, |this| this.font_family(MONO_FONT))
            .text_xs()
            .text_color(rgb(if mono { palette.muted } else { palette.text }));
        preview_text = if foldable {
            preview_text.truncate()
        } else {
            preview_text.whitespace_normal()
        };
        let mut header = div()
            .id(("transcript-row", order * 16 + section))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(palette.panel_soft))
                    .text_xs()
                    .text_color(rgb(color))
                    .child(label.to_string()),
            )
            .child(preview_text.child(preview));
        if foldable {
            header = header
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.toggle_transcript_details(order, key, cx);
                }))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(if expanded { "▾ 收起" } else { "▸ 展开" }),
                );
        }

        let mut rows = div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .min_w(px(0.))
            .child(header);
        if expanded && foldable {
            let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> = Rc::new(|_, _, _| {});
            let box_width = (width - EXPANDED_INDENT).max(120.);
            let text_width = (box_width - 2. * message::DETAIL_PAD).max(80.);
            let text = SelectableText::new(
                SharedString::from(body.to_string()),
                &[],
                &[],
                rgb(palette.accent).into(),
                px(text_width),
                None,
                rgb(palette.selection).into(),
                handler,
            );
            rows =
                rows.child(
                    div()
                        .id(("transcript-text", order * 16 + section))
                        .ml(px(EXPANDED_INDENT))
                        .flex()
                        .flex_col()
                        .min_h(px(0.))
                        .w(px(box_width))
                        .max_h(px(message::DETAIL_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .contained_wheel()
                        .track_scroll(&self.block_scroll_handle((
                            TRANSCRIPT_MESSAGE_BASE + order,
                            900_000 + section,
                        )))
                        .px(px(message::DETAIL_PAD))
                        .py_1()
                        .rounded_md()
                        .bg(rgb(palette.background))
                        .border_1()
                        .border_color(rgb(palette.border))
                        .when(mono, |this| this.font_family(MONO_FONT))
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .child(text),
                );
        }
        rows.into_any_element()
    }

    /// The step's own answer, rendered by the conversation's markdown renderer.
    ///
    /// The handlers are the transcript's own: a copied code block, an opened
    /// `<details>` and a scroll handle are keyed by a message id far above the
    /// chat's, so the two panes never share state.
    fn render_transcript_markdown(
        &self,
        order: usize,
        text: &str,
        dir: &Path,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let message = TRANSCRIPT_MESSAGE_BASE + order;
        let on_drag: Rc<dyn Fn(usize, DragEvent, &mut Window, &mut App)> = Rc::new(|_, _, _, _| {});
        let view = cx.entity();
        let on_copy: Rc<dyn Fn(usize, SharedString, &mut Window, &mut App)> =
            Rc::new(move |block, body, _, cx| {
                view.update(cx, |app, cx| {
                    app.copy_code((message, block), body.to_string(), cx);
                });
            });
        let view = cx.entity();
        let on_toggle: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(move |block, _, cx| {
            view.update(cx, |app, cx| {
                app.toggle_transcript_details(message, block, cx);
            });
        });
        let on_check: Rc<dyn Fn(usize, &mut Window, &mut App)> = Rc::new(|_, _, _| {});
        let ticked = std::collections::HashSet::new();
        markdown_blocks(
            &self.parsed_blocks(message, text),
            text,
            palette,
            message,
            (self.transcript_content_width() - STEP_INDENT).max(120.),
            None,
            &on_drag,
            self.copied_code
                .filter(|(id, _)| *id == message)
                .map(|(_, block)| block),
            &on_copy,
            self.transcript_details(),
            &on_toggle,
            &ticked,
            &on_check,
            dir,
            &|key| self.block_scroll_handle(key),
            None,
        )
    }

    /// A picture or a file a step handed over; a picture is drawn, not just named.
    fn render_transcript_artifact(
        &self,
        artifact: &ArtifactView,
        width: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let target = PathBuf::from(&artifact.path);
        let key = path_key(&artifact.path);
        let caption = if artifact.bytes == 0 {
            artifact.name.clone()
        } else {
            format!("{}  {}", artifact.name, short_bytes(artifact.bytes))
        };
        let open = target.clone();
        let click = cx.listener(move |view, _, _, cx| {
            if let Err(error) = open_path(&open) {
                view.banner = Some(Banner::error(format!("打不开 {}：{error}", open.display())));
            }
            cx.notify();
        });
        if artifact.kind == "image" && target.is_file() {
            return div()
                .id(("transcript-image", key))
                .flex()
                .flex_col()
                .gap_1()
                .cursor_pointer()
                .on_click(click)
                .child(
                    gpui::img(target)
                        .max_w(px(width))
                        .max_h(px(message::IMAGE_MAX_HEIGHT))
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.border)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(caption),
                )
                .into_any_element();
        }
        let reveal_target = artifact.path.clone();
        let size = if artifact.bytes == 0 {
            String::new()
        } else {
            short_bytes(artifact.bytes)
        };
        div()
            .id(("transcript-file", key))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .pl_2()
            .pr_1()
            .py_1()
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel_soft)))
            .on_click(click)
            .child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .bg(rgb(palette.panel_soft))
                    .text_xs()
                    .text_color(rgb(palette.sky))
                    .child(message::file_badge(&artifact.name)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child(artifact.name.clone()),
            )
            .when(!size.is_empty(), |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(size),
                )
            })
            .child(
                div()
                    .id(("transcript-reveal", key))
                    .flex_shrink_0()
                    .px_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .hover(move |style| style.text_color(rgb(palette.text)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Err(error) = reveal_path(Path::new(&reveal_target)) {
                            view.banner = Some(Banner::error(format!("打不开所在文件夹：{error}")));
                        }
                        cx.notify();
                    }))
                    .child("所在文件夹"),
            )
            .into_any_element()
    }
}

/// A structured error as one foldable block; empty fields contribute nothing.
fn error_text(error: &ErrorView) -> String {
    let mut lines: Vec<String> = Vec::new();
    if error.kind.trim().is_empty() && error.message.trim().is_empty() {
        // No category can be inferred without either field; an explicit hint
        // still gets its own line.
        if let Some(hint) = error
            .hint
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            lines.push(format!("建议：{hint}"));
        }
        return lines.join("\n");
    }
    let category = RunErrorKind::classify(&error.kind, &error.message);
    let head = [category.label(), error.message.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !head.is_empty() {
        lines.push(head);
    }
    if let Some(location) = error
        .location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("位置：{location}"));
    }
    let expected = error.expected.as_deref().map(str::trim).unwrap_or_default();
    let gotten = error.gotten.as_deref().map(str::trim).unwrap_or_default();
    if !expected.is_empty() || !gotten.is_empty() {
        lines.push(format!("期望 / 实际：{expected} · {gotten}"));
    }
    if let Some(hint) = error
        .hint
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        lines.push(format!("建议：{hint}"));
    } else if !error.message.trim().is_empty() {
        lines.push(format!("建议：{}", category.hint()));
    }
    lines.join("\n")
}

/// A stable number for one artifact path, so its row keeps an element id of its
/// own without hashing the whole transcript.
fn path_key(path: &str) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in path.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash as usize
}
#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: &str, message: &str) -> ErrorView {
        ErrorView {
            kind: kind.to_string(),
            message: message.to_string(),
            ..ErrorView::default()
        }
    }

    #[test]
    fn an_empty_error_contributes_no_rows() {
        assert!(error_text(&error("", "")).is_empty());
        assert!(error_text(&error("   ", "  ")).is_empty());
    }

    #[test]
    fn an_error_keeps_only_the_parts_it_has() {
        assert_eq!(error_text(&error("assert", "")), "校验未通过");
        assert_eq!(
            error_text(&error("assert", "expected 3 rows")),
            "校验未通过 · expected 3 rows\n建议：看期望/实际和步骤输出，修正数据或筛选条件。"
        );
        let with_details = ErrorView {
            kind: "assert".to_string(),
            message: "wrong count".to_string(),
            hint: Some("check the filter".to_string()),
            location: Some("read:rows".to_string()),
            expected: Some("3".to_string()),
            gotten: Some("2".to_string()),
        };
        assert_eq!(
            error_text(&with_details),
            "校验未通过 · wrong count\n位置：read:rows\n期望 / 实际：3 · 2\n建议：check the filter"
        );
        let hint_only = ErrorView {
            hint: Some("retry".to_string()),
            ..ErrorView::default()
        };
        assert_eq!(error_text(&hint_only), "建议：retry");
    }
}

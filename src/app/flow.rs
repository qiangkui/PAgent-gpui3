//! The workflow view: a title with the current/history tabs, the run transcript,
//! and the static step board.
//!
//! The page is two columns: the left repeats the conversation's rendering for a
//! run's output (read-only, program-driven, one level indented under each step),
//! and the right holds one card per step, each carrying the inputs that step
//! reads and, when opened, its source. The bottom log pane is gone: stdout and
//! stderr are rows of the transcript.

use super::*;

impl DshFlowApp {
    /// The workflow pane: banner, header, history, transcript, board, status.
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    // The lower page is the left transcript's canvas; the board paints its
                    // own chrome on top. No line between them: colour is the seam.
                    // The board's top-left corner is the only rounded one, and it
                    // shows the canvas underneath.
                    .bg(rgb(palette.canvas))
                    .rounded_tl(px(PANEL_RADIUS))
                    .when(self.flow_tab == FlowTab::History, |this| {
                        this.rounded_tr(px(PANEL_RADIUS))
                    })
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_1()
                            .min_h(px(0.))
                            .min_w(px(0.))
                            .child(self.render_flow_transcript(&pane, palette, cx))
                            .when(self.flow_tab != FlowTab::History, |this| {
                                this.child(self.render_flow_board(&pane, palette, cx))
                            }),
                    ),
            )
    }

    /// Workflow identity and state, plus the folder button; run controls live on the board.
    pub(crate) fn render_flow_header(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let (label, color) = pane_status(pane, palette);
        let title = self
            .workflow_labels
            .get(&pane.title)
            .cloned()
            .unwrap_or_else(|| util::workflow_display_name(&pane.title, pane.placeholders()));
        let folder = pane.manifest.and_then(Path::parent).map(Path::to_path_buf);
        let mut actions = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0();
        if let Some(folder) = folder {
            actions = actions.child(
                div()
                    .id("open-workflow-folder")
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .size(px(24.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Err(error) = open_path(&folder) {
                            view.banner = Some(Banner::error(format!(
                                "打不开文件夹 {}：{error}",
                                folder.display()
                            )));
                        }
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path("icons/folder-open.svg")
                            .w(px(16.))
                            .h(px(16.))
                            .text_color(rgb(palette.faint)),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_5()
                    .w_full()
                    .min_w(px(0.))
                    .px_5()
                    .pt_3()
                    .pb_1()
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
                                            .px_1()
                                            .text_xl()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .truncate()
                                            .text_color(rgb(palette.text))
                                            .child(title),
                                    )
                                    .child(status_chip(&label, color, palette)),
                            )
                            .when(!pane.description.is_empty(), |this| {
                                this.child(
                                    div()
                                        .px_1()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .line_clamp(1)
                                        .child(pane.description.clone()),
                                )
                            }),
                    )
                    .child(actions),
            )
            .child(self.render_flow_tabs(palette, cx))
            .child(
                div()
                    .w_full()
                    .h(px(1. / self.scale_factor.max(1.)))
                    .bg(rgb(palette.chrome)),
            )
    }

    /// One input card: its name, the box, and (for secrets) the local-store tools.
    pub(crate) fn render_input_card(
        &self,
        index: usize,
        input: &InputField,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let slot = Field::Input(index);
        let secret = input.spec.secret;
        let saved = input.saved();
        let missing = input.spec.required && input.field.text.trim().is_empty();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(input.spec.name.clone()),
                    )
                    .when(input.spec.required, |this| {
                        this.child(div().text_xs().text_color(rgb(palette.warn)).child("*"))
                    })
                    .when(secret, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(palette.border))
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child("密钥"),
                        )
                    })
                    .when(saved, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(palette.panel_soft))
                                .text_xs()
                                .text_color(rgb(palette.ok))
                                .child("本机保存"),
                        )
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
            .when(secret, |this| {
                this.child(self.render_secret_tools(index, palette, cx))
            })
            .when(!input.uses.is_empty(), |this| {
                this.child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(format!(
                            "用于 {} 步 · {}",
                            input.uses.len(),
                            input.uses.join(", ")
                        )),
                )
            })
            .when(secret && input.uses.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child("清单里没有步骤读它"),
                )
            })
            .when(missing, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.warn))
                        .child("必填：填写后才能运行"),
                )
            })
            .when(secret && !missing && input.field.text.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .whitespace_normal()
                        .child("留空时，用到它的步骤会报 credential"),
                )
            })
            .into_any_element()
    }

    /// The buttons under one secret input: hold to show, save, forget.
    ///
    /// Only this row writes to the credential store. The value never travels on
    /// a command line: it reaches the engine on the child's stdin, exactly like
    /// a hand-typed password.
    fn render_secret_tools(
        &self,
        index: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let revealed = self.revealed_input == Some(index);
        let has_value = self
            .inputs
            .get(index)
            .is_some_and(|input| !input.field.text.is_empty());
        let stored = self
            .inputs
            .get(index)
            .is_some_and(|input| input.stored.is_some());
        let saved = self.inputs.get(index).is_some_and(InputField::saved);

        let mut row = div().flex().flex_row().flex_wrap().items_center().gap_2();

        row = row.child(
            div()
                .id(("reveal-input", index))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(rgb(if revealed {
                    palette.accent
                } else {
                    palette.border
                }))
                .when(revealed, |this| this.bg(rgb(palette.panel_soft)))
                .text_xs()
                .text_color(rgb(if revealed {
                    palette.accent
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        // The window's background handler drops the caret (and
                        // with it any revealed value) on every press that reaches
                        // it; holding this button is a press the field should
                        // survive.
                        cx.stop_propagation();
                        view.revealed_input = Some(index);
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        if view.revealed_input == Some(index) {
                            view.revealed_input = None;
                            cx.notify();
                        }
                    }),
                )
                // Released, or dragged off the button: either way the value goes
                // back behind dots. There is no toggle to forget about.
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        if view.revealed_input == Some(index) {
                            view.revealed_input = None;
                            cx.notify();
                        }
                    }),
                )
                // Pointer dragged off while still held: hide again, so the
                // value is only visible for as long as the button is under it.
                .on_mouse_exit(cx.listener(move |view, _, _, cx| {
                    if view.revealed_input == Some(index) {
                        view.revealed_input = None;
                        cx.notify();
                    }
                }))
                .child(if revealed {
                    "松开隐藏"
                } else {
                    "按住显示"
                }),
        );

        if has_value && !saved {
            row = row.child(
                div()
                    .id(("save-input", index))
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.sky))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.save_input_secret(index, cx);
                    }))
                    .child("保存到本机"),
            );
        }
        if stored {
            row = row.child(
                div()
                    .id(("forget-input", index))
                    .px_2()
                    .py_0p5()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.forget_input_secret(index, cx);
                    }))
                    .child("清除"),
            );
        }
        row.into_any_element()
    }

    /// Keep one secret input in this machine's credential store.
    ///
    /// The value in the box is what gets saved, and only when it is not empty:
    /// saving a blank would make the next launch look configured when it is
    /// not. The store's refusal (Linux, or a locked profile) is shown as a hint
    /// instead of a banner, because the run itself still works this session.
    pub(crate) fn save_input_secret(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workflow) = self.selected_entry().map(|entry| entry.name.clone()) else {
            return;
        };
        let Some(input) = self.inputs.get(index) else {
            return;
        };
        let name = input.spec.name.clone();
        if !input.spec.secret {
            return;
        }
        let value = input.field.text.clone();
        if value.is_empty() {
            self.show_toast("先填好这个值，再保存到本机", true, cx);
            return;
        }
        match crate::secrets::save_input(&workflow, &name, &value) {
            Ok(()) => {
                if let Some(input) = self.inputs.get_mut(index) {
                    input.stored = Some(value);
                }
                self.show_toast(format!("已把 {name} 保存到本机"), false, cx);
            }
            Err(error) => self.show_toast(format!("保存失败：{error}"), true, cx),
        }
    }

    /// Forget one stored secret and empty its box.
    ///
    /// The box is cleared as well: leaving the value there would make the next
    /// run keep using something the reader just asked to remove.
    pub(crate) fn forget_input_secret(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(workflow) = self.selected_entry().map(|entry| entry.name.clone()) else {
            return;
        };
        let Some(name) = self.inputs.get(index).map(|input| input.spec.name.clone()) else {
            return;
        };
        crate::secrets::forget_input(&workflow, &name);
        if let Some(input) = self.inputs.get_mut(index) {
            input.stored = None;
            input.field.clear();
        }
        self.revealed_input = None;
        self.show_toast(format!("已删除本机保存的 {name}"), false, cx);
    }
}

/// How many input cards fit side by side in `width` logical pixels.
///
/// The count is exact because the cards are a fixed width: `N` cards plus the
/// gaps `render_step_panel` draws need `N * 260 + (N - 1) * 12` pixels. A
/// window narrow enough for none still gets one, which is why the card is
/// allowed to be the only thing in its row.
/// @param width - the panel's content width, from `flow_input_width`.
/// @returns at least one column.
pub(crate) fn input_columns(width: f32) -> usize {
    if !width.is_finite() || width <= 0. {
        return 1;
    }
    let columns = ((width + INPUT_CARD_GAP) / (INPUT_CARD_WIDTH + INPUT_CARD_GAP)).floor();
    (columns as usize).max(1)
}

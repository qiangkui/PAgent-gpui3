//! The board of step cards on the right of the workflow page.
//!
//! This side is static: one card per step, in execution order, carrying the
//! step's own description, command, inputs, and — when opened — the manifest
//! lines and program file it runs. Runtime status, output and errors belong to
//! the run transcript on the left; the two panes do not share a selection.

use super::*;

impl DshFlowApp {
    /// The right column: run bar plus one card per step.
    pub(crate) fn render_flow_board(
        &self,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let editable = !pane.running && !pane.history;
        let mut cards: Vec<AnyElement> = Vec::new();
        let orphans: Vec<usize> = self
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.step.is_none())
            .map(|(index, _)| index)
            .collect();
        if editable && !orphans.is_empty() {
            cards.push(self.render_orphan_card(&orphans, palette, cx));
        }
        for (order, row) in pane.rows.iter().enumerate() {
            cards.push(self.render_board_card(order, row, pane, palette, cx));
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
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .bg(rgb(palette.chrome))
            .rounded_tl(px(PANEL_RADIUS))
            .child(self.render_board_bar(pane, palette, cx))
            .child(
                div()
                    .id("flow-board")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .min_h(px(0.))
                    .px_4()
                    .py_3()
                    .overflow_y_scroll()
                    .contained_wheel()
                    .track_scroll(&self.steps_scroll)
                    .children(cards),
            )
    }

    /// The board title plus the one run control the current state calls for.
    ///
    /// The bar never stacks all three controls: nothing running offers 运行, a run
    /// in flight offers 停止运行, and a finished run offers 新建运行 until the
    /// reader starts the next one.
    fn render_board_bar(&self, pane: &Pane<'_>, palette: Palette, cx: &mut Context<Self>) -> Div {
        let running = pane.running;
        let done = !running && pane.outcome.is_some();
        let (label, background, border, text) = if running {
            (
                "停止运行",
                palette.panel_soft,
                palette.failed,
                palette.failed,
            )
        } else if done {
            ("新建运行", palette.panel, palette.accent, palette.accent)
        } else {
            ("运行", palette.accent, palette.accent, palette.accent_text)
        };
        // A distilled draft is a skeleton: its `skip` placeholders do nothing, and
        // its captured commands may point at files from the chat's workspace. Say
        // so before the reader spends a run finding out.
        let draft = pane.placeholders() > 0;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .px_4()
            .py_2()
            .child(
                div()
                    .flex_shrink_0()
                    .text_base()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(palette.text))
                    .child("步骤"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .when(draft && !running, |this| {
                        this.truncate()
                            .text_xs()
                            .text_color(rgb(palette.warn))
                            .child("草稿：skip 占位不会执行，先把占位实现成步骤再运行")
                    }),
            )
            .child(
                div()
                    .id("board-run")
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(background))
                    .border_1()
                    .border_color(rgb(border))
                    .text_xs()
                    .text_color(rgb(text))
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, window, cx| {
                        if view.active.is_some() {
                            view.cancel_run(cx);
                        } else {
                            let done = view.pane().is_some_and(|pane| pane.outcome.is_some());
                            if done {
                                view.new_run(window, cx);
                            } else {
                                view.start_run(cx);
                            }
                        }
                        view.leave_fields(window, cx);
                        cx.notify();
                    }))
                    .child(label),
            )
    }

    /// One step card: what the step does, its inputs, and (opened) its source.
    ///
    /// This side is static on purpose. Status, timing, output and errors belong
    /// to the run transcript on the left; a card answers "what does this step
    /// do, and what does its code look like", not "what happened".
    fn render_board_card(
        &self,
        order: usize,
        row: &StepRow,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.selected_step.as_deref() == Some(row.id.as_str());
        let step_id = row.id.clone();
        let editable = !pane.running && !pane.history;
        let bound: Vec<usize> = self
            .inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.step.as_deref() == Some(row.id.as_str()))
            .map(|(index, _)| index)
            .collect();

        let mut fields: Vec<AnyElement> = Vec::new();
        if editable {
            for index in &bound {
                if let Some(input) = self.inputs.get(*index) {
                    fields.push(self.render_input_card(*index, input, palette, cx));
                }
            }
        } else {
            for index in &bound {
                let Some(input) = self.inputs.get(*index) else {
                    continue;
                };
                let value = pane
                    .inputs
                    .iter()
                    .find(|(name, _)| name == &input.spec.name)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_else(|| "—".to_string());
                fields.push(input_value_row(&input.spec.name, &value, palette));
            }
        }

        let header = div()
            .id(("flow-card", order))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, window, cx| {
                if view.selected_step.as_deref() == Some(step_id.as_str()) {
                    view.selected_step = None;
                } else {
                    view.selected_step = Some(step_id.clone());
                }
                view.leave_fields(window, cx);
                cx.notify();
            }))
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
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(if selected { "▾" } else { "▸" }),
            );

        let mut body: Vec<AnyElement> = Vec::new();
        if !row.description.is_empty() {
            body.push(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(row.description.clone())
                    .into_any_element(),
            );
        }
        body.extend(self.render_field_grid(fields));
        if selected {
            if !row.plan_summary.is_empty() {
                body.push(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .whitespace_normal()
                        .child(row.plan_summary.clone())
                        .into_any_element(),
                );
            }
            let mut meta: Vec<String> = Vec::new();
            if let Some(when) = &row.when {
                meta.push(format!("当 {when}"));
            }
            if let Some(save) = &row.save {
                meta.push(format!("保存 {save}"));
            }
            if !row.notes.is_empty() {
                meta.push(row.notes.join(" · "));
            }
            if !meta.is_empty() {
                body.push(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .whitespace_normal()
                        .child(meta.join(" · "))
                        .into_any_element(),
                );
            }
            body.extend(self.render_step_code(order, row, pane, palette, cx));
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(if selected {
                palette.sky
            } else {
                palette.border
            }))
            .child(header)
            .children(body)
            .into_any_element()
    }

    /// The opened card's code: the manifest lines for this step, then the
    /// program (or child manifest) it runs.
    fn render_step_code(
        &self,
        order: usize,
        row: &StepRow,
        pane: &Pane<'_>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(manifest) = pane.manifest else {
            return vec![muted_note("清单路径未知，打不开源码", palette)];
        };
        let dir = manifest.parent().unwrap_or_else(|| Path::new("."));
        let mut sections: Vec<AnyElement> = Vec::new();

        if let Ok(text) = std::fs::read_to_string(manifest) {
            let lines: Vec<&str> = text.lines().collect();
            if row.start_line > 0 && row.end_line >= row.start_line {
                let from = row.start_line.min(lines.len());
                let to = row.end_line.min(lines.len());
                if from >= 1 && to >= from {
                    let body = lines[from - 1..to].join("\n");
                    let label = format!(
                        "清单 · {}:{}-{}",
                        manifest
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("flow.yml"),
                        from,
                        to
                    );
                    sections.push(self.render_code_section(
                        order,
                        sections.len(),
                        &label,
                        "yaml",
                        &body,
                        palette,
                        cx,
                    ));
                }
            }
        }

        if let Some(program) = &row.program {
            let path = if Path::new(program).is_absolute() {
                PathBuf::from(program)
            } else {
                dir.join(program)
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let (body, cut) = clip_code(&text);
                    let label = format!("程序 · {program}");
                    sections.push(self.render_code_section(
                        order,
                        sections.len(),
                        &label,
                        code_language(program),
                        &body,
                        palette,
                        cx,
                    ));
                    if cut {
                        sections.push(muted_note(
                            &format!("只显示前 {CODE_MAX_LINES} 行"),
                            palette,
                        ));
                    }
                }
                Err(_) => sections.push(muted_note(
                    &format!("找不到程序文件：{}", path.display()),
                    palette,
                )),
            }
        }

        if let Some(child) = &row.child_manifest {
            let path = if Path::new(child).is_absolute() {
                PathBuf::from(child)
            } else {
                dir.join(child)
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let (body, cut) = clip_code(&text);
                    let label = format!("子流程 · {child}");
                    sections.push(self.render_code_section(
                        order,
                        sections.len(),
                        &label,
                        "yaml",
                        &body,
                        palette,
                        cx,
                    ));
                    if cut {
                        sections.push(muted_note(
                            &format!("只显示前 {CODE_MAX_LINES} 行"),
                            palette,
                        ));
                    }
                }
                Err(_) => sections.push(muted_note(
                    &format!("找不到子流程清单：{}", path.display()),
                    palette,
                )),
            }
        }

        if sections.is_empty() {
            sections.push(muted_note("清单没有记录这一步的源码位置", palette));
        }
        sections
    }

    /// One labelled, highlighted code block that wraps and scrolls vertically.
    fn render_code_section(
        &self,
        order: usize,
        section: usize,
        label: &str,
        language: &str,
        code: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let message = CODE_MESSAGE_BASE + order * 10 + section;
        let spans = crate::markdown::highlight(language, code);
        let tokens = TokenColors {
            comment: rgb(palette.faint).into(),
            string: rgb(palette.ok).into(),
            number: rgb(palette.warn).into(),
            keyword: rgb(palette.accent).into(),
            function: rgb(palette.code_function).into(),
            type_name: rgb(palette.code_type).into(),
            added: rgb(palette.ok).into(),
            removed: rgb(palette.failed).into(),
        };
        let body: SharedString = code.to_string().into();
        let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> = Rc::new(|_, _, _| {});
        let text = SelectableText::new(
            body.clone(),
            &spans,
            &[],
            rgb(palette.accent).into(),
            px((self.flow_input_width - 2. * CODE_PAD).max(120.)),
            None,
            rgb(palette.selection).into(),
            handler,
        )
        .with_tokens(tokens);
        let copied = self
            .copied_code
            .filter(|(id, _)| *id == message)
            .map(|(_, block)| block);
        let copy_source = body.clone();
        let scroll = self.block_scroll_handle((message, usize::MAX));
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .id(("board-code-copy", message))
                            .flex_shrink_0()
                            .px_1()
                            .rounded_sm()
                            .text_xs()
                            .text_color(rgb(if copied.is_some() {
                                palette.ok
                            } else {
                                palette.faint
                            }))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.border)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.copy_code((message, 0), copy_source.to_string(), cx);
                            }))
                            .child(if copied.is_some() {
                                "已复制"
                            } else {
                                "复制"
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .min_w(px(0.))
                    .max_h(px(CODE_VIEW_HEIGHT))
                    .child(
                        div()
                            .id(("board-code", message))
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w(px(0.))
                            .max_h(px(CODE_VIEW_HEIGHT))
                            .overflow_y_scroll()
                            .contained_wheel()
                            .track_scroll(&scroll)
                            .px(px(CODE_PAD))
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel_soft))
                            .font_family(MONO_FONT)
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(text),
                    )
                    .child(code_scroll_thumb(&scroll, palette, message, cx)),
            )
            .into_any_element()
    }

    /// The "运行前" card: inputs no step reads, kept visible instead of hidden.
    fn render_orphan_card(
        &self,
        indices: &[usize],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut fields: Vec<AnyElement> = Vec::new();
        for index in indices {
            if let Some(input) = self.inputs.get(*index) {
                fields.push(self.render_input_card(*index, input, palette, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
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
                            .child("运行前"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("清单没说这些输入属于哪一步"),
                    ),
            )
            .children(self.render_field_grid(fields))
            .into_any_element()
    }

    /// Lay fields out in rows that fit the board; two per row on a wide window.
    fn render_field_grid(&self, fields: Vec<AnyElement>) -> Vec<AnyElement> {
        if fields.is_empty() {
            return Vec::new();
        }
        let columns = input_columns(self.flow_input_width).max(1);
        // Never wider than what the board actually has, so a very narrow
        // window cannot clip a field.
        let field_width = self.flow_input_width.clamp(120., INPUT_CARD_WIDTH);
        let count = fields.len();
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut row: Vec<AnyElement> = Vec::new();
        for (index, field) in fields.into_iter().enumerate() {
            row.push(
                div()
                    .w(px(field_width))
                    .flex_shrink_0()
                    .child(field)
                    .into_any_element(),
            );
            if row.len() == columns || index + 1 == count {
                rows.push(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap_3()
                        .w_full()
                        .min_w(px(0.))
                        .children(std::mem::take(&mut row))
                        .into_any_element(),
                );
            }
        }
        rows
    }
}

/// Message ids for the board's code blocks start above the transcript's, so a
/// copy highlight or `<details>` toggle cannot collide with a run row.
const CODE_MESSAGE_BASE: usize = 3_000_000;

/// Lines of a program file shown at once.
pub(crate) const CODE_MAX_LINES: usize = 800;

/// Height past which a code block scrolls instead of growing.
pub(crate) const CODE_VIEW_HEIGHT: f32 = 420.;

/// A thin thumb down the right edge of a code box, from its scroll handle.
///
/// gpui has no scrollbar element, so the thumb is drawn from the handle's
/// numbers. Clicking the track jumps there; dragging reuses the window's
/// `tool_drag` state and the same mouse-move handler as the chat's code blocks.
fn code_scroll_thumb(
    scroll: &ScrollHandle,
    palette: Palette,
    message: usize,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    let Some((track, thumb, max, _origin)) = tool_bar_metrics(scroll) else {
        return div().into_any_element();
    };
    let travel = (track - thumb).max(px(1.));
    let progress = if max > px(0.) {
        (-scroll.offset().y / max).clamp(0., 1.)
    } else {
        0.
    };
    let handle = scroll.clone();
    div()
        .id(("board-code-scrollbar", message))
        .absolute()
        .right(px(0.))
        .top(px(0.))
        .bottom(px(0.))
        .w(px(8.))
        .block_mouse_except_scroll()
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                let Some((track, thumb, max, origin)) = tool_bar_metrics(&handle) else {
                    return;
                };
                let travel = (track - thumb).max(px(1.));
                let progress = if max > px(0.) {
                    (-handle.offset().y / max).clamp(0., 1.)
                } else {
                    0.
                };
                let thumb_top = origin + travel * progress;
                if !(event.position.y >= thumb_top && event.position.y <= thumb_top + thumb) {
                    let ratio = ((event.position.y - origin) / travel).clamp(0., 1.);
                    let current = handle.offset();
                    handle.set_offset(point(current.x, -max * ratio));
                }
                view.tool_drag = Some((message, event.position.y, handle.offset().y));
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .top(px(2.) + travel * progress)
                .right(px(1.))
                .w(px(4.))
                .h(thumb)
                .rounded_full()
                .bg(rgb(palette.faint)),
        )
        .into_any_element()
}

/// Cap a source file and say whether anything was cut.
pub(crate) fn clip_code(text: &str) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= CODE_MAX_LINES {
        (text.trim_end().to_string(), false)
    } else {
        (lines[..CODE_MAX_LINES].join("\n"), true)
    }
}

/// The fence language for a program path.
pub(crate) fn code_language(path: &str) -> &'static str {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "ps1" => "powershell",
        "sh" => "bash",
        "bat" | "cmd" => "batch",
        "rb" => "ruby",
        "yml" | "yaml" => "yaml",
        "json" => "json",
        "md" => "markdown",
        _ => "text",
    }
}

/// A small muted sentence inside a card.
fn muted_note(text: &str, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .text_color(rgb(palette.faint))
        .whitespace_normal()
        .child(text.to_string())
        .into_any_element()
}

/// A read-only value on a card in a finished run.
fn input_value_row(name: &str, value: &str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(palette.background))
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(name.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .text_color(rgb(palette.text))
                .child(value.to_string()),
        )
        .into_any_element()
}

//! Small shared widgets: buttons, frames, chips, and detail rows.

use super::*;

/// The drag handle between the sidebar and the pane.
///
/// Painted in the sidebar's own colour: the two read as one surface, and the
/// drag cursor is the only hint that it can be moved.
pub(crate) fn splitter(palette: Palette, cx: &mut Context<DshFlowApp>) -> impl IntoElement {
    div()
        .id("splitter")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .w(px(SPLITTER_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(palette.chrome))
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| {
                view.resizing_sidebar = true;
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

/// The error/warning strip under the title bar, if one is set.
///
/// Chat sets it for things like a file that could not be attached; the workflow
/// page has always shown it. Dropping it in the chat view left those failures
/// invisible, which is why this is shared rather than drawn twice.
impl DshFlowApp {
    pub(crate) fn render_banner(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let banner = self.banner.as_ref()?;
        let color = if banner.error {
            palette.failed
        } else {
            palette.warn
        };
        Some(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_3()
                .mx_5()
                .mt_3()
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(color))
                .bg(rgb(palette.panel))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_xs()
                        .text_color(rgb(color))
                        .whitespace_normal()
                        .child(banner.text.clone()),
                )
                .child(
                    div()
                        .id("dismiss-banner")
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.banner = None;
                            cx.notify();
                        }))
                        .child("关闭"),
                )
                .into_any_element(),
        )
    }
}
///
/// Windows sizes its own caption buttons at 46x32; matching that keeps the hit
/// area where the pointer expects it.
pub(crate) fn window_button(area: WindowControlArea, glyph: &'static str, palette: Palette) -> Div {
    let hover_background = if matches!(area, WindowControlArea::Close) {
        palette.failed
    } else {
        palette.panel_soft
    };

    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(46.))
        .h_full()
        .window_control_area(area)
        .hover(move |style| style.bg(rgb(hover_background)))
        .text_color(rgb(palette.text))
        .child(div().text_size(px(19.)).child(glyph))
}

/// A 20px square icon button, used for the page actions beside a heading.
pub(crate) fn icon_button(
    id: &'static str,
    icon: &'static str,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(20.))
        .rounded_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| run(view, cx)))
        .child(
            svg()
                .path(icon)
                .w(px(13.))
                .h(px(13.))
                .text_color(rgb(palette.muted)),
        )
        .into_any_element()
}

/// A titled block on the settings page.
///
/// Every settings page is a column of these: a bordered card on the panel
/// colour, one bold heading, an optional line of explanation, then the controls.
/// The nav on the left and the cards on the right share one rhythm, so a page of
/// two blocks and a page of five read the same way.
pub(crate) fn settings_section(title: &'static str, palette: Palette) -> Div {
    settings_card(title, palette)
}

/// The card itself: the one surface every settings block sits on.
pub(crate) fn settings_card(title: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.canvas))
        .shadow_sm()
        .p_5()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(title.to_string()),
        )
}

/// The heading of a whole page: a title, then one line about the page.
pub(crate) fn settings_page_header(title: &str, note: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(title.to_string()),
        )
        .child(settings_note(note, palette))
}

/// One line of explanation: the muted voice under a heading.
pub(crate) fn settings_note(note: &str, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .line_height(px(17.))
        .text_color(rgb(palette.muted))
        .whitespace_normal()
        .child(note.to_string())
        .into_any_element()
}

/// A path, a command or a file name on a soft chip, so it reads as a value.
pub(crate) fn settings_code(text: String, palette: Palette) -> AnyElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(rgb(palette.panel_soft))
        .text_xs()
        .text_color(rgb(palette.text))
        .child(text)
        .into_any_element()
}

/// How a value on a row should read: ordinary, good, a caution, or broken.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTone {
    /// Ordinary text.
    Normal,
    /// Something that works.
    Good,
    /// Something to watch.
    Warn,
    /// Something that does not work.
    Bad,
}

impl SettingsTone {
    /// The colour this tone paints.
    fn color(self, palette: Palette) -> u32 {
        match self {
            Self::Normal => palette.text,
            Self::Good => palette.ok,
            Self::Warn => palette.warn,
            Self::Bad => palette.failed,
        }
    }
}

/// One label/value row: a fixed label column, then the value.
pub(crate) fn settings_row(label: &str, value: AnyElement, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(124.))
                .flex_shrink_0()
                .text_xs()
                .line_height(px(17.))
                .text_color(rgb(palette.muted))
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w(px(0.)).child(value))
        .into_any_element()
}

/// A row whose value is plain text in the given tone.
pub(crate) fn settings_text_row(
    label: &str,
    value: String,
    tone: SettingsTone,
    palette: Palette,
) -> AnyElement {
    let text = div()
        .text_xs()
        .line_height(px(17.))
        .whitespace_normal()
        .text_color(rgb(tone.color(palette)))
        .child(value)
        .into_any_element();
    settings_row(label, text, palette)
}

/// The look of a settings button; the caller adds the action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsButton {
    /// The one action a card is about.
    Primary,
    /// Anything else on the card.
    Secondary,
    /// Removing or discarding something.
    Danger,
}

/// A settings button, sized and coloured like every other one.
pub(crate) fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    kind: SettingsButton,
    palette: Palette,
) -> gpui::Stateful<Div> {
    let (background, border, ink) = match kind {
        SettingsButton::Primary => (palette.accent, palette.accent, palette.accent_text),
        SettingsButton::Secondary => (palette.panel_soft, palette.border, palette.text),
        SettingsButton::Danger => (palette.panel_soft, palette.failed, palette.failed),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .text_xs()
        .text_color(rgb(ink))
        .cursor_pointer()
        .hover(move |style| style.opacity(0.92))
        .child(label.to_string())
}

/// A small pill saying whether something is working.
pub(crate) fn settings_status_pill(ok: bool, label: &str, palette: Palette) -> AnyElement {
    let colour = if ok { palette.ok } else { palette.failed };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(rgb(palette.panel_soft))
        .child(status_dot(colour))
        .child(
            div()
                .text_xs()
                .text_color(rgb(colour))
                .child(label.to_string()),
        )
        .into_any_element()
}

/// A short result line under an action: what just happened.
pub(crate) fn settings_notice(text: &str, failed: bool, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .line_height(px(17.))
        .whitespace_normal()
        .text_color(rgb(if failed { palette.failed } else { palette.ok }))
        .child(text.to_string())
        .into_any_element()
}

/// A page title with its controls just to the right of it.
///
/// The plugins, MCP and Skill pages are read as flat lists: the title is the
/// only heading, its actions follow it after a gap (not pinned to the far
/// edge), and there is no line of explanation beneath it.
pub(crate) fn page_title(title: &'static str, actions: Vec<AnyElement>, palette: Palette) -> Div {
    let has_actions = !actions.is_empty();
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(20.))
        .flex_shrink_0()
        .px_6()
        .py_4()
        .child(div().text_xl().text_color(rgb(palette.text)).child(title));
    if has_actions {
        row = row.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .children(actions),
        );
    }
    row
}

/// Merge the declared plan with the engine's results, keeping plan order.
pub(crate) fn build_rows(spec: &ManifestSpec, results: &[StepView]) -> Vec<StepRow> {
    let mut rows: Vec<StepRow> = spec
        .steps
        .iter()
        .map(|step| {
            let result = results.iter().find(|view| view.id == step.id);
            StepRow {
                id: step.id.clone(),
                kind: step.kind.label().to_string(),
                summary: step.summary.clone(),
                when: step.when.clone(),
                save: step.save.clone(),
                notes: step.notes.clone(),
                status: result.map_or_else(|| "pending".to_string(), |view| view.status.clone()),
                duration_ms: result.map_or(0, |view| view.duration_ms),
                detail: result.map_or_else(String::new, |view| view.detail.clone()),
                command: result.and_then(|view| view.command.clone()),
                stderr: result.and_then(|view| view.stderr.clone()),
                stdout: result.and_then(|view| view.stdout.clone()),
                text: result.and_then(|view| view.text.clone()),
                model: result.and_then(|view| view.model.clone()),
                cached: result.is_some_and(|view| view.cached),
            }
        })
        .collect();

    for result in results {
        if spec.steps.iter().any(|step| step.id == result.id) {
            continue;
        }
        rows.push(StepRow {
            id: result.id.clone(),
            kind: result.kind.clone(),
            status: result.status.clone(),
            duration_ms: result.duration_ms,
            detail: result.detail.clone(),
            command: result.command.clone(),
            stderr: result.stderr.clone(),
            stdout: result.stdout.clone(),
            text: result.text.clone(),
            model: result.model.clone(),
            cached: result.cached,
            ..Default::default()
        });
    }
    rows
}

/// Label and colour for the pane's status chip.
pub(crate) fn pane_status(pane: &Pane<'_>, palette: Palette) -> (String, u32) {
    if pane.running {
        return (
            format!("运行中 {}", format_duration(pane.elapsed_ms)),
            palette.running,
        );
    }
    if pane.cancelled {
        return ("已取消".to_string(), palette.warn);
    }
    match pane.outcome {
        Some(outcome) if outcome.ok => ("成功".to_string(), palette.ok),
        Some(_) => ("失败".to_string(), palette.failed),
        None => ("计划".to_string(), palette.faint),
    }
}

/// Colour of a step status.
pub(crate) fn status_color(status: &str, palette: Palette) -> u32 {
    match status {
        "ok" => palette.ok,
        "failed" => palette.failed,
        "skipped" => palette.skipped,
        "running" => palette.running,
        "pending" | "unknown" => palette.faint,
        _ => palette.muted,
    }
}

/// A small filled circle.
pub(crate) fn status_dot(color: u32) -> Div {
    div().w(px(7.)).h(px(7.)).rounded_full().bg(rgb(color))
}

/// A pill with a dot and a label.
pub(crate) fn status_chip(label: &str, color: u32, palette: Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .rounded_full()
        .bg(rgb(palette.panel_soft))
        .child(status_dot(color))
        .child(
            div()
                .text_xs()
                .text_color(rgb(color))
                .child(label.to_string()),
        )
}

/// The colour grid under one role: six rows of hues, a row of greys, and the
/// theme's own colour.
///
/// Picking a tile applies it and closes the grid; the last tile puts the role
/// back on the theme's colour, which is the same as never having changed it.
pub(crate) fn color_grid(
    key: &'static str,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    for (row_index, row) in crate::theme::swatch_grid().into_iter().enumerate() {
        let mut tiles: Vec<AnyElement> = Vec::new();
        for (column, value) in row.into_iter().enumerate() {
            let id = row_index * 16 + column;
            tiles.push(
                div()
                    .id(("swatch", id))
                    .w(px(18.))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(value))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.apply_color(key, value);
                        view.sync_color_fields(cx);
                        view.color_menu = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .flex()
                .flex_row()
                .gap_1()
                .children(tiles)
                .into_any_element(),
        );
    }
    rows.push(
        div()
            .id("swatch-theme")
            .mt_1()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(palette.panel))
            .text_xs()
            .text_color(rgb(palette.muted))
            .cursor_pointer()
            .child("用主题原色")
            .on_click(cx.listener(move |view, _, _, cx| {
                view.reset_color(key);
                view.sync_color_fields(cx);
                view.color_menu = None;
                cx.notify();
            }))
            .into_any_element(),
    );

    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.panel_soft))
        .children(rows)
        .into_any_element()
}

/// One palette in the theme picker: a colour preview, a name, and a badge when
/// it is the one in use.
///
/// The preview is painted with the palette the card ships, so the cards show the
/// choice rather than the current state; the card chrome follows the live theme.
pub(crate) fn theme_card(
    kind: ThemeKind,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> impl IntoElement {
    let shown = kind.palette();

    div()
        .id(kind.label())
        .flex()
        .flex_col()
        .gap_2()
        .w(px(150.))
        .p_2()
        .rounded_lg()
        .bg(rgb(palette.panel))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .cursor_pointer()
        .on_click(cx.listener(move |view, _, _, cx| {
            view.set_theme(kind, cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .h(px(56.))
                .px_3()
                .rounded_md()
                .bg(rgb(shown.background))
                .border_1()
                .border_color(rgb(shown.border))
                .child(
                    div()
                        .w(px(26.))
                        .h(px(26.))
                        .rounded_md()
                        .bg(rgb(shown.panel)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .w(px(56.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.text)),
                        )
                        .child(
                            div()
                                .w(px(38.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.muted)),
                        ),
                )
                .child(
                    div()
                        .w(px(10.))
                        .h(px(10.))
                        .rounded_full()
                        .bg(rgb(shown.accent)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(kind.label()),
                )
                .when(selected, |this| {
                    this.child(status_chip("当前", palette.accent, palette))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(kind.description()),
        )
}

/// The picker's third card: the reader's own colours.
///
/// It is a state, not a choice: the window becomes custom the moment one colour
/// differs from the theme, so the card only reports that. Its preview is the
/// live palette, which is the only thing it could honestly show.
pub(crate) fn custom_theme_card(
    shown: &Palette,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> impl IntoElement {
    div()
        .id("theme-custom")
        .flex()
        .flex_col()
        .gap_2()
        .w(px(150.))
        .p_2()
        .rounded_lg()
        .bg(rgb(palette.panel))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .h(px(56.))
                .px_3()
                .rounded_md()
                .bg(rgb(shown.background))
                .border_1()
                .border_color(rgb(shown.border))
                .child(
                    div()
                        .w(px(26.))
                        .h(px(26.))
                        .rounded_md()
                        .bg(rgb(shown.panel)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .w(px(56.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.text)),
                        )
                        .child(
                            div()
                                .w(px(38.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.muted)),
                        ),
                )
                .child(
                    div()
                        .w(px(10.))
                        .h(px(10.))
                        .rounded_full()
                        .bg(rgb(shown.accent)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(if selected {
                            palette.text
                        } else {
                            palette.muted
                        }))
                        .child(crate::theme::CUSTOM_LABEL),
                )
                .when(selected, |this| {
                    this.child(status_chip("当前", palette.accent, palette))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .whitespace_normal()
                .child(crate::theme::CUSTOM_DESCRIPTION),
        )
        // The card reports the mode; the switches that change it are the theme
        // cards above and the colour rows below.
        .when(!selected, |this| this.cursor(CursorStyle::Arrow))
        .when(!selected, |this| this.opacity(0.75))
        .on_click(cx.listener(|_, _, _, cx| {
            // Clicking it is harmless: it only means "stay as I am".
            cx.stop_propagation();
        }))
}

/// A label/value line inside a step detail.
pub(crate) fn detail_line(label: &str, value: &str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(value.to_string()),
        )
        .into_any_element()
}

/// A captured stream inside a step detail.
pub(crate) fn detail_block(label: &str, body: &str, palette: Palette) -> AnyElement {
    let shown: Vec<&str> = body.lines().take(DETAIL_LINES).collect();
    let truncated = body.lines().count() > shown.len();
    let mut text = shown.join("\n");
    if truncated {
        text.push_str("\n…");
    }
    div()
        .flex()
        .flex_col()
        .min_w(px(0.))
        .gap_1()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(palette.field))
        .border_1()
        .border_color(rgb(palette.border))
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.text))
                .whitespace_normal()
                .child(text),
        )
        .into_any_element()
}

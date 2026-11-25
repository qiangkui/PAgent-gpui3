//! The cards under the request list: timing, actions, panels.

use super::*;

/// One row of the inline round list: the whole request (`round` empty) or one
/// round of it.
///
/// The list replaced a popup: the rounds are a handful of short rows, and having
/// them under the request's heading means picking one is a single press.
pub(crate) fn context_round_row(
    id: usize,
    label: String,
    note: String,
    selected: bool,
    round: Option<u32>,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    div()
        .id(("context-round-row", id))
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .px_2()
        .py_1()
        .rounded_sm()
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .bg(rgb(if selected {
            palette.panel_soft
        } else {
            palette.canvas
        }))
        .text_xs()
        .text_color(rgb(if selected {
            palette.accent
        } else {
            palette.text
        }))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| {
            view.context_round = round;
            view.context_item = None;
            view.context_item_scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        }))
        .child(label)
        .child(div().flex_1().min_w(px(0.)))
        .child(
            div()
                .flex_shrink_0()
                .text_color(rgb(palette.muted))
                .child(note),
        )
        .into_any_element()
}

/// The picked request's wall clock split: a three-segment bar, then the rows.
///
/// Shown only when the engine reported timing — an answer saved before this was
/// recorded has nothing to split, and an empty bar would read as "no time".
pub(crate) fn render_timing_card(turn: u64, timing: RequestTiming, palette: Palette) -> AnyElement {
    let rows = timing_rows(timing, palette);
    let total = timing.total_ms.max(1);
    let mut bar = div()
        .flex()
        .flex_row()
        .w_full()
        .min_w(px(0.))
        .h(px(8.))
        .overflow_hidden()
        .rounded_full()
        .bg(rgb(palette.panel_soft));
    for (_, ms, colour) in rows {
        if ms == 0 {
            continue;
        }
        let share = (ms as f64 / total as f64).clamp(0., 1.) as f32;
        bar = bar.child(div().h_full().w(relative(share)).bg(rgb(colour)));
    }
    let mut legend = div().flex().flex_col().gap_1().w_full().min_w(px(0.));
    for (label, ms, colour) in rows {
        let share = ms as f64 / total as f64;
        legend = legend.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .size(px(7.))
                        .rounded_sm()
                        .bg(rgb(if ms > 0 { colour } else { palette.border })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(rgb(if ms > 0 { palette.text } else { palette.faint }))
                        .child(label),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(64.))
                        .text_right()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format_duration(ms)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(48.))
                        .text_right()
                        .text_xs()
                        .text_color(rgb(if ms > 0 { palette.muted } else { palette.faint }))
                        .child(share_text(true, share)),
                ),
        );
    }
    let mut card = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .rounded_lg()
        .bg(rgb(palette.canvas))
        .border_1()
        .border_color(rgb(palette.border))
        .shadow_md()
        .p_4()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.muted))
                        .child("计时"),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(format!(
                            "第{turn} 次请求· {}",
                            format_duration(timing.total_ms)
                        )),
                ),
        )
        .child(bar)
        .child(legend);
    if timing.other_ms > 0 {
        // The two measured buckets name themselves; the third is the one that
        // needs saying, so nobody reads it as a bug.
        card = card.child(
            div()
                .w_full()
                .min_w(px(0.))
                .whitespace_normal()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child("其他 = 压缩上下文、等待审批、拼装提示词与进程开销"),
        );
    }
    card.into_any_element()
}

/// The picked request against the model window: the categories it sent, the
/// room left before the engine compacts, and the output the connection reserved.
///
/// The bar is scaled to the whole window, so the empty track at the end is the
/// room that is genuinely unused; the two masks name the parts that are spoken
/// for even though no text arrives in them.
pub(crate) fn render_window_usage(
    categories: &[(u32, f32)],
    usage: WindowUsage,
    filled_tokens: u64,
    window: u64,
    output: u64,
    palette: Palette,
) -> AnyElement {
    let mut bar = div()
        .relative()
        .flex()
        .flex_row()
        .w_full()
        .min_w(px(0.))
        .h(px(10.))
        .overflow_hidden()
        .rounded_sm()
        .bg(rgb(palette.panel_soft));
    for (colour, share) in categories {
        if *share <= 0. {
            continue;
        }
        bar = bar.child(
            div()
                .flex_shrink_0()
                .h_full()
                .w(relative(*share))
                .bg(rgb(*colour)),
        );
    }
    if usage.headroom > 0. {
        bar = bar.child(
            div()
                .flex_shrink_0()
                .h_full()
                .w(relative(usage.headroom))
                .bg(rgb(palette.border)),
        );
    }
    // Pinned to the right edge: the answer's room is the last slice of the
    // window, and drawing it last lets it cover the headroom where they meet.
    if usage.reserve > 0. {
        bar = bar.child(
            div()
                .absolute()
                .top(px(0.))
                .right(px(0.))
                .h_full()
                .w(relative(usage.reserve))
                .bg(rgb(palette.skipped))
                .opacity(0.35),
        );
    }
    // The compaction line, only while the filled part has not reached it.
    if usage.filled < usage.pressure {
        bar = bar.child(
            div()
                .absolute()
                .top(px(0.))
                .left(relative(usage.pressure))
                .w(px(1.))
                .h_full()
                .bg(rgb(palette.warn))
                .opacity(0.7),
        );
    }

    let dot = |colour: u32| {
        div()
            .flex_shrink_0()
            .size(px(7.))
            .rounded_sm()
            .bg(rgb(colour))
    };
    let label = |text: String, colour: u32| {
        div()
            .min_w(px(0.))
            .truncate()
            .text_xs()
            .text_color(rgb(colour))
            .child(text)
    };
    let pressure_tokens = (window as f64 * f64::from(WINDOW_PRESSURE)).round() as u64;
    let headroom = pressure_tokens.saturating_sub(filled_tokens);
    let mut note = div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .child(dot(palette.accent))
        .child(label(
            format!(
                "已用 {} / {}（{}）",
                short_count(filled_tokens),
                short_count(window),
                percent_label(usage.filled as f64 * 100.)
            ),
            palette.muted,
        ));
    if headroom > 0 {
        note = note.child(dot(palette.border)).child(label(
            format!("压缩预留 {}", short_count(headroom)),
            palette.muted,
        ));
    }
    if output > 0 {
        note = note.child(dot(palette.skipped)).child(label(
            format!("输出预留 {}", short_count(output)),
            palette.muted,
        ));
    }
    if usage.filled >= usage.pressure {
        note = note.child(label("已过 80% 压缩线".to_string(), palette.warn));
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .min_w(px(0.))
        .child(bar)
        .child(note)
        .into_any_element()
}

/// The card listing what the selected request did to files and the machine.
pub(crate) fn render_actions_card(
    turn: u64,
    actions: &[RequestAction],
    palette: Palette,
) -> AnyElement {
    let kinds = [
        ActionKind::File,
        ActionKind::Command,
        ActionKind::Network,
        ActionKind::Mcp,
        ActionKind::Plugin,
        ActionKind::Other,
    ];
    let count = |kind: ActionKind| actions.iter().filter(|action| action.kind == kind).count();
    let failed = actions.iter().filter(|action| !action.ok).count();
    let mut chips = div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap_2()
        .w_full()
        .min_w(px(0.));
    for kind in kinds {
        let total = count(kind);
        if total == 0 {
            continue;
        }
        chips = chips.child(action_pill(kind.label(), total, palette.text, palette));
    }
    if failed > 0 {
        chips = chips.child(action_pill("失败", failed, palette.failed, palette));
    }
    let mut card = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .rounded_lg()
        .bg(rgb(palette.canvas))
        .border_1()
        .border_color(rgb(palette.border))
        .shadow_md()
        .p_4()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.muted))
                        .child("动作"),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(format!("第{turn} 次请求· {} 次工具调用", actions.len())),
                ),
        )
        .child(chips);
    let files: Vec<&RequestAction> = actions
        .iter()
        .filter(|action| action.kind == ActionKind::File)
        .collect();
    if !files.is_empty() {
        card = card.child(action_section("文件活动", &files, palette));
    }
    let commands: Vec<&RequestAction> = actions
        .iter()
        .filter(|action| action.kind == ActionKind::Command)
        .collect();
    if !commands.is_empty() {
        card = card.child(action_section("命令与代码", &commands, palette));
    }
    card.into_any_element()
}

/// A chip in the actions card: a group name and how many calls it had.
pub(crate) fn action_pill(label: &str, count: usize, colour: u32, palette: Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .px_2()
        .py_1()
        .rounded_sm()
        .bg(rgb(palette.panel_soft))
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(colour))
                .child(count.to_string()),
        )
}

/// One titled list of actions, in order, capped so the card cannot grow.
pub(crate) fn action_section(title: &str, actions: &[&RequestAction], palette: Palette) -> Div {
    let mut list = div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .min_w(px(0.))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.muted))
                .child(title.to_string()),
        );
    for action in actions.iter().take(ACTION_ROWS) {
        list = list.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(if action.ok {
                            palette.ok
                        } else {
                            palette.failed
                        }))
                        .child(if action.ok { "✓" } else { "✗" }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .child(action.detail.clone()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(action.tool.clone()),
                ),
        );
    }
    if actions.len() > ACTION_ROWS {
        list = list.child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(format!("还有 {} 项", actions.len() - ACTION_ROWS)),
        );
    }
    list
}

/// How many lines of a text row a panel card shows before it clamps.
///
/// The engine allows 4000 characters per entry; a card that grew to that size
/// would push the breakdown off the page, so the row is capped instead.
pub(crate) const PANEL_TEXT_LINES: usize = 6;

/// The card holding every panel the enabled plugins built for a request.
///
/// The window draws them in its own style — the engine only supplies numbers,
/// short text and the shape (text rows, bars or a table), never layout.
pub(crate) fn render_panels_card(
    panels: &[crate::ai::PanelReport],
    palette: Palette,
    asking: bool,
) -> AnyElement {
    let mut card = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap_3()
        .w_full()
        .min_w(px(0.))
        .rounded_lg()
        .bg(rgb(palette.canvas))
        .border_1()
        .border_color(rgb(palette.border))
        .shadow_md()
        .p_4()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.muted))
                        .child("插件面板"),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(if asking {
                            "由插件生成· 本次提问进行中"
                        } else {
                            "由插件生成· 随这次提问重建"
                        }),
                ),
        );
    for panel in panels {
        card = card.child(render_panel_block(panel, palette));
    }
    card.into_any_element()
}

/// One panel: its heading, the headline figures, then its sections.
pub(crate) fn render_panel_block(panel: &crate::ai::PanelReport, palette: Palette) -> Div {
    let origin = if panel.plugin.is_empty() {
        panel.id.clone()
    } else {
        format!("{} · {}", panel.plugin, panel.id)
    };
    let mut block = div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.panel_soft))
        .p_3()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(panel.title.clone()),
                )
                .child(div().flex_1().min_w(px(0.)))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(origin),
                ),
        );
    if !panel.summary.is_empty() {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap_2()
            .w_full()
            .min_w(px(0.));
        for (label, value) in &panel.summary {
            row = row.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(palette.canvas))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(label.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(value.clone()),
                    ),
            );
        }
        block = block.child(row);
    }
    for section in &panel.sections {
        block = block.child(render_panel_section(section, palette));
    }
    block
}

/// One section of a panel, in whichever of the three shapes it asked for.
pub(crate) fn render_panel_section(section: &crate::ai::PanelSection, palette: Palette) -> Div {
    let mut block = div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .min_w(px(0.))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.muted))
                .child(section.title.clone()),
        );
    match section.kind {
        crate::ai::PanelKind::Text => {
            for item in &section.items {
                let body = if item.text.is_empty() {
                    item.note.clone()
                } else {
                    item.text.clone()
                };
                block = block.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .w_full()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(item.label.clone()),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .whitespace_normal()
                                .line_clamp(PANEL_TEXT_LINES)
                                .child(body),
                        ),
                );
            }
        }
        crate::ai::PanelKind::Bars => {
            let largest = section
                .items
                .iter()
                .filter_map(|item| item.value)
                .fold(0.0_f64, f64::max);
            for item in &section.items {
                let share = if largest > 0.0 {
                    (item.value.unwrap_or(0.0) / largest).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                let printed = match item.value {
                    Some(value) if value.fract() == 0.0 => format!("{value:.0}"),
                    Some(value) => format!("{value}"),
                    None if !item.text.is_empty() => item.text.clone(),
                    None => String::new(),
                };
                let mut head = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(item.label.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(printed),
                    );
                if !item.note.is_empty() {
                    head = head.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(item.note.clone()),
                    );
                }
                block = block.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .w_full()
                        .min_w(px(0.))
                        .child(head)
                        .child(
                            div()
                                .h(px(3.))
                                .w_full()
                                .min_w(px(0.))
                                .rounded_full()
                                .overflow_hidden()
                                .bg(rgb(palette.canvas))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(share))
                                        .rounded_full()
                                        .bg(rgb(palette.accent)),
                                ),
                        ),
                );
            }
        }
        crate::ai::PanelKind::Table => {
            block = block.child(render_panel_table(section, palette));
        }
    }
    block
}

/// The grid of a `table` section: column headings, then one line per row.
pub(crate) fn render_panel_table(section: &crate::ai::PanelSection, palette: Palette) -> Div {
    let columns = section
        .columns
        .len()
        .max(section.rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut grid = div().flex().flex_col().gap_0p5().w_full().min_w(px(0.));
    if columns == 0 {
        // No grid to lay out: keep whatever text came along visible.
        for item in &section.items {
            grid = grid.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(if item.text.is_empty() {
                        item.label.clone()
                    } else {
                        format!("{}：{}", item.label, item.text)
                    }),
            );
        }
        return grid;
    }
    let mut head = div().flex().flex_row().gap_2().w_full().min_w(px(0.));
    for index in 0..columns {
        head = head.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(section.columns.get(index).cloned().unwrap_or_default()),
        );
    }
    grid = grid.child(head);
    for row in &section.rows {
        let mut line = div().flex().flex_row().gap_2().w_full().min_w(px(0.));
        for index in 0..columns {
            line = line.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .text_color(rgb(if index == 0 {
                        palette.text
                    } else {
                        palette.muted
                    }))
                    .child(row.get(index).cloned().unwrap_or_default()),
            );
        }
        grid = grid.child(line);
    }
    grid
}

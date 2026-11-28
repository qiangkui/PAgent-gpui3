//! The statistics cards: the summary tiles, the chart frames and the two
//! distribution lists.
//!
//! Data, aggregation and the layout constants stay in `stats.rs`; everything
//! here only draws, so a change to a chart cannot disturb the figures it shows.

use super::*;

/// One chip of the range picker; the chosen one is filled with the accent.
pub(super) fn range_chip(
    index: usize,
    range: StatsRange,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    div()
        .id(("stats-range", index))
        .px_3()
        .py_1()
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(if selected {
            palette.accent
        } else {
            palette.panel
        }))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .text_xs()
        .text_color(rgb(if selected {
            palette.accent_text
        } else {
            palette.muted
        }))
        .cursor_pointer()
        .hover(move |style| {
            style.bg(rgb(if selected {
                palette.accent
            } else {
                palette.panel_soft
            }))
        })
        .on_click(cx.listener(move |view, _, _, cx| view.set_stats_range(range, cx)))
        .child(range.label())
        .into_any_element()
}

/// The line above the cards: the window's first moment, its bucket width, and
/// how many conversations answered in it.
pub(super) fn coverage_line(report: &StatsReport, range: StatsRange, palette: Palette) -> Div {
    let chats = if report.chats == 0 {
        "还没有对话有请求".to_string()
    } else {
        format!("{} 个对话有请求", report.chats)
    };
    let dot = || div().text_xs().text_color(rgb(palette.faint)).child("·");
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap_2()
        .px_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{} 至今", stamp_label(report.start_ms))),
        )
        .child(dot())
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(format!("每格 {}", range.bucket_name())),
        )
        .child(dot())
        .child(div().text_xs().text_color(rgb(palette.faint)).child(chats))
}

/// `11-23 00:00`, the first moment of a window.
pub(super) fn stamp_label(ms: u64) -> String {
    let (_, month, day) = civil_from_days((ms / DAY_MS) as i64);
    let hour = (ms / HOUR_MS) % 24;
    format!("{month:02}-{day:02} {hour:02}:00")
}

/// The surface every card shares: the panel colour and a soft shadow, with no
/// border. Width and height are left to the row the card is placed in.
pub(super) fn card_surface(palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .rounded_lg()
        .bg(rgb(palette.panel))
        .shadow_md()
}

/// A chart card in the two-per-row grid: the pair shares width and height.
///
/// Fixed pixel widths keep both cards on whole pixels, so their shadows never
/// meet on a fractional edge; a pane narrower than the pair wraps instead of
/// squeezing the cards into subpixel widths.
pub(super) fn chart_card(palette: Palette) -> Div {
    card_surface(palette)
        .w(px(STATS_CARD_WIDTH))
        .h(px(STATS_CARD_HEIGHT))
}

/// The token chart's card: one full row of its own.
pub(super) fn long_card(palette: Palette) -> Div {
    card_surface(palette).w_full().h(px(STATS_LONG_HEIGHT))
}

/// One of the three totals on the first row: a label, a big figure and a note.
///
/// The three tiles split their row equally, so the three headline figures read
/// as one band rather than as three differently sized cards.
pub(super) fn summary_tile(
    label: &str,
    value: String,
    note: String,
    colour: u32,
    palette: Palette,
) -> AnyElement {
    card_surface(palette)
        .w(px(STATS_TILE_WIDTH))
        .h(px(STATS_TILE_HEIGHT))
        // A tighter shadow than the chart cards: three of them sit side by side
        // and the wider blur would meet in the gaps as a visible seam.
        .shadow_sm()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(status_dot(colour))
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(label.to_string()),
                ),
        )
        .child(
            div()
                .text_size(px(30.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(value),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(note),
        )
        .into_any_element()
}

/// A card's title, with a short note pinned to its right edge.
pub(super) fn card_heading(title: &str, note: &str, palette: Palette) -> Div {
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
                .child(title.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_right()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(note.to_string()),
        )
}

/// The big figure at the top of a card, with its unit beside it.
pub(super) fn card_value(value: String, unit: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_end()
        .gap_1()
        .child(
            div()
                .text_size(px(26.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(value),
        )
        .child(
            div()
                .pb(px(3.))
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(unit.to_string()),
        )
}

/// Every digit of a count, for the headline totals where an abbreviation would
/// hide how much was spent.
pub(super) fn full_count(count: u64) -> String {
    count.to_string()
}

/// One faint line under a chart: its peak, or what the bars add up to.
pub(super) fn chart_foot(text: String, palette: Palette) -> Div {
    div()
        .text_xs()
        .text_color(rgb(palette.faint))
        .whitespace_normal()
        .child(text)
}

/// A bar per bucket, newest on the right and tinted with the card's colour.
///
/// `labels` is one per bar, or empty when the window is too dense to name every
/// bar; `step` lets a day's twelve bars show every other hour. The newest bar
/// keeps the accent colour, so the reader can always find "now" in the chart.
/// `vivid` keeps every bar in the card's colour (the rest slightly faded), which
/// a day's twelve bars need to read at a glance; the longer windows leave the
/// older bars quiet so the newest one stands out on its own.
pub(super) fn bar_chart(
    values: &[u64],
    labels: &[String],
    peak: u64,
    colour: u32,
    step: usize,
    vivid: bool,
    height: f32,
    palette: Palette,
) -> AnyElement {
    let peak = peak.max(1);
    let newest = values.len().saturating_sub(1);
    let mut bars = div()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(2.))
        .w_full()
        .min_w(px(0.))
        .h(px(height));
    for (index, value) in values.iter().enumerate() {
        let bar = if *value == 0 {
            // A flat bar would be invisible; a hair of the track keeps each
            // empty bucket visible without pretending it holds a value.
            div()
                .w_full()
                .h(px(1.))
                .rounded_sm()
                .bg(rgb(palette.panel_soft))
        } else {
            let share = (*value as f32 / peak as f32).clamp(0.05, 1.);
            let (fill, faded) = if index == newest {
                (colour, false)
            } else if vivid {
                (colour, true)
            } else {
                (palette.border, false)
            };
            let mut bar = div().w_full().h(relative(share)).rounded_sm().bg(rgb(fill));
            if faded {
                bar = bar.opacity(0.7);
            }
            bar
        };
        bars = bars.child(
            div()
                .flex()
                .flex_col()
                .justify_end()
                .flex_1()
                .min_w(px(2.))
                .h_full()
                .child(bar),
        );
    }
    // The plot is the bars over equally spaced horizontal rules. The rules are
    // painted first, so a bar always sits on top of the line behind it.
    let mut plot = div()
        .relative()
        .w_full()
        .min_w(px(0.))
        .h(px(height))
        .border_b_1()
        .border_color(rgb(palette.border));
    for band in 0..CHART_BANDS {
        plot = plot.child(
            div()
                .absolute()
                .top(relative(band as f32 / CHART_BANDS as f32))
                .left(px(0.))
                .w_full()
                .h(px(1.))
                .bg(rgb(palette.border))
                .opacity(0.7),
        );
    }
    plot = plot.child(bars);
    let mut body = div()
        .flex()
        .flex_col()
        .gap_1()
        .w_full()
        .min_w(px(0.))
        .child(plot);
    if !labels.is_empty() {
        let mut ticks = div().flex().flex_row().gap(px(2.)).w_full().min_w(px(0.));
        for index in 0..values.len() {
            let show = index % step.max(1) == 0 || index == newest;
            ticks = ticks.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_center()
                    .text_size(px(9.))
                    .text_color(rgb(if index == newest {
                        colour
                    } else {
                        palette.faint
                    }))
                    .child(if show {
                        labels.get(index).cloned().unwrap_or_default()
                    } else {
                        String::new()
                    }),
            );
        }
        body = body.child(ticks);
    }
    body.into_any_element()
}

/// How many requests were answered in each bar.
pub(super) fn requests_card(
    report: &StatsReport,
    range: StatsRange,
    palette: Palette,
) -> AnyElement {
    let values: Vec<u64> = report
        .buckets
        .iter()
        .map(|bucket| bucket.requests)
        .collect();
    let labels = range.tick_labels(report.start_ms);
    let peak = values.iter().copied().max().unwrap_or(0);
    chart_card(palette)
        .child(card_heading(
            "请求次数",
            &format!("每格 {}", range.bucket_name()),
            palette,
        ))
        .child(card_value(short_count(report.requests), "次", palette))
        .child(bar_chart(
            &values,
            &labels,
            peak,
            palette.accent,
            range.label_step(),
            range == StatsRange::Day,
            CHART_HEIGHT,
            palette,
        ))
        .child(chart_foot(
            format!(
                "最新在右 · 最高 {} 次 · {} 个对话",
                short_count(peak),
                report.chats
            ),
            palette,
        ))
        .into_any_element()
}

/// Tokens the endpoint billed, per bar.
pub(super) fn tokens_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
    let values: Vec<u64> = report.buckets.iter().map(|bucket| bucket.tokens).collect();
    let labels = range.tick_labels(report.start_ms);
    let peak = values.iter().copied().max().unwrap_or(0);
    long_card(palette)
        .child(card_heading(
            "Token 用量",
            &format!("每格 {}", range.bucket_name()),
            palette,
        ))
        .child(card_value(full_count(report.tokens), "tokens", palette))
        .child(bar_chart(
            &values,
            &labels,
            peak,
            palette.sky,
            range.label_step(),
            range == StatsRange::Day,
            LONG_CHART_HEIGHT,
            palette,
        ))
        .child(chart_foot(
            format!(
                "最新在右 · 提示 {} · 平均每次 {} tokens",
                full_count(report.prompt),
                full_count(report.average_tokens())
            ),
            palette,
        ))
        .into_any_element()
}

/// Conversations that answered in each bar, counted once per bar.
pub(super) fn chats_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
    let values: Vec<u64> = report.buckets.iter().map(|bucket| bucket.chats).collect();
    let labels = range.tick_labels(report.start_ms);
    let peak = values.iter().copied().max().unwrap_or(0);
    chart_card(palette)
        .child(card_heading(
            "活跃对话",
            &format!("每格 {}", range.bucket_name()),
            palette,
        ))
        .child(card_value(report.chats.to_string(), "个", palette))
        .child(bar_chart(
            &values,
            &labels,
            peak,
            palette.ok,
            range.label_step(),
            range == StatsRange::Day,
            CHART_HEIGHT,
            palette,
        ))
        .child(chart_foot(
            format!("最新在右 · 同一对话一格只算一次 · 最高 {peak} 个"),
            palette,
        ))
        .into_any_element()
}

/// Tokens per request in each bar, so a heavy day shows up even when its
/// request count looks ordinary.
pub(super) fn average_card(
    report: &StatsReport,
    range: StatsRange,
    palette: Palette,
) -> AnyElement {
    let values: Vec<u64> = report
        .buckets
        .iter()
        .map(|bucket| bucket.tokens.checked_div(bucket.requests).unwrap_or(0))
        .collect();
    let labels = range.tick_labels(report.start_ms);
    let peak = values.iter().copied().max().unwrap_or(0);
    chart_card(palette)
        .child(card_heading(
            "平均每次请求",
            &format!("每格 {}", range.bucket_name()),
            palette,
        ))
        .child(card_value(
            full_count(report.average_tokens()),
            "tokens / 次",
            palette,
        ))
        .child(bar_chart(
            &values,
            &labels,
            peak,
            palette.warn,
            range.label_step(),
            range == StatsRange::Day,
            CHART_HEIGHT,
            palette,
        ))
        .child(chart_foot(
            format!("最新在右 · 按格平均 · 最高 {}", full_count(peak)),
            palette,
        ))
        .into_any_element()
}

/// Which models answered, as horizontal bars sized by their token share.
pub(super) fn models_card(report: &StatsReport, palette: Palette) -> AnyElement {
    let mut card = chart_card(palette).child(card_heading("模型分布", "按 token 排序", palette));
    if report.models.is_empty() {
        return card
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("这段时间还没有请求"),
            )
            .into_any_element();
    }
    let peak = report
        .models
        .iter()
        .map(|model| model.tokens)
        .max()
        .unwrap_or(0)
        .max(1);
    // The card has a fixed height, so the rows share it: five models plus a
    // "还有 N 个" line fit where six rows would spill out.
    let mut rows = div().flex().flex_col().gap_2().w_full().min_w(px(0.));
    for model in report.models.iter().take(MODELS_SHOWN) {
        let bar = if model.tokens == 0 {
            div()
                .h(px(1.))
                .w_full()
                .rounded_full()
                .bg(rgb(palette.faint))
        } else {
            let share = (model.tokens as f32 / peak as f32).clamp(0.03, 1.);
            div()
                .h_full()
                .w(relative(share))
                .rounded_full()
                .bg(rgb(palette.accent))
        };
        // A model's cache share rides in the same line as its counts; a model
        // the endpoint never reported hits for shows only the counts.
        let cache = (model.cache_hits > 0 && model.prompt_tokens > 0)
            .then(|| model.cache_hits.min(model.prompt_tokens) as f64 / model.prompt_tokens as f64);
        let summary = match cache {
            Some(rate) => format!(
                "{} 次 · {} · 缓存 {}",
                short_count(model.requests),
                short_count(model.tokens),
                share_text(true, rate)
            ),
            None => format!(
                "{} 次 · {}",
                short_count(model.requests),
                short_count(model.tokens)
            ),
        };
        rows = rows.child(
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
                        .gap_2()
                        .w_full()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(model.model.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(summary),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(rgb(palette.panel_soft))
                        .child(bar),
                ),
        );
    }
    card = card.child(rows);
    let hidden = report.models.len().saturating_sub(MODELS_SHOWN);
    if hidden > 0 {
        card = card.child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(format!("还有 {hidden} 个模型没有列出")),
        );
    }
    card.into_any_element()
}

/// Which tools ran, as horizontal bars sized by call count.
///
/// The same shape as the model card: one row per tool, its share of the window's
/// calls in words, and a bar sized against the busiest one.
pub(super) fn tools_card(report: &StatsReport, palette: Palette) -> AnyElement {
    let mut card = chart_card(palette).child(card_heading("工具分布", "按调用次数排序", palette));
    if report.tool_calls.is_empty() {
        return card
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("这段时间还没有记录到工具调用"),
            )
            .into_any_element();
    }
    let peak = report
        .tool_calls
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let total: u64 = report.tool_calls.iter().map(|(_, count)| *count).sum();
    let mut rows = div().flex().flex_col().gap_2().w_full().min_w(px(0.));
    for (name, count) in report.tool_calls.iter().take(TOOLS_SHOWN) {
        let bar = if *count == 0 {
            div()
                .h(px(1.))
                .w_full()
                .rounded_full()
                .bg(rgb(palette.faint))
        } else {
            let share = (*count as f32 / peak as f32).clamp(0.03, 1.);
            div()
                .h_full()
                .w(relative(share))
                .rounded_full()
                .bg(rgb(palette.warn))
        };
        rows = rows.child(
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
                        .gap_2()
                        .w_full()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.text))
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{} 次 · {}",
                                    short_count(*count),
                                    share_text(true, *count as f64 / total.max(1) as f64)
                                )),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(rgb(palette.panel_soft))
                        .child(bar),
                ),
        );
    }
    card = card.child(rows);
    let hidden = report.tool_calls.len().saturating_sub(TOOLS_SHOWN);
    if hidden > 0 {
        card = card.child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(format!("还有 {hidden} 个工具没有列出")),
        );
    }
    card.into_any_element()
}

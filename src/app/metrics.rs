//! The figures the composer's status capsule shows.
//!
//! The capsule replaced the two labels the header and the tab strip used to
//! carry (the billed total and the context share). Everything here is derived
//! from what the transcript already keeps — the answers' `tokens`, `context`,
//! `round_tokens` and `cache_hits` — so the strip costs no extra bookkeeping
//! and survives a restart with the conversation.
//!
//! The billed total itself is gone: a price in 元 needs a per-model table that
//! drifts away from the provider's own billing (and DeepSeek charges double in
//! peak hours), so the capsule reports only what the endpoint measured.

use super::*;

/// What one conversation reports: how many questions were asked, how many round
/// trips they took, what the endpoint billed, and how full the window is.
pub(crate) struct ChatMetrics {
    /// Questions asked in this conversation: one question is one 问.
    pub(crate) asked: u64,
    /// Model round trips across the whole conversation — the tool loop's
    /// back-and-forths plus the answer that closes each question.
    pub(crate) steps: u64,
    /// Tokens per second of the last answer; zero when unknown.
    pub(crate) speed: u64,
    /// Tokens the endpoint billed for the conversation, summed over every
    /// answer; the same figure the context panel shows.
    pub(crate) billed: u64,
    /// True when some answer reported cache hits at all.
    pub(crate) cache_known: bool,
    /// Share of the conversation's whole prompt that came from the provider's
    /// cache, over every request it answered.
    pub(crate) cache_share: f64,
    /// How full the model window is, 0–100, for the gauge beside them.
    pub(crate) context_percent: f64,
}

impl DshFlowApp {
    /// What the capsule under the composer reports.
    ///
    /// While a question is in flight its round trips are not in the transcript
    /// yet (the answer that will hold them has not landed), so the running count
    /// from the engine's loop is added on top: it grows as the loop goes.
    pub(crate) fn chat_metrics(&self) -> ChatMetrics {
        let mut metrics = chat_metrics(self.chat(), &self.chat_model());
        if let Some(pending) = self.active_pending() {
            metrics.steps = metrics.steps.saturating_add(u64::from(pending.round));
        }
        metrics
    }
}

/// The figures of one conversation, read from its transcript and its model.
///
/// The three counts follow the same rules as the context panel, so the strip and
/// the panel never disagree: one question is one 问, the loop's back-and-forths
/// (each tool round, plus the answer that closes the question) are the 步, and
/// the billed total is what the endpoint reported for every answer.
pub(crate) fn chat_metrics(chat: &Chat, model: &str) -> ChatMetrics {
    let last = chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant);
    let asked = chat
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .count() as u64;
    // One question is one 轮, and every round trip it took is one 步: a round
    // that called tools counts once, and the answer that closed it counts once.
    // Older saves only kept the round list, and a save older still gets the one
    // trip any answer must have taken.
    let mut steps = 0u64;
    for answer in chat
        .messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
    {
        let counted: u64 = if answer.rounds > 0 {
            u64::from(answer.rounds)
        } else if !answer.round_tokens.is_empty() {
            answer.round_tokens.len() as u64
        } else {
            1
        };
        steps = steps.saturating_add(counted);
    }
    let billed = chat.messages.iter().map(|message| message.tokens).sum();
    let speed = last.map_or(0, |answer| {
        if answer.elapsed_ms == 0 || answer.tokens == 0 {
            0
        } else {
            (answer.tokens * 1000 / answer.elapsed_ms).max(1)
        }
    });
    // The cache share is the whole conversation's: every request the transcript
    // answered, so a long chat that warmed the provider's cache reads higher
    // than a single lucky round trip.
    let mut cached = 0u64;
    let mut prompt = 0u64;
    let mut cache_known = false;
    for answer in chat
        .messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
    {
        cached = cached.saturating_add(answer.cache_hits);
        cache_known |= answer.cache_hits > 0;
        if answer.round_tokens.is_empty() {
            prompt = prompt.saturating_add(answer.tokens);
        } else {
            for (sent, _, _) in &answer.round_tokens {
                prompt = prompt.saturating_add(*sent);
            }
        }
    }
    ChatMetrics {
        asked,
        steps,
        speed,
        billed,
        cache_known,
        cache_share: share(cached, prompt),
        context_percent: chat_context_percent(chat, model),
    }
}

impl DshFlowApp {
    /// The long pill under the composer: rounds and speed, what the last
    /// request carried, what the conversation has cost, and how full the window
    /// is. It takes the card's surface and shadow so the two read as one stack.
    pub(crate) fn render_status_capsule(&self, palette: Palette) -> Div {
        let metrics = self.chat_metrics();
        let tint = move |text: String, colour: u32| {
            div()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .text_color(rgb(colour))
                .child(text)
        };
        let icon = move |path: &'static str, size: f32, colour: u32| {
            svg()
                .path(path)
                .w(px(size))
                .h(px(size))
                .flex_shrink_0()
                .text_color(rgb(colour))
        };
        // The two groups can give up width; the gauge beside them cannot, so a
        // narrow window clips text rather than the ring.
        let group = move || div().flex().flex_row().items_center().gap_1().min_w(px(0.));
        // Questions asked and round trips taken, then how fast the last answer was
        // written. Both counts cover the whole conversation, the same as the
        // context panel's 轮次 / 往返 columns.
        let mut rounds_text = asked_label(metrics.asked, metrics.steps);
        if let Some(speed) = speed_label(metrics.speed) {
            rounds_text.push_str(" · ");
            rounds_text.push_str(&speed);
        }
        let rounds_group = group()
            .child(icon("icons/regenerate2.svg", 12., palette.faint))
            .child(tint(rounds_text, palette.muted));
        // What the endpoint billed for the conversation, then how much of the
        // conversation's own prompt the provider served from its cache. With a
        // spending limit set, the figure is shown against it and turns amber once
        // the conversation has spent it.
        let budget = self.ai.budget_tokens;
        let over = budget > 0 && metrics.billed >= budget;
        let carried_text = if budget > 0 {
            format!(
                "{} / {} tok{}",
                token_amount(metrics.billed),
                token_amount(budget),
                if over { " · 已超上限" } else { "" }
            )
        } else {
            format!(
                "{} tok · {}",
                token_amount(metrics.billed),
                cache_share_label(metrics.cache_known, metrics.cache_share)
            )
        };
        let carried = group()
            .child(icon("icons/layers.svg", 12., palette.faint))
            .child(tint(
                carried_text,
                if over { palette.warn } else { palette.muted },
            ));
        // The gauge: a ring whose arc is the share of the window in use, then
        // the figure itself, tinted once the trim is close.
        let percent = metrics.context_percent;
        let warn = percent >= 90.;
        let gauge = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(consumption_ring(
                (percent > 0.)
                    .then_some((
                        if warn { palette.warn } else { palette.accent },
                        (percent / 100.) as f32,
                    ))
                    .into_iter()
                    .collect(),
                palette.panel_soft,
                12.,
                2.,
            ))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(if warn { palette.warn } else { palette.muted }))
                    .child(if percent > 0. {
                        percent_label(percent)
                    } else {
                        "—".to_string()
                    }),
            );

        // The capsule is as wide as the card, and the three parts sit as one centred
        // row inside it: the two side gaps always match, however wide the window
        // gets. That row takes the whole inner width (`flex_1`) so it has a
        // definite size to shrink into; a centred row that merely wraps its
        // content would spill out of both ends once the figures no longer fit,
        // and the capsule would clip them mid-glyph.
        div()
            .w_full()
            .max_w(px(COMPOSER_MAX_WIDTH))
            .h(px(STATUS_CAPSULE_HEIGHT))
            .mt(px(STATUS_CAPSULE_GAP))
            .flex()
            .flex_row()
            .items_center()
            .px_3()
            .rounded_full()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(rounds_group)
                    .child(carried)
                    .child(gauge),
            )
    }
}

/// `3 问 7 步`: questions asked, then the round trips they took. Called `问`
/// rather than `轮` because `轮` in the context panel already means a model
/// round trip, and this first figure is the number of questions.
pub(crate) fn asked_label(asked: u64, steps: u64) -> String {
    format!("{asked} 问 {steps} 步")
}

/// Tokens per second of the last answer; `None` when the time or the count is
/// unknown, so the strip does not print a meaningless zero.
pub(crate) fn speed_label(speed: u64) -> Option<String> {
    (speed > 0).then(|| format!("{speed} tok/s"))
}

/// `缓存命中 96%` for the whole conversation. Spelled out because the strip has
/// room for it now that the billed total is gone, and "命中" alone reads like a
/// hit count rather than a cache share.
pub(crate) fn cache_share_label(known: bool, share: f64) -> String {
    format!("缓存命中 {}", share_text(known, share))
}

/// A share of a prompt as a percentage: `96%`, `95.2%`, `<1%`, or `—`.
pub(crate) fn share_text(known: bool, share: f64) -> String {
    if !known {
        return "—".to_string();
    }
    let percent = (share * 100.).clamp(0., 100.);
    if percent > 0. && percent < 1. {
        return "<1%".to_string();
    }
    let rounded = (percent * 10.).round() / 10.;
    if (rounded - rounded.round()).abs() < f64::EPSILON {
        format!("{}%", rounded.round() as u64)
    } else {
        format!("{rounded:.1}%")
    }
}

/// The window gauge as a percentage, clamped to its two ends.
pub(crate) fn percent_label(percent: f64) -> String {
    if percent > 0. && percent < 1. {
        return "<1%".to_string();
    }
    format!("{}%", percent.round().clamp(0., 100.) as u64)
}

/// Token counts the way the strip writes them: `792`, `1.8K`, `38.1K`, `1.2M`.
pub(crate) fn token_amount(count: u64) -> String {
    if count < 1000 {
        return count.to_string();
    }
    if count < 1_000_000 {
        return format!("{:.1}K", count as f64 / 1000.);
    }
    format!("{:.2}M", count as f64 / 1_000_000.)
}

/// One part of a whole as a 0–1 share; zero when the whole is unknown.
fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.
    } else {
        (part.min(whole) as f64) / (whole as f64)
    }
}

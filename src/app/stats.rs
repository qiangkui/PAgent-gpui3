//! The statistics page: what the assistant was asked over a chosen window.
//!
//! Every card reads from the permanent statistics ledger in
//! `<program>/data/stats`: request details for the last 30 days, plus one
//! permanent aggregate per UTC day. Deleting a conversation never changes these
//! figures. The one-time migration in `stats_store` imports the older
//! per-conversation request files when the ledger is first created.

use super::*;
use std::collections::{BTreeMap, HashMap, HashSet};

mod cards;
use cards::*;

/// One hour, in milliseconds.
pub(super) const HOUR_MS: u64 = 3_600_000;
/// One day, in milliseconds.
pub(super) const DAY_MS: u64 = 86_400_000;
/// Widest the card area grows; a wider pane leaves equal gutters on both sides.
pub(super) const STATS_AREA_WIDTH: f32 = 822.;
/// Width of one summary tile: three tiles and two gaps fill the area exactly.
pub(super) const STATS_TILE_WIDTH: f32 = 266.;
/// Width of one chart card: two cards and one gap fill the area exactly.
pub(super) const STATS_CARD_WIDTH: f32 = 405.;
/// Height of the three summary tiles on the first row.
///
/// The surface pads 16px top and bottom and puts 8px between its three lines.
/// GPUI gives a line box the golden ratio of its font size by default, so the
/// 16px label, the 30px figure and the 12px note need 26 + 49 + 19 pixels:
/// 142 all told. At 128 the note — the only child that truncates, and therefore
/// the only one allowed to shrink — collapsed to a sliver of glyph tops.
pub(super) const STATS_TILE_HEIGHT: f32 = 144.;
/// Height of a chart card placed two per row, so the pair always matches.
pub(super) const STATS_CARD_HEIGHT: f32 = 260.;
/// Height of the full-width Token 用量 card.
pub(super) const STATS_LONG_HEIGHT: f32 = 320.;
/// Gap kept below the last element inside the scroll content, so the stop
/// point leaves that distance visible instead of pulling the content flush.
pub(super) const STATS_BOTTOM_GAP: f32 = 16.;
/// Plot area of a chart card's bars.
pub(super) const CHART_HEIGHT: f32 = 84.;
/// Equal horizontal bands the chart's grid rules split the plot into.
pub(super) const CHART_BANDS: usize = 4;
/// Plot area of the full-width token chart.
pub(super) const LONG_CHART_HEIGHT: f32 = 150.;
/// Models listed on the distribution card before the rest are folded away.
pub(super) const MODELS_SHOWN: usize = 5;
/// Tools listed on the distribution card before the rest are folded away.
pub(super) const TOOLS_SHOWN: usize = 5;

/// The span of time the page covers, and how finely it splits it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StatsRange {
    /// The last 24 hours, two hours per bar.
    Day,
    /// The last 7 days, one day per bar.
    Week,
    /// The last 30 days, one day per bar.
    Month,
    /// The last 90 days, one week per bar.
    Quarter,
}

impl StatsRange {
    /// Every window, in the order the chips show them.
    pub(crate) const ALL: [StatsRange; 4] = [
        StatsRange::Day,
        StatsRange::Week,
        StatsRange::Month,
        StatsRange::Quarter,
    ];

    /// Name on the chip.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Day => "24 小时",
            Self::Week => "7 天",
            Self::Month => "30 天",
            Self::Quarter => "90 天",
        }
    }

    /// How long one bar covers, in words.
    pub(crate) fn bucket_name(self) -> &'static str {
        match self {
            Self::Day => "2 小时",
            Self::Week | Self::Month => "1 天",
            Self::Quarter => "1 周",
        }
    }

    /// How long one bar covers, in milliseconds.
    pub(crate) fn bucket_ms(self) -> u64 {
        match self {
            Self::Day => 2 * HOUR_MS,
            Self::Week | Self::Month => DAY_MS,
            Self::Quarter => 7 * DAY_MS,
        }
    }

    /// How many bars the chart draws.
    pub(crate) fn buckets(self) -> usize {
        match self {
            Self::Day => 12,
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 13,
        }
    }

    /// Every `nth` bar gets a label; the newest bar is always labelled.
    pub(crate) fn label_step(self) -> usize {
        match self {
            Self::Day => 2,
            Self::Week => 1,
            Self::Month => 5,
            Self::Quarter => 2,
        }
    }

    /// Labels under the bars, or none when they would not fit.
    ///
    /// A day's bars are two hours apart, so two digits name each one; a week's
    /// seven bars have room for the date. A month and a quarter have too many
    /// bars for a label each, so their cards name the window above them
    /// instead of printing a crowd of clipped dates.
    pub(crate) fn tick_labels(self, start_ms: u64) -> Vec<String> {
        if matches!(self, Self::Month | Self::Quarter) {
            return Vec::new();
        }
        let bucket = self.bucket_ms();
        (0..self.buckets())
            .map(|index| {
                let at = start_ms + index as u64 * bucket;
                match self {
                    Self::Day => format!("{:02}", (at / HOUR_MS) % 24),
                    Self::Week => self.bucket_label(at),
                    Self::Month | Self::Quarter => String::new(),
                }
            })
            .collect()
    }

    /// The window ending with the bar that holds `now`: `(start, end)`.
    ///
    /// Buckets are aligned to the epoch, so a "day" bar is a UTC day and two
    /// requests answered in the same hour always land in the same bar.
    pub(crate) fn window(self, now_ms: u64) -> (u64, u64) {
        let bucket = self.bucket_ms();
        let last = now_ms / bucket * bucket;
        let start = last.saturating_sub(bucket * (self.buckets() as u64 - 1));
        (start, last + bucket)
    }

    /// Which bar `at_ms` falls in, for a window that started at `start_ms`.
    pub(crate) fn bucket_of(self, at_ms: u64, start_ms: u64) -> Option<usize> {
        if at_ms < start_ms {
            return None;
        }
        let index = (at_ms - start_ms) / self.bucket_ms();
        (index < self.buckets() as u64).then_some(index as usize)
    }

    /// The label under one bar: an hour range, or the UTC date it starts on.
    pub(crate) fn bucket_label(self, start_ms: u64) -> String {
        match self {
            Self::Day => {
                let hour = (start_ms / HOUR_MS) % 24;
                format!("{hour:02}–{:02}", (hour + 2) % 24)
            }
            Self::Week | Self::Month | Self::Quarter => {
                let (_, month, day) = civil_from_days((start_ms / DAY_MS) as i64);
                format!("{month:02}-{day:02}")
            }
        }
    }
}

/// What a cached report was built from: the window, the ledger generation and
/// the bucket `now` falls in. It deliberately does not include the conversation
/// list: deleting a conversation must leave the historical figures alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatsKey {
    /// Window on show.
    pub(crate) range: StatsRange,
    /// Bumped whenever a request is appended or the ledger is migrated.
    pub(crate) generation: u64,
    /// Which bucket `now` fell in when the report was built.
    pub(crate) bucket: u64,
}

/// One answered request, as the charts count it.
pub(crate) struct StatsPoint {
    /// When the answer arrived, in epoch milliseconds.
    pub(crate) at_ms: u64,
    /// Folder of the conversation it belongs to.
    pub(crate) chat: String,
    /// Conversation title at the time of the request.
    pub(crate) title: String,
    /// Which question it was in that conversation, counting from one.
    pub(crate) turn: u64,
    /// What was asked, as the request list names it; empty when the save did
    /// not keep the question.
    pub(crate) question: String,
    /// Model that answered; empty when the save did not record one.
    pub(crate) model: String,
    /// Tokens the endpoint billed for the whole question.
    pub(crate) tokens: u64,
    /// Prompt + answer of the last round, which is what filled the window.
    pub(crate) context: u64,
    /// Tool calls the question made; zero when the save did not record them.
    pub(crate) tools: u64,
    /// How often each tool was called, when the record kept the names; empty
    /// for requests recorded before it did.
    pub(crate) tool_calls: BTreeMap<String, u64>,
    /// Prompt tokens the provider served from its cache.
    pub(crate) cache_hits: u64,
    /// Prompt tokens the question sent, summed over its rounds.
    pub(crate) prompt_tokens: u64,
    /// Wall clock spent waiting for model round trips, in milliseconds.
    pub(crate) model_ms: u64,
    /// Wall clock spent running tools, in milliseconds.
    pub(crate) tool_ms: u64,
    /// Searches the provider ran on its side for the question.
    pub(crate) web_searches: u32,
    /// Pages the provider opened or read for the question.
    pub(crate) web_fetches: u32,
    /// Thinking tokens the question produced, summed over its rounds.
    pub(crate) reasoning_tokens: u64,
    /// True when the time is the conversation's last change, not the answer's.
    pub(crate) estimated: bool,
}

/// One bar of a chart.
#[derive(Clone, Default)]
pub(crate) struct StatsBucket {
    /// Requests answered in this bar.
    pub(crate) requests: u64,
    /// Tokens the endpoint billed in this bar.
    pub(crate) tokens: u64,
    /// Prompt tokens of the last rounds answered in this bar.
    pub(crate) prompt: u64,
    /// Conversations that answered at least once in this bar.
    pub(crate) chats: u64,
}

/// One model's share of the window.
#[derive(Clone)]
pub(crate) struct ModelShare {
    /// Name the endpoint reported; `未知模型` when it reported none.
    pub(crate) model: String,
    /// Requests it answered.
    pub(crate) requests: u64,
    /// Tokens it was billed for.
    pub(crate) tokens: u64,
    /// Prompt tokens the provider served from its cache, for this model.
    pub(crate) cache_hits: u64,
    /// Prompt tokens its requests sent, summed over rounds.
    pub(crate) prompt_tokens: u64,
    /// Wall clock it spent on model round trips, in milliseconds.
    pub(crate) model_ms: u64,
    /// Wall clock its tools spent, in milliseconds.
    pub(crate) tool_ms: u64,
}

/// Everything the cards show, built once per window.
#[derive(Clone, Default)]
pub(crate) struct StatsReport {
    /// One bar per bucket, oldest first.
    pub(crate) buckets: Vec<StatsBucket>,
    /// Requests answered in the window.
    pub(crate) requests: u64,
    /// Tokens billed in the window.
    pub(crate) tokens: u64,
    /// Tool calls made in the window.
    pub(crate) tools: u64,
    /// Prompt tokens of the last rounds in the window.
    pub(crate) prompt: u64,
    /// Prompt tokens the provider served from its cache in the window.
    pub(crate) cache_hits: u64,
    /// Prompt tokens the window's requests sent, summed over their rounds.
    pub(crate) prompt_tokens: u64,
    /// Wall clock the window spent waiting for model round trips.
    pub(crate) model_ms: u64,
    /// Wall clock the window spent running tools.
    pub(crate) tool_ms: u64,
    /// Searches the provider ran on its own side in the window.
    pub(crate) web_searches: u64,
    /// Pages those searches or questions opened.
    pub(crate) web_fetches: u64,
    /// Thinking tokens the window's questions produced.
    pub(crate) reasoning_tokens: u64,
    /// Conversations that answered at least once in the window.
    pub(crate) chats: u64,
    /// Models, busiest first.
    pub(crate) models: Vec<ModelShare>,
    /// Tools by call count, busiest first; empty when no request kept names.
    pub(crate) tool_calls: Vec<(String, u64)>,
    /// First millisecond the window covers.
    pub(crate) start_ms: u64,
    /// True when some figure is placed at a conversation's last change.
    pub(crate) estimated: bool,
}

impl StatsReport {
    /// Tokens per request across the window; zero when nothing was asked.
    pub(crate) fn average_tokens(&self) -> u64 {
        self.tokens.checked_div(self.requests).unwrap_or(0)
    }

    /// The window's cache share, when an endpoint reported hits at all.
    ///
    /// `None` rather than 0%: an endpoint that never reports cache hits would
    /// otherwise read as a cache that never worked.
    pub(crate) fn cache_rate(&self) -> Option<f64> {
        (self.cache_hits > 0 && self.prompt_tokens > 0)
            .then(|| self.cache_hits.min(self.prompt_tokens) as f64 / self.prompt_tokens as f64)
    }
}

/// What a background scan needs from one conversation.
///
/// A conversation with a request file is fully described by its folder and its
/// last change; only one without that file needs its transcript, because its
/// figures live on the answers themselves.
pub(crate) struct StatsJob {
    /// Folder of the conversation.
    pub(crate) dir: String,
    /// Title shown in the sidebar, kept with the imported records.
    pub(crate) title: String,
    /// When it last changed, in epoch milliseconds.
    pub(crate) updated_ms: u64,
    /// Its transcript, empty when the request file can answer for it.
    pub(crate) messages: Vec<ChatMessage>,
}

/// Every request the stored conversations hold, oldest last.
///
/// The same reading as [`stats_points_for`], for callers that hold the
/// conversations themselves; the window only ever passes jobs, so this is for
/// the tests, which find conversations easier to build.
#[cfg(test)]
pub(crate) fn stats_points(data_dir: &Path, chats: &[Chat], since_ms: u64) -> Vec<StatsPoint> {
    let jobs: Vec<StatsJob> = chats
        .iter()
        .map(|chat| StatsJob {
            dir: chat.dir.clone(),
            title: chat.title.clone(),
            updated_ms: chat.updated_ms,
            messages: chat.messages.clone(),
        })
        .collect();
    stats_points_for(data_dir, &jobs, since_ms)
}

/// The question a stored request was opened with.
///
/// `items` is in send order, and the replayed history before it also carries
/// earlier user turns, so the *last* user piece is the one that started this
/// request. Its label is the first line, which is exactly what the request list
/// shows; a line written before labels existed still has its text.
fn request_question(request: &crate::chats::StoredRequest) -> String {
    request
        .items
        .iter()
        .rev()
        .find(|item| item.kind == ContextKind::User.key())
        .map(|item| {
            let label = item.label.trim();
            if label.is_empty() {
                turn_label(&item.text)
            } else {
                label.to_string()
            }
        })
        .unwrap_or_default()
}

/// The question that opened request `turn` in a saved conversation, if its
/// transcript still holds it.
///
/// `requests.jsonl` keeps the question too, but an older file can have had its
/// pieces shortened; the transcript is the fuller record. A turn counts
/// answers, so the question is the last user or attachment line before that
/// answer.
fn question_of(chat: &Chat, turn: u64) -> Option<String> {
    if turn == 0 {
        return None;
    }
    let mut seen = 0u64;
    let mut pending = String::new();
    for message in &chat.messages {
        match message.role {
            Role::User | Role::Attachment => pending = turn_label(&message.text),
            Role::Assistant => {
                seen = seen.saturating_add(1);
                if seen == turn {
                    return (!pending.trim().is_empty()).then_some(pending);
                }
                pending.clear();
            }
            _ => {}
        }
    }
    None
}

/// Every request the stored conversations hold, oldest last.
///
/// A conversation whose last change predates `since_ms` can hold no request
/// inside the window, so its file is not opened at all. One saved without a
/// timestamp is always read, because nothing is known about its age.
pub(crate) fn stats_points_for(
    data_dir: &Path,
    chats: &[StatsJob],
    since_ms: u64,
) -> Vec<StatsPoint> {
    let mut points: Vec<StatsPoint> = Vec::new();
    for chat in chats {
        if chat.updated_ms > 0 && chat.updated_ms < since_ms {
            continue;
        }
        let requests = crate::chats::load_requests(data_dir, &chat.dir);
        if !requests.is_empty() {
            // The file stores one line per answered question; a rewritten
            // conversation keeps its newest line per turn.
            let mut stored: Vec<&crate::chats::StoredRequest> = requests.values().collect();
            stored.sort_by_key(|request| request.at_ms);
            for request in stored {
                // New lines carry the count directly; one written before that
                // field existed is counted from its tool-result items.
                let tools = if request.calls > 0 {
                    request.calls
                } else {
                    request
                        .items
                        .iter()
                        .filter(|item| item.kind == ContextKind::ToolResults.key())
                        .count() as u64
                };
                points.push(StatsPoint {
                    tool_calls: counted_tools(&request.tool_names),
                    at_ms: request.at_ms,
                    chat: chat.dir.clone(),
                    title: chat.title.clone(),
                    turn: request.turn,
                    question: request_question(request),
                    model: request.model.clone(),
                    tokens: request.tokens,
                    context: request.context,
                    tools,
                    cache_hits: request.cache_hits,
                    prompt_tokens: request.prompt_tokens,
                    model_ms: request.model_ms,
                    tool_ms: request.tool_ms,
                    web_searches: request.web_searches,
                    web_fetches: request.web_fetches,
                    reasoning_tokens: u64::from(request.reasoning_tokens),
                    estimated: false,
                });
            }
            continue;
        }
        // Conversations written before requests.jsonl existed still hold the
        // figures on their answers; their exact times are not known, so every
        // one of them is placed at the conversation's last change.
        if chat.updated_ms == 0 {
            continue;
        }
        let newest = chat
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::Assistant)
            .map(|message| message.model.clone())
            .unwrap_or_default();
        // One pass over the transcript also counts the tool rows of each
        // question, so an old save still reports its calls. The question is
        // carried along as well: without it every fallback row would show the
        // same time, conversation and model, and the list would read as one
        // request repeated.
        let mut pending_tools = 0u64;
        // The rows also carry the tool names (`read · a.txt`), so an old
        // transcript can report which tools did the work, not only how many.
        let mut pending_tool_names: Vec<String> = Vec::new();
        let mut pending_question = String::new();
        let mut turn = 0u64;
        for message in &chat.messages {
            match message.role {
                Role::Tool => {
                    pending_tools = pending_tools.saturating_add(1);
                    let name = tool_name_of(&message.text);
                    if !name.is_empty() {
                        pending_tool_names.push(name);
                    }
                }
                Role::User | Role::Attachment => {
                    pending_tools = 0;
                    pending_tool_names.clear();
                    pending_question = turn_label(&message.text);
                }
                // Interaction cards are not user questions or answers.
                Role::Question | Role::Plan => {}
                Role::Assistant => {
                    turn = turn.saturating_add(1);
                    let recorded: u64 = message
                        .round_tokens
                        .iter()
                        .map(|(_, _, calls)| u64::from(*calls))
                        .sum();
                    let tools = if recorded > 0 {
                        recorded
                    } else {
                        pending_tools
                    };
                    pending_tools = 0;
                    let fallback_tools = counted_tools(&pending_tool_names);
                    pending_tool_names.clear();
                    let model = if message.model.is_empty() {
                        newest.clone()
                    } else {
                        message.model.clone()
                    };
                    let prompt_tokens: u64 = message
                        .round_tokens
                        .iter()
                        .map(|(prompt, _, _)| *prompt)
                        .sum();
                    points.push(StatsPoint {
                        tool_calls: fallback_tools,
                        at_ms: chat.updated_ms,
                        chat: chat.dir.clone(),
                        title: chat.title.clone(),
                        turn,
                        question: std::mem::take(&mut pending_question),
                        model,
                        tokens: message.tokens,
                        context: message.context,
                        tools,
                        cache_hits: message.cache_hits,
                        prompt_tokens,
                        model_ms: message.model_ms,
                        tool_ms: message.tool_ms,
                        web_searches: 0,
                        web_fetches: 0,
                        reasoning_tokens: message
                            .round_reasoning
                            .iter()
                            .map(|count| u64::from(*count))
                            .sum(),
                        estimated: true,
                    });
                }
                Role::Note | Role::Approval | Role::Thinking => {}
            }
        }
    }
    points
}

/// Add up `points` into the bars and totals the cards draw.
pub(crate) fn stats_report(points: &[StatsPoint], range: StatsRange, now_ms: u64) -> StatsReport {
    let (start_ms, end_ms) = range.window(now_ms);
    let count = range.buckets();
    let mut buckets: Vec<StatsBucket> = (0..count).map(|_| StatsBucket::default()).collect();
    let mut seen: Vec<HashSet<String>> = (0..count).map(|_| HashSet::new()).collect();
    let mut chats: HashSet<String> = HashSet::new();
    let mut tool_counts: HashMap<String, u64> = HashMap::new();
    let mut models: HashMap<String, ModelShare> = HashMap::new();
    let mut report = StatsReport {
        buckets: Vec::new(),
        requests: 0,
        tokens: 0,
        tools: 0,
        prompt: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning_tokens: 0,
        chats: 0,
        models: Vec::new(),
        tool_calls: Vec::new(),
        start_ms,
        estimated: false,
    };
    for point in points {
        if point.at_ms < start_ms || point.at_ms >= end_ms {
            continue;
        }
        let Some(index) = range.bucket_of(point.at_ms, start_ms) else {
            continue;
        };
        let bucket = &mut buckets[index];
        bucket.requests = bucket.requests.saturating_add(1);
        bucket.tokens = bucket.tokens.saturating_add(point.tokens);
        bucket.prompt = bucket.prompt.saturating_add(point.context);
        seen[index].insert(point.chat.clone());
        chats.insert(point.chat.clone());
        report.requests = report.requests.saturating_add(1);
        report.tokens = report.tokens.saturating_add(point.tokens);
        report.tools = report.tools.saturating_add(point.tools);
        add_tool_counts(&mut tool_counts, &point.tool_calls);
        report.prompt = report.prompt.saturating_add(point.context);
        report.cache_hits = report.cache_hits.saturating_add(point.cache_hits);
        report.prompt_tokens = report.prompt_tokens.saturating_add(point.prompt_tokens);
        report.model_ms = report.model_ms.saturating_add(point.model_ms);
        report.tool_ms = report.tool_ms.saturating_add(point.tool_ms);
        report.web_searches = report
            .web_searches
            .saturating_add(u64::from(point.web_searches));
        report.web_fetches = report
            .web_fetches
            .saturating_add(u64::from(point.web_fetches));
        report.reasoning_tokens = report
            .reasoning_tokens
            .saturating_add(point.reasoning_tokens);
        report.estimated |= point.estimated;
        let name = point.model.trim();
        let name = if name.is_empty() {
            "未知模型"
        } else {
            name
        };
        let entry = models.entry(name.to_string()).or_insert(ModelShare {
            model: name.to_string(),
            requests: 0,
            tokens: 0,
            cache_hits: 0,
            prompt_tokens: 0,
            model_ms: 0,
            tool_ms: 0,
        });
        entry.requests = entry.requests.saturating_add(1);
        entry.tokens = entry.tokens.saturating_add(point.tokens);
        entry.cache_hits = entry.cache_hits.saturating_add(point.cache_hits);
        entry.prompt_tokens = entry.prompt_tokens.saturating_add(point.prompt_tokens);
        entry.model_ms = entry.model_ms.saturating_add(point.model_ms);
        entry.tool_ms = entry.tool_ms.saturating_add(point.tool_ms);
    }
    for (index, filled) in seen.iter().enumerate() {
        buckets[index].chats = filled.len() as u64;
    }
    report.buckets = buckets;
    report.chats = chats.len() as u64;
    report.tool_calls = ranked_tools(tool_counts);
    report.models = models.into_values().collect();
    // Busiest by tokens, then by requests, then by name: a stable order keeps
    // the rows from swapping places while the window rolls forward.
    report.models.sort_by(|left, right| {
        right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| right.requests.cmp(&left.requests))
            .then_with(|| left.model.cmp(&right.model))
    });
    report
}

/// Count one question's tool names, keeping the map sorted by name.
///
/// A name repeated across rounds counts once per round: the ledger answers
/// "which tools did this question lean on", not "which were offered".
/// @param names - tool names in call order, duplicates included.
/// @returns name → times called.
fn counted_tools(names: &[String]) -> BTreeMap<String, u64> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let entry = counts.entry(name.to_string()).or_insert(0);
        *entry = entry.saturating_add(1);
    }
    counts
}

/// The tool name at the head of a transcript row: `read · a.txt — …` → `read`.
///
/// A conversation without a request file has no stored names, so they come from
/// the rows the transcript already shows. A row that names no tool yields
/// nothing rather than a blank entry.
/// @param text - the row's text.
/// @returns the name before the separator, trimmed.
fn tool_name_of(text: &str) -> String {
    text.split('·')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The card's order for per-tool counts: busiest first, then by name.
/// @param counts - name → times called, in any order.
/// @returns the pairs the card draws.
fn ranked_tools(counts: HashMap<String, u64>) -> Vec<(String, u64)> {
    let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked
}

/// Add one record's per-tool counts into a running total.
/// @param total - the window's counts so far.
/// @param counts - one request's or one day's counts.
fn add_tool_counts(total: &mut HashMap<String, u64>, counts: &BTreeMap<String, u64>) {
    for (name, count) in counts {
        let entry = total.entry(name.clone()).or_insert(0);
        *entry = entry.saturating_add(*count);
    }
}

/// Turn a completed request into a permanent ledger record.
pub(crate) fn ledger_record(
    chat: &str,
    title: &str,
    request: &crate::chats::StoredRequest,
) -> crate::stats_store::StatsRecord {
    let tools = if request.calls > 0 {
        request.calls
    } else {
        request
            .items
            .iter()
            .filter(|item| item.kind == ContextKind::ToolResults.key())
            .count() as u64
    };
    crate::stats_store::StatsRecord {
        at_ms: request.at_ms,
        chat: chat.to_string(),
        title: title.to_string(),
        turn: request.turn,
        question: request_question(request),
        model: request.model.clone(),
        tokens: request.tokens,
        context: request.context,
        tools,
        cache_hits: request.cache_hits,
        prompt_tokens: request.prompt_tokens,
        model_ms: request.model_ms,
        tool_ms: request.tool_ms,
        web_searches: request.web_searches,
        web_fetches: request.web_fetches,
        reasoning_tokens: u64::from(request.reasoning_tokens),
        tool_calls: counted_tools(&request.tool_names),
        estimated: false,
    }
}

/// Turn a persisted record into the row shape the page already renders.
fn point_from_record(record: crate::stats_store::StatsRecord) -> StatsPoint {
    StatsPoint {
        at_ms: record.at_ms,
        chat: record.chat,
        title: record.title,
        turn: record.turn,
        question: record.question,
        model: record.model,
        tokens: record.tokens,
        context: record.context,
        tools: record.tools,
        tool_calls: record.tool_calls,
        cache_hits: record.cache_hits,
        prompt_tokens: record.prompt_tokens,
        model_ms: record.model_ms,
        tool_ms: record.tool_ms,
        web_searches: record.web_searches,
        web_fetches: record.web_fetches,
        reasoning_tokens: record.reasoning_tokens,
        estimated: record.estimated,
    }
}

/// Turn a request read from a conversation into a ledger record, used only for
/// the one-time import of the old statistics model.
fn record_from_point(point: StatsPoint) -> crate::stats_store::StatsRecord {
    crate::stats_store::StatsRecord {
        at_ms: point.at_ms,
        chat: point.chat,
        title: point.title,
        turn: point.turn,
        question: point.question,
        model: point.model,
        tokens: point.tokens,
        context: point.context,
        tools: point.tools,
        tool_calls: point.tool_calls,
        cache_hits: point.cache_hits,
        prompt_tokens: point.prompt_tokens,
        model_ms: point.model_ms,
        tool_ms: point.tool_ms,
        web_searches: 0,
        web_fetches: 0,
        reasoning_tokens: point.reasoning_tokens,
        estimated: point.estimated,
    }
}

/// Build a report from permanent daily aggregates. Used for the 90-day window,
/// whose first 60 days no longer have request details.
fn report_from_daily(
    days: &[crate::stats_store::DayRollup],
    range: StatsRange,
    now_ms: u64,
) -> StatsReport {
    let (start_ms, end_ms) = range.window(now_ms);
    let count = range.buckets();
    let mut buckets: Vec<StatsBucket> = (0..count).map(|_| StatsBucket::default()).collect();
    let mut seen: Vec<HashSet<String>> = (0..count).map(|_| HashSet::new()).collect();
    let mut chats: HashSet<String> = HashSet::new();
    let mut tool_counts: HashMap<String, u64> = HashMap::new();
    let mut models: HashMap<String, ModelShare> = HashMap::new();
    let mut report = StatsReport {
        buckets: Vec::new(),
        requests: 0,
        tokens: 0,
        tools: 0,
        prompt: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning_tokens: 0,
        chats: 0,
        models: Vec::new(),
        tool_calls: Vec::new(),
        start_ms,
        estimated: false,
    };
    for day in days {
        if day.day_ms < start_ms || day.day_ms >= end_ms {
            continue;
        }
        let Some(index) = range.bucket_of(day.day_ms, start_ms) else {
            continue;
        };
        let bucket = &mut buckets[index];
        bucket.requests = bucket.requests.saturating_add(day.requests);
        bucket.tokens = bucket.tokens.saturating_add(day.tokens);
        bucket.prompt = bucket.prompt.saturating_add(day.prompt);
        for chat in &day.chats {
            seen[index].insert(chat.clone());
            chats.insert(chat.clone());
        }
        report.requests = report.requests.saturating_add(day.requests);
        report.tokens = report.tokens.saturating_add(day.tokens);
        report.tools = report.tools.saturating_add(day.tools);
        add_tool_counts(&mut tool_counts, &day.tool_calls);
        report.prompt = report.prompt.saturating_add(day.prompt);
        report.cache_hits = report.cache_hits.saturating_add(day.cache_hits);
        report.prompt_tokens = report.prompt_tokens.saturating_add(day.prompt_tokens);
        report.model_ms = report.model_ms.saturating_add(day.model_ms);
        report.tool_ms = report.tool_ms.saturating_add(day.tool_ms);
        report.web_searches = report.web_searches.saturating_add(day.web_searches);
        report.web_fetches = report.web_fetches.saturating_add(day.web_fetches);
        report.reasoning_tokens = report.reasoning_tokens.saturating_add(day.reasoning_tokens);
        report.estimated |= day.estimated;
        for (name, counts) in &day.models {
            let name = name.trim();
            let name = if name.is_empty() {
                "未知模型"
            } else {
                name
            };
            let entry = models.entry(name.to_string()).or_insert(ModelShare {
                model: name.to_string(),
                requests: 0,
                tokens: 0,
                cache_hits: 0,
                prompt_tokens: 0,
                model_ms: 0,
                tool_ms: 0,
            });
            entry.requests = entry.requests.saturating_add(counts.requests);
            entry.tokens = entry.tokens.saturating_add(counts.tokens);
            entry.cache_hits = entry.cache_hits.saturating_add(counts.cache_hits);
            entry.prompt_tokens = entry.prompt_tokens.saturating_add(counts.prompt_tokens);
            entry.model_ms = entry.model_ms.saturating_add(counts.model_ms);
            entry.tool_ms = entry.tool_ms.saturating_add(counts.tool_ms);
        }
    }
    for (index, filled) in seen.iter().enumerate() {
        buckets[index].chats = filled.len() as u64;
    }
    report.buckets = buckets;
    report.chats = chats.len() as u64;
    report.tool_calls = ranked_tools(tool_counts);
    report.models = models.into_values().collect();
    report.models.sort_by(|left, right| {
        right
            .tokens
            .cmp(&left.tokens)
            .then_with(|| right.requests.cmp(&left.requests))
            .then_with(|| left.model.cmp(&right.model))
    });
    report
}

impl DshFlowApp {
    /// Switch the window the page covers.
    pub(crate) fn set_stats_range(&mut self, range: StatsRange, cx: &mut Context<Self>) {
        if self.stats_range != range {
            self.stats_range = range;
            *self.stats_cache.borrow_mut() = None;
            cx.notify();
        }
    }

    /// The report for the window on show, rebuilt when the ledger or the bucket
    /// `now` falls in have changed.
    ///
    /// Reading the ledger is done on a worker thread; the page keeps showing the
    /// previous report until the new one lands.
    pub(crate) fn stats_snapshot(&self, cx: &mut Context<Self>) -> StatsReport {
        let now = now_ms();
        let key = StatsKey {
            range: self.stats_range,
            generation: self.stats_generation,
            bucket: now / self.stats_range.bucket_ms(),
        };
        let cached = self.stats_cache.borrow().as_ref().cloned();
        if let Some((cached_key, report, _)) = &cached
            && *cached_key == key
        {
            return report.clone();
        }
        // A miss: show what is there and fill in the new report off-thread.
        if self.stats_loading.borrow().is_none() {
            self.schedule_stats(now, key, cx);
        }
        cached.map(|(_, report, _)| report).unwrap_or_default()
    }

    /// What the background scan needs: a transcript only for a conversation that
    /// has no request file yet.
    fn stats_jobs(&self, data: &Path) -> Vec<StatsJob> {
        self.chats
            .iter()
            .map(|chat| StatsJob {
                dir: chat.dir.clone(),
                title: chat.title.clone(),
                updated_ms: chat.updated_ms,
                messages: if crate::chats::has_requests(data, &chat.dir) {
                    Vec::new()
                } else {
                    chat.messages.clone()
                },
            })
            .collect()
    }

    /// Import the old per-conversation request records into the permanent
    /// ledger once. New requests are appended directly from `chat_actions`.
    fn migration_records(&self) -> Vec<crate::stats_store::StatsRecord> {
        let data = self.ai.data_dir();
        let jobs = self.stats_jobs(&data);
        stats_points_for(&data, &jobs, 0)
            .into_iter()
            .map(record_from_point)
            .collect()
    }

    /// Run the one-time import if the ledger has not been initialized.
    pub(crate) fn migrate_stats(&mut self) -> Result<bool, String> {
        let dir = crate::stats_store::ledger_dir();
        if crate::stats_store::initialized(&dir) {
            crate::stats_store::prune_details(&dir)?;
        }
        crate::stats_store::migrate_if_needed(&dir, &self.migration_records())
    }

    /// Read the ledger on a worker thread and store the report when it arrives.
    fn schedule_stats(&self, now: u64, key: StatsKey, cx: &mut Context<Self>) {
        let dir = crate::stats_store::ledger_dir();
        let range = self.stats_range;
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (start, end) = range.window(now);
            let points: Vec<StatsPoint> = crate::stats_store::load_records(&dir, start, end)
                .into_iter()
                .map(point_from_record)
                .collect();
            // A quarter spans more than the 30-day detail window, so its cards
            // come from the permanent daily rollups. The other ranges fit in the
            // detail file and keep their exact request times.
            let report = if range == StatsRange::Quarter {
                let days = crate::stats_store::load_daily(&dir, start, end);
                if days.is_empty() {
                    stats_report(&points, range, now)
                } else {
                    report_from_daily(&days, range, now)
                }
            } else {
                stats_report(&points, range, now)
            };
            let _ = sender.send((report, points));
        });
        *self.stats_loading.borrow_mut() = Some(key);
        cx.spawn(async move |this, cx| {
            let (report, points) = loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                match result.try_recv() {
                    Ok(answer) => break answer,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Keep waiting only while this scan still owns the state.
                        let alive = this
                            .update(cx, |view, _| view.stats_loading.borrow().is_some())
                            .unwrap_or(false);
                        if !alive {
                            return;
                        }
                    }
                    // The worker died: nothing will arrive. Clearing the flag is
                    // what lets a later repaint start a fresh scan.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        this.update(cx, |view, cx| {
                            *view.stats_loading.borrow_mut() = None;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                }
            };
            this.update(cx, |view, cx| {
                *view.stats_loading.borrow_mut() = None;
                *view.stats_cache.borrow_mut() = Some((key, report, Rc::new(points)));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The requests the report on show was built from, once a scan has landed.
    fn stats_points_cached(&self) -> Option<Rc<Vec<StatsPoint>>> {
        self.stats_cache
            .borrow()
            .as_ref()
            .map(|(_, _, points)| points.clone())
    }

    /// 请求明细: every counted request in the window in the shared table, newest
    /// first. Sorting and paging keep a long window readable.
    ///
    /// The card's title and surface are the same ones every other card on the
    /// page uses, so it reads as part of the grid rather than as a separate
    /// panel. Each row names the question that opened it: a request list that
    /// showed only time, conversation and model made several recent requests in
    /// the same conversation look like one row repeated.
    fn render_request_table(
        &self,
        report: &StatsReport,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.prepare_table("stats-requests", 8, Some((0, true)));
        let columns = [
            Column::text("时间", 96.),
            Column::text("对话", 96.),
            Column::fill("请求"),
            Column::text("模型", 120.),
            Column::count("计费", 64.),
            Column::count("上下文", 64.),
            Column::count("思考", 56.),
            Column::count("工具", 56.),
        ];
        // The scan keeps every request of the conversations that changed in the
        // window, so the table trims to the window itself instead of listing
        // lines the cards above did not count.
        let end_ms = report.start_ms.saturating_add(
            self.stats_range
                .bucket_ms()
                .saturating_mul(self.stats_range.buckets() as u64),
        );
        let mut rows: Vec<Row> = Vec::new();
        for (index, point) in self
            .stats_points_cached()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            if point.at_ms < report.start_ms || point.at_ms >= end_ms {
                continue;
            }
            let found = self.chats.iter().find(|chat| chat.dir == point.chat);
            let title = found.map_or_else(
                || {
                    if point.title.trim().is_empty() {
                        point.chat.clone()
                    } else {
                        point.title.clone()
                    }
                },
                |chat| chat.title.clone(),
            );
            let label = if point.estimated {
                format!("≈ {}", ago(point.at_ms))
            } else {
                ago(point.at_ms)
            };
            // The transcript names the question exactly; the stored label is
            // the fallback for a turn its history has already dropped. The turn
            // number keeps a row apart even when neither is known.
            let asked = found
                .and_then(|chat| question_of(chat, point.turn))
                .filter(|question| !question.trim().is_empty())
                .unwrap_or_else(|| point.question.clone());
            let question = if asked.trim().is_empty() {
                format!("#{}", point.turn.max(1))
            } else {
                format!("#{} {}", point.turn.max(1), asked.trim())
            };
            rows.push(Row::new(
                format!("request-{index}"),
                vec![
                    Value::At(point.at_ms, label),
                    Value::Text(title),
                    Value::Text(question),
                    Value::Muted(if point.model.is_empty() {
                        "—".to_string()
                    } else {
                        point.model.clone()
                    }),
                    Value::Count(point.tokens),
                    Value::Count(point.context),
                    if point.reasoning_tokens == 0 {
                        Value::Faint("—".to_string())
                    } else {
                        Value::Count(point.reasoning_tokens)
                    },
                    if point.tools == 0 {
                        Value::Faint("—".to_string())
                    } else {
                        Value::Count(point.tools)
                    },
                ],
            ));
        }
        let table = self.render_table(
            "stats-requests",
            &columns,
            rows,
            Some(px(232.)),
            None,
            palette,
            cx,
        );
        card_surface(palette)
            .w_full()
            .child(card_heading(
                "请求明细",
                "最近 30 天 · 点表头排序 · Shift 连选",
                palette,
            ))
            .child(table)
            .into_any_element()
    }

    /// The usage statistics page: one card per figure, each with a chart.
    pub(crate) fn render_stats(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let report = self.stats_snapshot(cx);
        let mut chips: Vec<AnyElement> = Vec::new();
        for (index, range) in StatsRange::ALL.into_iter().enumerate() {
            chips.push(range_chip(
                index,
                range,
                self.stats_range == range,
                palette,
                cx,
            ));
        }
        let range_row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .gap_2()
            .px_6()
            .children(chips);
        let tools_average = report.tools.checked_div(report.requests).unwrap_or(0);
        let totals = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .justify_center()
            .gap_3()
            .w_full()
            .min_w(px(0.))
            .child(summary_tile(
                "请求总数",
                short_count(report.requests),
                format!("{} 个对话有请求", report.chats),
                palette.accent,
                palette,
            ))
            .child(summary_tile(
                "Token 总数",
                full_count(report.tokens),
                format!(
                    "提示 {} · 平均每次 {}",
                    short_count(report.prompt),
                    short_count(report.average_tokens())
                ),
                palette.sky,
                palette,
            ))
            .child(summary_tile(
                "工具调用量",
                short_count(report.tools),
                format!("每问平均 {tools_average} 次"),
                palette.warn,
                palette,
            ))
            // A cache share needs hits to have been reported, so the tile
            // appears on its own rather than as a permanent zero.
            .when_some(report.cache_rate(), |this, rate| {
                this.child(summary_tile(
                    "缓存命中",
                    share_text(true, rate),
                    format!(
                        "命中 {} / 提示 {}",
                        short_count(report.cache_hits),
                        short_count(report.prompt_tokens)
                    ),
                    palette.ok,
                    palette,
                ))
            })
            // Thinking is billed as output but never rides in a prompt, so the
            // window keeps it as its own figure beside the token total.
            .when(report.reasoning_tokens > 0, |this| {
                this.child(summary_tile(
                    "思考 token",
                    short_count(report.reasoning_tokens),
                    if report.tokens > 0 {
                        format!(
                            "占输出 {}",
                            share_text(true, report.reasoning_tokens as f64 / report.tokens as f64)
                        )
                    } else {
                        "各轮估算合计".to_string()
                    },
                    palette.sky,
                    palette,
                ))
            })
            // Server-side searching costs tokens like anything else, so the
            // window keeps its own count; the tile appears only once a
            // provider actually searched.
            .when(report.web_searches > 0, |this| {
                this.child(summary_tile(
                    "联网搜索",
                    format!("{} 次", full_count(report.web_searches)),
                    format!("读页 {} 次", full_count(report.web_fetches)),
                    palette.running,
                    palette,
                ))
            });
        // The chart cards go two per row, each pair sharing width and height so the
        // band reads as a grid rather than a wrapping strip; the tools card gets a
        // row of its own under them.
        let chart_rows = div()
            .flex()
            .flex_col()
            .gap_3()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap_3()
                    .w_full()
                    .min_w(px(0.))
                    .child(requests_card(&report, self.stats_range, palette))
                    .child(chats_card(&report, self.stats_range, palette)),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap_3()
                    .w_full()
                    .min_w(px(0.))
                    .child(average_card(&report, self.stats_range, palette))
                    .child(models_card(&report, palette)),
            )
            // Which tools did the work is the second half of the same question the
            // model card answers, so it gets its own row under it.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap_3()
                    .w_full()
                    .min_w(px(0.))
                    .child(tools_card(&report, palette)),
            );
        let hint = (report.requests == 0).then(|| {
            div()
                .px_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(
                    "这段时间没有记录到请求；问一次之后，这里会按时间汇总请求次数、token 与工具调用。",
                )
        });
        let note = if report.estimated {
            "数据来自持久统计账本；历史迁移记录按对话最后更新时间估算，次数可能偏少。请求明细保留 30 天，日汇总长期保留。"
        } else {
            "数据来自持久统计账本；时间取回答到达的那一刻，窗口按 UTC 对齐。请求明细保留 30 天，日汇总长期保留。"
        };
        let mut content = div()
            .w_full()
            .max_w(px(STATS_AREA_WIDTH))
            .flex()
            .flex_col()
            .gap_3()
            .min_w(px(0.))
            .child(coverage_line(&report, self.stats_range, palette));
        if let Some(hint) = hint {
            content = content.child(hint);
        }
        content = content
            .child(totals)
            .child(tokens_card(&report, self.stats_range, palette))
            .child(chart_rows)
            .child(self.render_request_table(&report, palette, cx))
            .child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child(note),
            );
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .child(page_title("统计", Vec::new(), palette))
            .child(range_row)
            .child(
                // Same scrolling shape as the context panel: one scroll
                // element directly under the page header, and its only child
                // is a non-shrinking row whose padding belongs to the content.
                // Without `flex_shrink_0` the row is squeezed to the viewport
                // and the scroll range no longer matches what is on screen,
                // which is what let this page keep sliding at the bottom.
                div()
                    .id("stats-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .track_scroll(&self.stats_scroll)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_center()
                            .flex_shrink_0()
                            .w_full()
                            .min_w(px(0.))
                            .px_6()
                            .pt_4()
                            .pb(px(STATS_BOTTOM_GAP))
                            .child(content),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One request, as a chart point.
    fn point(
        at_ms: u64,
        chat: &str,
        model: &str,
        tokens: u64,
        context: u64,
        tools: u64,
    ) -> StatsPoint {
        StatsPoint {
            at_ms,
            chat: chat.to_string(),
            title: chat.to_string(),
            turn: 0,
            question: String::new(),
            model: model.to_string(),
            tokens,
            context,
            tools,
            tool_calls: BTreeMap::new(),
            cache_hits: 0,
            prompt_tokens: 0,
            model_ms: 0,
            tool_ms: 0,
            web_searches: 0,
            web_fetches: 0,
            reasoning_tokens: 0,
            estimated: false,
        }
    }

    #[test]
    fn a_summary_tile_has_room_for_its_note() {
        // A line box defaults to the golden ratio of the font size, and the tile
        // adds 16px of padding on both sides plus two 8px gaps. The note is the
        // only child that truncates, so a short tile silently squeezes it away
        // instead of pushing the card taller.
        let lines: f32 = [16., 30., 12.]
            .into_iter()
            .map(|font| (font * std::f32::consts::GOLDEN_RATIO).round())
            .sum();
        let needed = 16. * 2. + 8. * 2. + lines;
        assert!(
            STATS_TILE_HEIGHT >= needed,
            "the note needs {needed}px, the tile gives it {STATS_TILE_HEIGHT}px"
        );
    }

    #[test]
    fn full_counts_keep_every_digit() {
        assert_eq!(full_count(0), "0");
        assert_eq!(full_count(999), "999");
        assert_eq!(full_count(123_456_789), "123456789");
    }

    #[test]
    fn windows_line_up_with_their_buckets() {
        let now = 1_000 * DAY_MS + 5 * HOUR_MS;
        let (start, end) = StatsRange::Week.window(now);
        assert_eq!(end - start, 7 * DAY_MS);
        assert_eq!(start, 994 * DAY_MS);
        assert_eq!(end, 1_001 * DAY_MS);
        assert_eq!(StatsRange::Week.bucket_of(now, start), Some(6));
        // A moment before the window and the first moment past it belong to no
        // bar at all, so an old request cannot be folded into the first one.
        assert_eq!(StatsRange::Week.bucket_of(start - 1, start), None);
        assert_eq!(StatsRange::Week.bucket_of(end, start), None);

        let (day_start, day_end) = StatsRange::Day.window(now);
        assert_eq!((day_end - day_start) / HOUR_MS, 24);
        assert_eq!(StatsRange::Day.buckets(), 12);
        assert_eq!(StatsRange::Day.bucket_of(day_start, day_start), Some(0));
        assert_eq!(
            StatsRange::Day.bucket_of(day_start + 3 * HOUR_MS, day_start),
            Some(1)
        );
        assert_eq!(StatsRange::Quarter.buckets(), 13);
        assert_eq!(StatsRange::Quarter.bucket_ms(), 7 * DAY_MS);
    }

    #[test]
    fn labels_read_as_hours_and_utc_dates() {
        // 2026-11-23 is day 20 780 since the epoch.
        assert_eq!(StatsRange::Week.bucket_label(20_780 * DAY_MS), "11-23");
        assert_eq!(StatsRange::Month.bucket_label(20_780 * DAY_MS), "11-23");
        assert_eq!(
            StatsRange::Day.bucket_label(20_780 * DAY_MS + 8 * HOUR_MS),
            "08–10"
        );
        // The last bar of a day wraps around midnight rather than printing 24.
        assert_eq!(
            StatsRange::Day.bucket_label(20_780 * DAY_MS + 22 * HOUR_MS),
            "22–00"
        );
    }

    #[test]
    fn tick_labels_stay_readable() {
        let start = 20_780 * DAY_MS;
        let day = StatsRange::Day.tick_labels(start);
        assert_eq!(day.len(), 12);
        assert_eq!(day[0], "00");
        assert_eq!(day[11], "22");
        let week = StatsRange::Week.tick_labels(start);
        assert_eq!(week.len(), 7);
        assert_eq!(week[0], "11-23");
        assert_eq!(week[6], "11-29");
        // Too many bars to name each one: the card's own heading says which
        // window the bars cover.
        assert!(StatsRange::Month.tick_labels(start).is_empty());
        assert!(StatsRange::Quarter.tick_labels(start).is_empty());
    }

    #[test]
    fn the_report_adds_up_requests_tokens_and_models() {
        let now = 20_780 * DAY_MS + 12 * HOUR_MS;
        let range = StatsRange::Week;
        let (start, _) = range.window(now);
        let points = vec![
            point(start + HOUR_MS, "chat-1", "m1", 100, 80, 2),
            point(start + 2 * HOUR_MS, "chat-1", "m1", 50, 40, 0),
            point(start + 3 * DAY_MS, "chat-2", "m2", 200, 150, 3),
            point(start + 6 * DAY_MS, "chat-1", "", 25, 20, 0),
            // Older than the window, and later than it: neither is counted.
            point(start - 1, "chat-3", "m1", 900, 900, 9),
            point(now + DAY_MS, "chat-3", "m1", 900, 900, 9),
        ];
        let report = stats_report(&points, range, now);
        assert_eq!(report.requests, 4);
        assert_eq!(report.tokens, 375);
        assert_eq!(report.tools, 5);
        assert_eq!(report.prompt, 290);
        assert_eq!(report.chats, 2);
        assert_eq!(report.average_tokens(), 93);
        assert_eq!(report.buckets.len(), 7);
        assert_eq!(report.buckets[0].requests, 2);
        assert_eq!(report.buckets[0].tokens, 150);
        assert_eq!(report.buckets[0].chats, 1);
        assert_eq!(report.buckets[3].requests, 1);
        assert_eq!(report.buckets[6].requests, 1);
        assert_eq!(report.buckets[6].chats, 1);
        assert_eq!(report.models.len(), 3);
        assert_eq!(report.models[0].model, "m2");
        assert_eq!(report.models[0].tokens, 200);
        assert_eq!(report.models[1].model, "m1");
        assert_eq!(report.models[1].requests, 2);
        assert_eq!(report.models[2].model, "未知模型");

        let empty = stats_report(&[], StatsRange::Day, now);
        assert_eq!(empty.requests, 0);
        assert_eq!(empty.tools, 0);
        assert_eq!(empty.average_tokens(), 0);
        // Nothing reported a cache or a clock, so neither figure is invented.
        assert!(empty.cache_rate().is_none());
        assert_eq!(empty.model_ms, 0);
        assert_eq!(empty.tool_ms, 0);
        assert!(empty.models.is_empty());
        assert_eq!(empty.buckets.len(), 12);
    }

    #[test]
    fn the_report_sums_cache_and_time_with_the_tokens() {
        let now = 20_780 * DAY_MS + 12 * HOUR_MS;
        let range = StatsRange::Week;
        let (start, _) = range.window(now);
        let mut first = point(start + HOUR_MS, "chat-1", "m1", 100, 80, 1);
        first.cache_hits = 640;
        first.prompt_tokens = 1_000;
        first.model_ms = 2_100;
        first.tool_ms = 640;
        first.reasoning_tokens = 1_200;
        first.web_searches = 2;
        first.web_fetches = 1;
        let mut second = point(start + 2 * HOUR_MS, "chat-2", "m1", 50, 30, 0);
        second.model_ms = 400;
        // A request from before the fields existed contributes no cache and no
        // clock, so it cannot drag the share down.
        let older = point(start + 3 * HOUR_MS, "chat-3", "m2", 20, 10, 0);
        let report = stats_report(&[first, second, older], range, now);
        assert_eq!(report.cache_hits, 640);
        assert_eq!(report.prompt_tokens, 1_000);
        assert_eq!(report.model_ms, 2_500);
        assert_eq!(report.tool_ms, 640);
        assert_eq!(report.reasoning_tokens, 1_200);
        assert_eq!(report.web_searches, 2);
        assert_eq!(report.web_fetches, 1);
        let rate = report.cache_rate().expect("hits were reported");
        assert!((rate - 0.64).abs() < 1e-6, "rate {rate}");
        // The same figures land on the model that produced them.
        let m1 = report
            .models
            .iter()
            .find(|model| model.model == "m1")
            .expect("m1 answered");
        assert_eq!(m1.cache_hits, 640);
        assert_eq!(m1.model_ms, 2_500);
    }
    #[test]
    fn a_conversation_without_requests_is_placed_at_its_last_change() {
        let now = 20_780 * DAY_MS + 12 * HOUR_MS;
        let mut chat = Chat::restored(
            "旧对话".to_string(),
            "chat-1".to_string(),
            vec![
                ChatMessage::user("问题"),
                ChatMessage::tool("read · a.txt", false),
                {
                    let mut answer = ChatMessage::assistant("回答");
                    answer.tokens = 300;
                    answer.context = 240;
                    answer.model = "old-model".to_string();
                    answer
                },
            ],
        );
        chat.updated_ms = now;
        // A data directory with no `chats/chat-1/requests.jsonl`.
        let missing =
            std::env::temp_dir().join(format!("dsh-flow-ui-stats-missing-{}", std::process::id()));
        let (start, _) = StatsRange::Week.window(now);
        let points = stats_points(&missing, std::slice::from_ref(&chat), start);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].at_ms, now);
        assert_eq!(points[0].tokens, 300);
        assert_eq!(points[0].context, 240);
        assert_eq!(points[0].tools, 1);
        assert_eq!(points[0].tool_calls.get("read"), Some(&1));
        assert_eq!(points[0].model, "old-model");
        // A fallback row still names the question that opened it.
        assert_eq!(points[0].turn, 1);
        assert_eq!(points[0].question, "问题");
        assert!(points[0].estimated);

        // A conversation untouched since before the window contributes nothing.
        let mut older = chat;
        older.updated_ms = start - 1;
        assert!(stats_points(&missing, &[older], start).is_empty());
    }

    #[test]
    fn tool_counts_add_up_and_rank_by_calls() {
        let now = 20_780 * DAY_MS + 9 * HOUR_MS;
        let mut first = point(now, "chat-a", "m", 10, 5, 3);
        first.tool_calls = BTreeMap::from([("read".to_string(), 2), ("grep".to_string(), 1)]);
        let mut second = point(now, "chat-b", "m", 10, 5, 2);
        second.tool_calls = BTreeMap::from([("read".to_string(), 1), ("shell".to_string(), 1)]);
        let report = stats_report(&[first, second], StatsRange::Day, now);
        assert_eq!(report.tools, 5, "the total still comes from the call count");
        assert_eq!(
            report.tool_calls,
            vec![
                ("read".to_string(), 3),
                ("grep".to_string(), 1),
                ("shell".to_string(), 1)
            ],
            "busiest first, then by name"
        );
        // A request written before names were kept adds no tool rows but still
        // counts toward the total.
        let bare = point(now, "chat-c", "m", 10, 5, 1);
        let report = stats_report(&[bare], StatsRange::Day, now);
        assert_eq!(report.tools, 1);
        assert!(report.tool_calls.is_empty());
    }

    #[test]
    fn a_request_counts_each_tool_it_called() {
        let names = vec![
            "read".to_string(),
            " ".to_string(),
            "grep".to_string(),
            "read".to_string(),
        ];
        let counts = counted_tools(&names);
        assert_eq!(counts.get("read"), Some(&2));
        assert_eq!(counts.get("grep"), Some(&1));
        assert_eq!(counts.len(), 2, "a blank name is not a tool");
        assert_eq!(
            ranked_tools(counts.into_iter().collect()),
            vec![("read".to_string(), 2), ("grep".to_string(), 1)]
        );
    }

    #[test]
    fn stored_requests_win_over_the_answers_fallback() {
        use crate::chats::{StoredRequest, StoredRequestItem, append_request};
        let dir =
            std::env::temp_dir().join(format!("dsh-flow-ui-stats-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let at = 20_780 * DAY_MS + 9 * HOUR_MS;
        let request = StoredRequest {
            turn: 1,
            at_ms: at,
            model: "new-model".to_string(),
            tokens: 42,
            context: 30,
            calls: 4,
            cache_hits: 0,
            prompt_tokens: 0,
            model_ms: 0,
            tool_ms: 0,
            web_searches: 0,
            web_fetches: 0,
            reasoning: String::new(),
            reasoning_tokens: 0,
            tool_names: vec!["read".to_string(), "grep".to_string(), "read".to_string()],
            items: vec![
                StoredRequestItem {
                    kind: ContextKind::User.key().to_string(),
                    label: "更早的问题".to_string(),
                    text: "更早的问题".to_string(),
                    tokens: 3,
                },
                StoredRequestItem {
                    kind: ContextKind::User.key().to_string(),
                    label: "这次的问题".to_string(),
                    text: "这次的问题".to_string(),
                    tokens: 4,
                },
            ],
        };
        append_request(&dir, "chat-1", &request).expect("append");
        let mut chat = Chat::restored(
            "对话".to_string(),
            "chat-1".to_string(),
            vec![ChatMessage::user("问题"), {
                let mut answer = ChatMessage::assistant("回答");
                answer.tokens = 999;
                answer
            }],
        );
        chat.updated_ms = at + 1;
        let (start, _) = StatsRange::Week.window(at);
        let points = stats_points(&dir, &[chat], start);
        assert_eq!(points.len(), 1, "one stored line, not the fallback answer");
        assert_eq!(points[0].tokens, 42);
        assert_eq!(points[0].tools, 4);
        assert_eq!(points[0].model, "new-model");
        // The last user piece is the question that opened this request; the
        // earlier one is replayed history and must not label the row.
        assert_eq!(points[0].turn, 1);
        assert_eq!(points[0].question, "这次的问题");
        assert!(!points[0].estimated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fallback_requests_keep_the_question_that_opened_each_one() {
        let now = 20_780 * DAY_MS + 12 * HOUR_MS;
        let mut chat = Chat::restored(
            "旧对话".to_string(),
            "chat-1".to_string(),
            vec![
                ChatMessage::user("第一个问题"),
                ChatMessage::assistant("第一个回答"),
                ChatMessage::user("第二个问题"),
                ChatMessage::assistant("第二个回答"),
            ],
        );
        chat.updated_ms = now;
        let missing =
            std::env::temp_dir().join(format!("dsh-flow-ui-stats-missing-{}", std::process::id()));
        let (start, _) = StatsRange::Week.window(now);
        let points = stats_points(&missing, std::slice::from_ref(&chat), start);
        assert_eq!(points.len(), 2);
        // Both fallback rows share the conversation's last-change time, so the
        // question and turn are what keep them from reading as one request.
        assert_eq!(points[0].at_ms, points[1].at_ms);
        assert_eq!(points[0].turn, 1);
        assert_eq!(points[0].question, "第一个问题");
        assert_eq!(points[1].turn, 2);
        assert_eq!(points[1].question, "第二个问题");
    }
    #[test]
    fn daily_rollups_supply_quarter_cards() {
        let now = 20_780 * DAY_MS + 12 * HOUR_MS;
        let range = StatsRange::Quarter;
        let (start, _) = range.window(now);
        let mut first = crate::stats_store::DayRollup {
            day_ms: start,
            requests: 2,
            tokens: 100,
            prompt: 80,
            tools: 3,
            chats: vec!["chat-1".to_string()],
            ..crate::stats_store::DayRollup::default()
        };
        first.models.insert(
            "m1".to_string(),
            crate::stats_store::ModelCount {
                requests: 2,
                tokens: 100,
                cache_hits: 0,
                prompt_tokens: 0,
                model_ms: 0,
                tool_ms: 0,
            },
        );
        let mut second = crate::stats_store::DayRollup {
            day_ms: start + 2 * DAY_MS,
            requests: 1,
            tokens: 50,
            prompt: 40,
            tools: 1,
            chats: vec!["chat-1".to_string(), "chat-2".to_string()],
            ..crate::stats_store::DayRollup::default()
        };
        second.models.insert(
            "m2".to_string(),
            crate::stats_store::ModelCount {
                requests: 1,
                tokens: 50,
                cache_hits: 0,
                prompt_tokens: 0,
                model_ms: 0,
                tool_ms: 0,
            },
        );
        let report = report_from_daily(&[first, second], range, now);
        assert_eq!(report.requests, 3);
        assert_eq!(report.tokens, 150);
        assert_eq!(report.prompt, 120);
        assert_eq!(report.tools, 4);
        assert_eq!(report.chats, 2);
        assert_eq!(report.average_tokens(), 50);
        assert_eq!(report.buckets[0].chats, 2);
        assert_eq!(report.models.len(), 2);
    }
}

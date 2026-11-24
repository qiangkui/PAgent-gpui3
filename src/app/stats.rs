//! The statistics page: what the assistant was asked over a chosen window.
//!
//! Every card reads from one series: the per-request lines in each
//! conversation's `requests.jsonl`. Each line carries the time the answer
//! arrived, the model that wrote it and the tokens the endpoint billed, which is
//! exactly what "requests per day" and "tokens per hour" need. A conversation
//! saved before that file existed falls back to its answers' totals at the time
//! the conversation was last touched, so older work is not silently missing.

use super::*;
use std::collections::{HashMap, HashSet};

/// One hour, in milliseconds.
const HOUR_MS: u64 = 3_600_000;
/// One day, in milliseconds.
const DAY_MS: u64 = 86_400_000;
/// Widest the card area grows; a wider pane leaves equal gutters on both sides.
const STATS_AREA_WIDTH: f32 = 822.;
/// Width of one summary tile: three tiles and two gaps fill the area exactly.
const STATS_TILE_WIDTH: f32 = 266.;
/// Width of one chart card: two cards and one gap fill the area exactly.
const STATS_CARD_WIDTH: f32 = 405.;
/// Height of the three summary tiles on the first row.
const STATS_TILE_HEIGHT: f32 = 128.;
/// Height of a chart card placed two per row, so the pair always matches.
const STATS_CARD_HEIGHT: f32 = 260.;
/// Height of the full-width Token 用量 card.
const STATS_LONG_HEIGHT: f32 = 320.;
/// Gap kept below the last element inside the scroll content, so the stop
/// point leaves that distance visible instead of pulling the content flush.
const STATS_BOTTOM_GAP: f32 = 16.;
/// Plot area of a chart card's bars.
const CHART_HEIGHT: f32 = 84.;
/// Equal horizontal bands the chart's grid rules split the plot into.
const CHART_BANDS: usize = 4;
/// Plot area of the full-width token chart.
const LONG_CHART_HEIGHT: f32 = 150.;
/// Models listed on the distribution card before the rest are folded away.
const MODELS_SHOWN: usize = 5;

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

/// What a cached report was built from: the window, the conversations and the
/// bucket `now` falls in. Any change to one of them rebuilds the report; a
/// bucket boundary moves it forward on its own, without waiting for a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StatsKey {
    /// Window on show.
    pub(crate) range: StatsRange,
    /// How many conversations the report was built over.
    pub(crate) chats: usize,
    /// Sum of their last-change times, so a new answer invalidates it.
    pub(crate) updated: u64,
    /// Sum of their message counts, so an edit that keeps the time counts too.
    pub(crate) messages: u64,
    /// Which bucket `now` fell in when the report was built.
    pub(crate) bucket: u64,
}

/// One answered request, as the charts count it.
pub(crate) struct StatsPoint {
    /// When the answer arrived, in epoch milliseconds.
    pub(crate) at_ms: u64,
    /// Folder of the conversation it belongs to.
    pub(crate) chat: String,
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
    /// Conversations that answered at least once in the window.
    pub(crate) chats: u64,
    /// Models, busiest first.
    pub(crate) models: Vec<ModelShare>,
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
}

/// What a background scan needs from one conversation.
///
/// A conversation with a request file is fully described by its folder and its
/// last change; only one without that file needs its transcript, because its
/// figures live on the answers themselves.
pub(crate) struct StatsJob {
    /// Folder of the conversation.
    pub(crate) dir: String,
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
                    at_ms: request.at_ms,
                    chat: chat.dir.clone(),
                    turn: request.turn,
                    question: request_question(request),
                    model: request.model.clone(),
                    tokens: request.tokens,
                    context: request.context,
                    tools,
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
        let mut pending_question = String::new();
        let mut turn = 0u64;
        for message in &chat.messages {
            match message.role {
                Role::Tool => {
                    pending_tools = pending_tools.saturating_add(1);
                }
                Role::User | Role::Attachment => {
                    pending_tools = 0;
                    pending_question = turn_label(&message.text);
                }
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
                    let model = if message.model.is_empty() {
                        newest.clone()
                    } else {
                        message.model.clone()
                    };
                    points.push(StatsPoint {
                        at_ms: chat.updated_ms,
                        chat: chat.dir.clone(),
                        turn,
                        question: std::mem::take(&mut pending_question),
                        model,
                        tokens: message.tokens,
                        context: message.context,
                        tools,
                        estimated: true,
                    });
                }
                Role::Note | Role::Approval => {}
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
    let mut models: HashMap<String, ModelShare> = HashMap::new();
    let mut report = StatsReport {
        buckets: Vec::new(),
        requests: 0,
        tokens: 0,
        tools: 0,
        prompt: 0,
        chats: 0,
        models: Vec::new(),
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
        report.prompt = report.prompt.saturating_add(point.context);
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
        });
        entry.requests = entry.requests.saturating_add(1);
        entry.tokens = entry.tokens.saturating_add(point.tokens);
    }
    for (index, filled) in seen.iter().enumerate() {
        buckets[index].chats = filled.len() as u64;
    }
    report.buckets = buckets;
    report.chats = chats.len() as u64;
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
impl DshFlowApp {
    /// Switch the window the page covers.
    pub(crate) fn set_stats_range(&mut self, range: StatsRange, cx: &mut Context<Self>) {
        if self.stats_range != range {
            self.stats_range = range;
            *self.stats_cache.borrow_mut() = None;
            cx.notify();
        }
    }

    /// The report for the window on show, rebuilt only when the conversations
    /// or the bucket `now` falls in have changed.
    ///
    /// Building it reads every conversation's request file, which is far too
    /// much to do on a repaint once the history has grown: the read happens on a
    /// worker thread, and the page shows the previous report until it lands.
    pub(crate) fn stats_snapshot(&self, cx: &mut Context<Self>) -> StatsReport {
        let now = now_ms();
        let key = StatsKey {
            range: self.stats_range,
            chats: self.chats.len(),
            updated: self
                .chats
                .iter()
                .fold(0u64, |sum, chat| sum.saturating_add(chat.updated_ms)),
            messages: self
                .chats
                .iter()
                .map(|chat| chat.messages.len() as u64)
                .sum(),
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
                updated_ms: chat.updated_ms,
                messages: if crate::chats::has_requests(data, &chat.dir) {
                    Vec::new()
                } else {
                    chat.messages.clone()
                },
            })
            .collect()
    }

    /// Read the history on a worker thread and store the report when it arrives.
    fn schedule_stats(&self, now: u64, key: StatsKey, cx: &mut Context<Self>) {
        let data = self.ai.data_dir();
        let chats = self.stats_jobs(&data);
        let range = self.stats_range;
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (start, _) = range.window(now);
            let points = stats_points_for(&data, &chats, start);
            let report = stats_report(&points, range, now);
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
        for (index, point) in self.stats_points_cached().unwrap_or_default().iter().enumerate() {
            if point.at_ms < report.start_ms || point.at_ms >= end_ms {
                continue;
            }
            let found = self.chats.iter().find(|chat| chat.dir == point.chat);
            let title = found.map_or_else(|| point.chat.clone(), |chat| chat.title.clone());
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
                "点表头排序 · Shift 连选",
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
            ));
        // The four chart cards go two per row, each pair sharing width and
        // height so the band reads as a grid rather than a wrapping strip.
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
            "数据来自每个对话的 requests.jsonl；更早的对话没有请求明细，按对话最后更新时间估算，次数可能偏少。"
        } else {
            "数据来自每个对话的 requests.jsonl；时间取回答到达的那一刻，窗口按 UTC 对齐。"
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

/// One chip of the range picker; the chosen one is filled with the accent.
fn range_chip(
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
fn coverage_line(report: &StatsReport, range: StatsRange, palette: Palette) -> Div {
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
fn stamp_label(ms: u64) -> String {
    let (_, month, day) = civil_from_days((ms / DAY_MS) as i64);
    let hour = (ms / HOUR_MS) % 24;
    format!("{month:02}-{day:02} {hour:02}:00")
}

/// The surface every card shares: the panel colour and a soft shadow, with no
/// border. Width and height are left to the row the card is placed in.
fn card_surface(palette: Palette) -> Div {
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
fn chart_card(palette: Palette) -> Div {
    card_surface(palette)
        .w(px(STATS_CARD_WIDTH))
        .h(px(STATS_CARD_HEIGHT))
}

/// The token chart's card: one full row of its own.
fn long_card(palette: Palette) -> Div {
    card_surface(palette).w_full().h(px(STATS_LONG_HEIGHT))
}

/// One of the three totals on the first row: a label, a big figure and a note.
///
/// The three tiles split their row equally, so the three headline figures read
/// as one band rather than as three differently sized cards.
fn summary_tile(
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
fn card_heading(title: &str, note: &str, palette: Palette) -> Div {
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
fn card_value(value: String, unit: &str, palette: Palette) -> Div {
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
fn full_count(count: u64) -> String {
    count.to_string()
}

/// One faint line under a chart: its peak, or what the bars add up to.
fn chart_foot(text: String, palette: Palette) -> Div {
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
fn bar_chart(
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
fn requests_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
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
fn tokens_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
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
fn chats_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
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
fn average_card(report: &StatsReport, range: StatsRange, palette: Palette) -> AnyElement {
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
fn models_card(report: &StatsReport, palette: Palette) -> AnyElement {
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
                                .child(format!(
                                    "{} 次 · {}",
                                    short_count(model.requests),
                                    short_count(model.tokens)
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
            turn: 0,
            question: String::new(),
            model: model.to_string(),
            tokens,
            context,
            tools,
            estimated: false,
        }
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
        assert!(empty.models.is_empty());
        assert_eq!(empty.buckets.len(), 12);
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
}

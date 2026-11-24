//! The 上下文 tab: one answered request and its rounds, counted by category.
//!
//! A request is a run of model round trips, and every round sends the whole
//! prompt again with the tool results produced so far. The request's figures
//! are therefore the sum of its rounds, while each round can be opened on its
//! own to read the pieces that round actually sent.
//!
//! Everything here is measured from the same pieces the window itself sends
//! (`prompt_blocks`, the history it replays, the tool schemas the engine
//! reports), so a category's number moves when the thing it names changes
//! instead of drifting from a copy of the prompt kept for display.

use super::*;

/// How many answers the growth chart plots.
const GROWTH_POINTS: usize = 12;

/// How many answered questions the detail table lists, newest first.
const REQUEST_ROWS: usize = 30;

/// Diameter of the ring that shows the whole conversation's consumption.
const RING_SIZE: f32 = 148.0;

/// Width of the band the ring paints.
const RING_THICKNESS: f32 = 18.0;

/// Width of the narrow ring card beside the totals table.
const TOTALS_CHART_WIDTH: f32 = 300.0;
/// Height the ring card and the totals table share.
const TOTALS_CARD_HEIGHT: f32 = 280.0;

/// One answered question, as the panel's detail table lists it.
#[derive(Clone)]
pub(crate) struct RequestRow {
    /// Which question it was in this conversation, counting from one.
    pub(crate) turn: u64,
    /// Model that answered.
    pub(crate) model: String,
    /// How long the whole question took, in milliseconds.
    pub(crate) elapsed_ms: u64,
    /// Tokens the endpoint billed: every tool round of that question, added up.
    pub(crate) tokens: u64,
    /// Prompt tokens the endpoint reported for its rounds, added up; zero when
    /// it reported no per-round counts.
    pub(crate) prompt_tokens: u64,
    /// Answer tokens the endpoint reported for its rounds, added up; zero when
    /// it reported no per-round counts.
    pub(crate) answer_tokens: u64,
    /// Prompt + answer of its last round, which is what filled the window.
    pub(crate) context: u64,
    /// Model round trips it took, the answer included; zero when unknown.
    pub(crate) rounds: u32,
    /// What each round cost, oldest first, as `(prompt, completion, tool calls)`;
    /// empty for answers stored before the engine reported it.
    pub(crate) round_tokens: Vec<(u64, u64, u32)>,
    /// Tool calls made while answering it.
    pub(crate) tools: u64,
    /// What that request was made of, in [`ContextKind::ALL`] order, with every
    /// round's estimate added up; empty for answers saved before it was
    /// recorded.
    pub(crate) composition: Vec<u32>,
    /// What each round of the request was made of, oldest first, as raw
    /// estimates in [`ContextKind::ALL`] order. One entry when the answer has
    /// no per-round figures; empty when nothing was recorded.
    pub(crate) round_composition: Vec<Vec<usize>>,
}

/// Which tab the conversation pane shows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatTab {
    /// The transcript and its composer.
    Chat,
    /// The breakdown of the next request.
    Context,
    /// The conversation's background tasks.
    Tasks,
}

/// Where one block of the system prompt came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptKind {
    /// Rules the window always sends.
    System,
    /// Everything added on top: project instructions, maps, summaries, the
    /// previous turn's tool recap.
    Injected,
    /// Instructions from the skill folders switched on in the Skill page.
    Skills,
}

/// One labelled, separately counted piece of the system prompt.
pub(crate) struct PromptBlock {
    /// Which panel category it belongs to.
    pub(crate) kind: PromptKind,
    /// Short name shown above the block's text.
    pub(crate) label: &'static str,
    /// Separator that goes before it, so joining matches what was sent before
    /// this breakdown existed.
    pub(crate) sep: &'static str,
    /// The text itself.
    pub(crate) text: String,
}

/// Put the blocks back into the single string the engine receives.
pub(crate) fn joined_prompt(blocks: &[PromptBlock]) -> String {
    let mut joined = String::new();
    for block in blocks.iter().filter(|block| !block.text.is_empty()) {
        if !joined.is_empty() {
            joined.push_str(block.sep);
        }
        joined.push_str(&block.text);
    }
    joined
}

/// One category of the next request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextKind {
    /// The rules and the environment line.
    System,
    /// The tool schemas the engine declares.
    Tools,
    /// What the person asked.
    User,
    /// Project instructions, the workspace map, the early-turns summary.
    Injected,
    /// Prompt text from switched-on skill folders.
    Skills,
    /// What the assistant said.
    Assistant,
    /// What the previous turn's tools returned.
    ToolResults,
    /// What the model wrote back as that round's output.
    Output,
}

impl ContextKind {
    /// Every category, in the order the panel lists them.
    pub(crate) const ALL: [Self; 8] = [
        Self::System,
        Self::Tools,
        Self::User,
        Self::Injected,
        Self::Skills,
        Self::Assistant,
        Self::ToolResults,
        Self::Output,
    ];

    /// The name the panel shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "系统提示词",
            Self::Tools => "工具定义",
            Self::User => "用户消息",
            Self::Injected => "注入内容",
            Self::Skills => "注入技能",
            Self::Assistant => "助手消息",
            Self::ToolResults => "工具结果",
            Self::Output => "输出文字",
        }
    }

    /// The chart colour: the same in both themes, because it names a category
    /// rather than a surface.
    pub(crate) fn color(self) -> u32 {
        match self {
            Self::System => 0x006b_7fd7,
            Self::Tools => 0x00e0_8a2c,
            Self::User => 0x002f_9e5f,
            Self::Injected => 0x00a8_55f7,
            Self::Skills => 0x00ef_6c2f,
            Self::Assistant => 0x003b_82f6,
            Self::ToolResults => 0x0014_a3a3,
            Self::Output => 0x00d9_4a8b,
        }
    }

    /// Anything to say about a category this build never fills.
    pub(crate) fn empty_note(self) -> Option<&'static str> {
        match self {
            Self::Skills => Some("没有启用的技能；在左侧 Skill 页打开开关后，这里会出现注入内容"),
            _ => None,
        }
    }

    /// The name this category is stored under in `requests.jsonl`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Tools => "tools",
            Self::User => "user",
            Self::Injected => "injected",
            Self::Skills => "skills",
            Self::Assistant => "assistant",
            Self::ToolResults => "toolResults",
            Self::Output => "output",
        }
    }

    /// Read a stored category back; an unknown key is skipped rather than
    /// guessed at.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// One countable piece of a category, with the text that can be read.
pub(crate) struct ContextItem {
    /// What it is: a block name, a tool name, a turn's first line.
    pub(crate) label: String,
    /// The whole text that goes to the model.
    pub(crate) text: String,
    /// Estimated tokens for this piece.
    pub(crate) tokens: usize,
}

/// One category of one request.
pub(crate) struct ContextPart {
    /// Which category this is.
    pub(crate) kind: ContextKind,
    /// The pieces it is made of.
    pub(crate) items: Vec<ContextItem>,
    /// Estimated tokens for the whole category.
    pub(crate) tokens: usize,
}

/// One counted piece of a request, kept so a past request can be opened again.
#[derive(Clone)]
pub(crate) struct RequestItem {
    /// Which category it belongs to.
    pub(crate) kind: ContextKind,
    /// What it is: a block name, a tool name, a turn's first line.
    pub(crate) label: String,
    /// The whole text that went to the model.
    pub(crate) text: String,
    /// Estimated tokens for this piece.
    pub(crate) tokens: usize,
}

/// What a conversation holds, for the 上下文 tab.
#[derive(Default)]
pub(crate) struct ContextReport {
    /// The model's window, for the percentage.
    pub(crate) window: u64,
    /// Questions asked in this conversation.
    pub(crate) asked: u64,
    /// Estimated tokens of the replayed history alone.
    pub(crate) history_tokens: usize,
    /// Tool calls recorded in this conversation.
    pub(crate) steps: u64,
    /// Answers produced in this conversation.
    pub(crate) replies: u64,
    /// Room kept for the answer, from the connection's token cap.
    pub(crate) output: u64,
    /// Estimated tokens of history the engine replays before it starts
    /// dropping the oldest turns.
    pub(crate) history_budget: usize,
    /// What each earlier answer carried, oldest first, for the growth chart.
    pub(crate) growth: Vec<u64>,
    /// Tokens the endpoint billed over the whole conversation.
    pub(crate) spend: u64,
    /// Every request's pieces added up, in [`ContextKind::ALL`] order; drives
    /// the whole-conversation ring.
    pub(crate) total_composition: Vec<usize>,
    /// Every round's prompt the endpoint reported, over the whole conversation.
    pub(crate) prompt_total: u64,
    /// Every round's answer the endpoint reported, over the whole conversation.
    pub(crate) answer_total: u64,
    /// Billed tokens no round accounted for, over the whole conversation: the
    /// answers whose endpoint reported a total but no per-round counts (an
    /// older record, or a question sent with the tools switched off).
    pub(crate) unreported: u64,
    /// The answered questions, newest first, for the request list.
    pub(crate) requests: Vec<RequestRow>,
}
impl ContextReport {
    /// True when the replayed history has reached the trimming budget.
    pub(crate) fn history_full(&self) -> bool {
        self.history_budget > 0 && self.history_tokens >= self.history_budget
    }
}

/// Spread a measured number of tokens over the parts' own proportions.
///
/// The parts are floored first and the leftovers go to the parts with the
/// largest remainders, so the parts always add up to `target` exactly instead
/// of drifting by a few tokens.
fn anchor_parts(tokens: &[usize], target: u64) -> Vec<usize> {
    let total: usize = tokens.iter().sum();
    if total == 0 || target == 0 {
        return tokens.to_vec();
    }
    let target = usize::try_from(target).unwrap_or(usize::MAX);
    let mut scaled: Vec<usize> = tokens
        .iter()
        .map(|tokens| target.saturating_mul(*tokens) / total)
        .collect();
    // Floors never overshoot; the tokens they leave over go to the parts with
    // the largest remainders, so the sum is `target` exactly.
    let mut remaining = target.saturating_sub(scaled.iter().sum());
    let mut order: Vec<usize> = (0..tokens.len()).collect();
    order.sort_by_key(|index| std::cmp::Reverse(target.saturating_mul(tokens[*index]) % total));
    for index in order {
        if remaining == 0 {
            break;
        }
        scaled[index] = scaled[index].saturating_add(1);
        remaining -= 1;
    }
    scaled
}

/// The context size of each answer, newest last, at most [`GROWTH_POINTS`].
fn context_growth(messages: &[ChatMessage]) -> Vec<u64> {
    let mut sizes: Vec<u64> = messages
        .iter()
        .filter(|message| message.role == Role::Assistant && message.context > 0)
        .map(|message| message.context)
        .collect();
    if sizes.len() > GROWTH_POINTS {
        sizes.drain(..sizes.len() - GROWTH_POINTS);
    }
    sizes
}

/// The text of the answer that closed one question, read from the transcript.
///
/// `turn` counts answers from one, the same way [`request_rows`] numbers them,
/// so a request saved before the output text was kept can still show it.
fn closing_answer(messages: &[ChatMessage], turn: u64) -> Option<String> {
    let index = usize::try_from(turn.checked_sub(1)?).ok()?;
    messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .nth(index)
        .map(|message| message.text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Which category a system-prompt block is counted under.
fn prompt_block_kind(kind: PromptKind) -> ContextKind {
    match kind {
        PromptKind::System => ContextKind::System,
        PromptKind::Injected => ContextKind::Injected,
        PromptKind::Skills => ContextKind::Skills,
    }
}

/// Which category one replayed turn is counted under.
fn turn_kind(speaker: Speaker) -> ContextKind {
    match speaker {
        Speaker::User => ContextKind::User,
        Speaker::Assistant => ContextKind::Assistant,
    }
}

/// A cheap fingerprint of one conversation's tool schemas.
///
/// The panel rebuilds its breakdown when the schemas change; hashing the names
/// and the length of each schema is enough to notice a different tool set
/// without walking every schema on each frame.
pub(crate) fn schemas_fingerprint(schemas: &[(String, String)]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, schema) in schemas {
        for byte in name.as_bytes().iter().chain(&schema.len().to_le_bytes()) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Every counted piece of one request, in the order it goes to the model.
///
/// `question` is the message that starts the request: the replayed history
/// stops before it, so it has to be added here or the person's own words would
/// be missing from the record.
pub(crate) fn request_items(
    blocks: &[PromptBlock],
    tool_schemas: &[(String, String)],
    plugin_prompts: &[(String, String)],
    history: &[Turn],
    question: &str,
) -> Vec<RequestItem> {
    let mut items = Vec::new();
    let mut push = |kind: ContextKind, label: String, text: String| {
        let tokens = estimate_tokens(&text);
        items.push(RequestItem {
            kind,
            label,
            text,
            tokens,
        });
    };
    for block in blocks {
        push(
            prompt_block_kind(block.kind),
            block.label.to_string(),
            block.text.clone(),
        );
    }
    for (name, schema) in tool_schemas {
        push(ContextKind::Tools, name.clone(), schema.clone());
    }
    for (id, text) in plugin_prompts {
        push(ContextKind::Injected, format!("插件 {id}"), text.clone());
    }
    for turn in history {
        push(
            turn_kind(turn.speaker),
            turn_label(&turn.text),
            turn.text.clone(),
        );
    }
    if !question.is_empty() {
        push(
            ContextKind::User,
            turn_label(question),
            question.to_string(),
        );
    }
    items
}

/// The first non-empty line of a turn, cut short, as its label.
pub(crate) fn turn_label(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(40)
        .collect()
}

/// The pieces grouped into the categories, in [`ContextKind::ALL`] order.
pub(crate) fn parts_of(items: &[RequestItem]) -> Vec<ContextPart> {
    ContextKind::ALL
        .iter()
        .map(|kind| {
            let own: Vec<ContextItem> = items
                .iter()
                .filter(|item| item.kind == *kind)
                .map(|item| ContextItem {
                    label: item.label.clone(),
                    text: item.text.clone(),
                    tokens: item.tokens,
                })
                .collect();
            ContextPart {
                kind: *kind,
                tokens: own.iter().map(|item| item.tokens).sum(),
                items: own,
            }
        })
        .collect()
}

/// The item that keeps the text a request wrote back.
///
/// That text never rides along in a later prompt, so `requests.jsonl` has to
/// hold it or the panel could never show the answer again. An empty answer has
/// nothing to keep.
pub(crate) fn output_item(text: &str) -> Option<RequestItem> {
    let text = text.trim();
    (!text.is_empty()).then(|| RequestItem {
        kind: ContextKind::Output,
        label: "最终输出".to_string(),
        text: text.to_string(),
        tokens: estimate_tokens(text),
    })
}

/// Add the tool schemas a run actually sent, when the request was recorded
/// before the engine reported them.
///
/// The engine sends `{"type":"tools"}` only once a run has started, so a
/// conversation's first question recorded none of them; they are added when the
/// answer arrives, so the panel still shows what that request carried.
pub(crate) fn add_missing_tool_schemas(items: &mut Vec<RequestItem>, schemas: &[(String, String)]) {
    if schemas.is_empty() || items.iter().any(|item| item.kind == ContextKind::Tools) {
        return;
    }
    items.extend(schemas.iter().map(|(name, schema)| RequestItem {
        kind: ContextKind::Tools,
        label: name.clone(),
        text: schema.clone(),
        tokens: estimate_tokens(schema),
    }));
}

/// The pieces one round carried: every category as stored, but only the first
/// `upto` tool results — the ones the rounds before it had already produced.
fn parts_up_to(parts: &[ContextPart], upto: usize) -> Vec<ContextPart> {
    parts
        .iter()
        .map(|part| {
            let items: Vec<ContextItem> = part
                .items
                .iter()
                .enumerate()
                .filter(|(index, _)| part.kind != ContextKind::ToolResults || *index < upto)
                .map(|(_, item)| ContextItem {
                    label: item.label.clone(),
                    text: item.text.clone(),
                    tokens: item.tokens,
                })
                .collect();
            ContextPart {
                kind: part.kind,
                tokens: items.iter().map(|item| item.tokens).sum(),
                items,
            }
        })
        .collect()
}

/// The category totals of one request, as `chat.json` keeps them.
pub(crate) fn composition_of(items: &[RequestItem]) -> Vec<u32> {
    let mut totals = [0usize; ContextKind::ALL.len()];
    for item in items {
        if let Some(index) = ContextKind::ALL.iter().position(|kind| *kind == item.kind) {
            totals[index] += item.tokens;
        }
    }
    totals
        .iter()
        .map(|tokens| (*tokens).min(u32::MAX as usize) as u32)
        .collect()
}

/// The category totals an answer recorded, in [`ContextKind::ALL`] order.
///
/// `None` for an answer stored before the figures were kept — there is no
/// honest way to split a total after the fact.
fn recorded_totals(row: &RequestRow) -> Option<Vec<usize>> {
    let mut totals: Vec<usize> = row
        .composition
        .iter()
        .map(|tokens| *tokens as usize)
        .collect();
    // A save written before the output category existed has one slot fewer;
    // its output was never recorded, so it stays zero rather than guessed at.
    if totals.len() + 1 == ContextKind::ALL.len() {
        totals.push(0);
    }
    (totals.len() == ContextKind::ALL.len()).then_some(totals)
}

/// The pieces a stored request was made of, with unknown categories skipped.
fn stored_items(stored: &crate::chats::StoredRequest) -> Vec<RequestItem> {
    stored
        .items
        .iter()
        .filter_map(|item| {
            ContextKind::from_key(&item.kind).map(|kind| RequestItem {
                kind,
                label: item.label.clone(),
                text: item.text.clone(),
                tokens: item.tokens as usize,
            })
        })
        .collect()
}

/// The category breakdown of one stored request, if its detail was saved.
fn stored_parts(data: &Path, chat: &str, turn: u64) -> Option<Vec<ContextPart>> {
    let stored = crate::chats::load_request(data, chat, turn)?;
    let items = stored_items(&stored);
    Some(parts_of(&items))
}

/// What each round of a request carried, oldest first, as raw estimates in
/// [`ContextKind::ALL`] order.
///
/// Round `k` gets every piece the request started with plus the tool results
/// produced by the rounds before it: those are what the engine replays into it.
/// Without per-round figures the whole request is one entry, so callers never
/// have to guess at a split that was not recorded.
///
/// The answer each round wrote is counted in its `Output` slot, and the text of
/// the final answer is stored as an item there. That text is not part of the
/// next request's prompt, but it *is* what the endpoint billed for the round, so
/// leaving it out made the classified total fall short of the bill by exactly
/// the completion tokens — and the last round's answer, which no later request
/// ever carries, could never show up at all.
pub(crate) fn round_estimates(
    items: &[RequestItem],
    round_tokens: &[(u64, u64, u32)],
    rounds: u32,
) -> Vec<Vec<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    let parts = parts_of(items);
    let output = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Output)
        .unwrap_or(0);
    // The output text's own estimate is only a placeholder: once the endpoint
    // reported its completion count, that count is what the text cost, so the
    // estimate is dropped rather than counted twice.
    let totals = |parts: &[ContextPart], measured: bool| -> Vec<usize> {
        let mut totals: Vec<usize> = parts.iter().map(|part| part.tokens).collect();
        if measured {
            totals[output] = 0;
        }
        totals
    };
    if round_tokens.is_empty() {
        // Nothing was counted per round, so the request stays one block rather
        // than being split by a guess.
        return vec![totals(&parts, false)];
    }
    let mut calls = 0usize;
    let mut counted = Vec::with_capacity(round_tokens.len());
    for (_, answer, made) in round_tokens {
        let mut round = totals(&parts_up_to(&parts, calls), true);
        // What the round wrote back, as the endpoint itself counted it.
        round[output] = usize::try_from(*answer).unwrap_or(usize::MAX);
        counted.push(round);
        calls = calls.saturating_add(*made as usize);
    }
    // The endpoint can report the number of rounds without a count for each
    // one. The rounds it did not count still sent everything the request had,
    // so the complete set of pieces is the best record of them.
    let recorded = counted.len() as u32;
    if rounds > recorded {
        let full = totals(&parts, true);
        for _ in recorded..rounds {
            counted.push(full.clone());
        }
    }
    counted
}

/// Add every round's category figures together.
pub(crate) fn summed_rounds(rounds: &[Vec<usize>]) -> Vec<usize> {
    let mut totals = vec![0usize; ContextKind::ALL.len()];
    for round in rounds {
        for (slot, tokens) in totals.iter_mut().zip(round) {
            *slot = slot.saturating_add(*tokens);
        }
    }
    totals
}

/// The category figures as the transcript stores them.
pub(crate) fn composition_counts(totals: &[usize]) -> Vec<u32> {
    totals
        .iter()
        .map(|tokens| (*tokens).min(u32::MAX as usize) as u32)
        .collect()
}

/// One arc of the ring: the outside edge out, the inside edge back, so a
/// filled path paints a slice of an annulus.
fn ring_path(
    center: Point<Pixels>,
    outer: f32,
    inner: f32,
    start: f32,
    sweep: f32,
) -> Option<gpui::Path> {
    let steps = ((sweep.abs() / 0.05).ceil() as usize).clamp(2, 512);
    let at = |angle: f32, radius: f32| {
        point(
            px(f32::from(center.x) + radius * angle.cos()),
            px(f32::from(center.y) + radius * angle.sin()),
        )
    };
    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(at(start, outer));
    for step in 1..=steps {
        builder.line_to(at(start + sweep * step as f32 / steps as f32, outer));
    }
    for step in (0..=steps).rev() {
        builder.line_to(at(start + sweep * step as f32 / steps as f32, inner));
    }
    builder.close();
    builder.build().ok()
}

/// The whole conversation as a ring: one arc per category, sized by its share
/// of everything the endpoints billed.
///
/// `segments` are `(colour, share of the circle)`; the shares are expected to
/// add up to one. A share of zero draws nothing, so a conversation with no
/// figures is just the quiet track.
pub(crate) fn consumption_ring(
    segments: Vec<(u32, f32)>,
    track: u32,
    size: f32,
    thickness: f32,
) -> AnyElement {
    let element = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let outer =
                (f32::from(bounds.size.width).min(f32::from(bounds.size.height)) / 2.0) - 1.0;
            let inner = (outer - thickness).max(0.0);
            if let Some(path) = ring_path(center, outer, inner, 0.0, std::f32::consts::TAU) {
                window.paint_path(path, rgb(track));
            }
            // A hair of space between arcs keeps the seams readable when two
            // neighbouring categories hold similar shares.
            let gap = 0.014;
            let mut start = -std::f32::consts::FRAC_PI_2;
            for (colour, share) in &segments {
                let sweep = share * std::f32::consts::TAU - gap;
                let path = (sweep > 0.0)
                    .then(|| ring_path(center, outer, inner, start + gap / 2.0, sweep))
                    .flatten();
                if let Some(path) = path {
                    window.paint_path(path, rgb(*colour));
                }
                start += share * std::f32::consts::TAU;
            }
        },
    );
    element.w(px(size)).h(px(size)).into_any_element()
}

/// The answered questions, newest first, for the request list.
///
/// Only answers carry a figure: a question's own text is part of the request
/// that follows it, so it has nothing of its own to bill. The tool rows that sit
/// between one answer and the next belong to that next answer.
///
/// A request is a run of model round trips, and every round sends the whole
/// prompt again. Its figures are therefore the sum of its rounds: `stored`
/// holds the pieces of each request, so the sums survive even for transcripts
/// written before the sums themselves were stored.
fn request_composition(
    message: &ChatMessage,
    turn: u64,
    stored: &std::collections::HashMap<u64, crate::chats::StoredRequest>,
) -> (Vec<u32>, Vec<Vec<usize>>) {
    let round_composition = stored
        .get(&turn)
        .map(|request| {
            round_estimates(
                &stored_items(request),
                &message.round_tokens,
                message.rounds,
            )
        })
        .unwrap_or_default();
    if round_composition.is_empty() {
        return (recorded_with_answers(message), Vec::new());
    }
    let totals = summed_rounds(&round_composition);
    let counted = composition_counts(&totals);
    let composition = if counted.iter().any(|tokens| *tokens > 0) {
        counted
    } else {
        // The stored pieces are blank or older than the figures; keep what
        // the answer itself recorded.
        recorded_with_answers(message)
    };
    (composition, round_composition)
}

/// The totals an answer recorded, with the answers it produced added in when the
/// save predates that.
///
/// The categories used to describe the *prompt* only, so an old save's
/// `Assistant` slot counts the earlier turns replayed into it and nothing of what
/// this request itself wrote. That is the whole of the gap between the classified
/// total and the bill, so it is filled here rather than left to look like a
/// rounding error. A save that already counted them has a non-zero slot and is
/// left alone, so nothing is added twice.
fn recorded_with_answers(message: &ChatMessage) -> Vec<u32> {
    let mut recorded = message.composition.clone();
    // Saves from before the output category have one slot fewer and kept the
    // answers in `Assistant`; current saves keep them in `Output`.
    let legacy = recorded.len() + 1 == ContextKind::ALL.len();
    if legacy {
        recorded.push(0);
    }
    if recorded.len() != ContextKind::ALL.len() {
        return recorded;
    }
    let written: u64 = message
        .round_tokens
        .iter()
        .map(|(_, answer, _)| *answer)
        .sum();
    if written == 0 {
        return recorded;
    }
    let kind = if legacy {
        ContextKind::Assistant
    } else {
        ContextKind::Output
    };
    let slot = ContextKind::ALL
        .iter()
        .position(|entry| *entry == kind)
        .unwrap_or(0);
    if recorded[slot] == 0 {
        recorded[slot] = written.min(u64::from(u32::MAX)) as u32;
    }
    recorded
}

/// The whole conversation's totals, summed over every answer.
#[derive(Default)]
struct RequestSums {
    /// Every request's pieces added up, in [`ContextKind::ALL`] order.
    composition: Vec<usize>,
    /// Every round's prompt the endpoint reported.
    prompt: u64,
    /// Every round's answer the endpoint reported.
    answer: u64,
    /// Billed tokens no round accounted for: answers whose endpoint reported a
    /// total but no per-round counts.
    unreported: u64,
}

fn request_rows(
    messages: &[ChatMessage],
    stored: &std::collections::HashMap<u64, crate::chats::StoredRequest>,
) -> (Vec<RequestRow>, RequestSums) {
    let mut rows: Vec<RequestRow> = Vec::new();
    let mut sums = RequestSums {
        composition: vec![0usize; ContextKind::ALL.len()],
        ..RequestSums::default()
    };
    let mut tools = 0u64;
    for message in messages {
        match message.role {
            Role::Tool => tools += 1,
            Role::Assistant => {
                let turn = rows.len() as u64 + 1;
                let (composition, round_composition) = request_composition(message, turn, stored);
                for (slot, tokens) in sums.composition.iter_mut().zip(&composition) {
                    *slot = slot.saturating_add(*tokens as usize);
                }
                let prompt: u64 = message
                    .round_tokens
                    .iter()
                    .map(|(prompt, _, _)| *prompt)
                    .sum();
                let answer: u64 = message
                    .round_tokens
                    .iter()
                    .map(|(_, answer, _)| *answer)
                    .sum();
                sums.prompt = sums.prompt.saturating_add(prompt);
                sums.answer = sums.answer.saturating_add(answer);
                sums.unreported = sums
                    .unreported
                    .saturating_add(message.tokens.saturating_sub(prompt.saturating_add(answer)));
                rows.push(RequestRow {
                    turn,
                    model: message.model.clone(),
                    elapsed_ms: message.elapsed_ms,
                    tokens: message.tokens,
                    prompt_tokens: prompt,
                    answer_tokens: answer,
                    context: message.context,
                    rounds: message.rounds,
                    round_tokens: message.round_tokens.clone(),
                    tools,
                    composition,
                    round_composition,
                });
                tools = 0;
            }
            _ => {}
        }
    }
    rows.reverse();
    rows.truncate(REQUEST_ROWS);
    (rows, sums)
}

/// What the tools cost during the question that just finished, as counted
/// pieces of that request: the arguments the model wrote and what came back.
///
/// Those rows are what the engine replays into the next round of the same
/// question, so they belong to its cost; the replayed history never carries
/// them (only the recap block does), which is why they have to be read from the
/// transcript.
pub(crate) fn posted_tool_items(messages: &[ChatMessage]) -> Vec<RequestItem> {
    let mut items = Vec::new();
    for message in messages.iter().rev() {
        match message.role {
            Role::Tool => {
                // What the call cost is both halves: the arguments the model
                // wrote (replayed as an assistant `tool_calls` message) and what
                // the tool returned. Nothing of it is in the replayed history,
                // so both have to be taken from the transcript.
                let args = message.input.trim();
                let output = message.output.trim();
                let text = match (args.is_empty(), output.is_empty()) {
                    (true, true) => continue,
                    (true, false) => message.output.trim_end().to_string(),
                    (false, true) => message.input.trim_end().to_string(),
                    (false, false) => format!(
                        "参数：{}\n\n返回：{}",
                        message.input.trim_end(),
                        message.output.trim_end()
                    ),
                };
                items.push(RequestItem {
                    kind: ContextKind::ToolResults,
                    label: message.text.trim().to_string(),
                    tokens: estimate_tokens(&text),
                    text,
                });
            }
            // The question that started this turn is where the run begins.
            Role::User | Role::Attachment | Role::Assistant => break,
            Role::Note | Role::Approval => {}
        }
    }
    items.reverse();
    items
}

impl DshFlowApp {
    /// The system prompt as labelled blocks, for one conversation.
    ///
    /// Read before the question goes in: the early-turns summary and the tool
    /// recap both describe the turns that came before it.
    pub(crate) fn prompt_blocks(&self, chat_index: usize, model: &str) -> Vec<PromptBlock> {
        let Some(chat) = self.chats.get(chat_index) else {
            return Vec::new();
        };
        let tools = chat.tools;
        let workspace = self.chat_dir(chat_index);
        let mut blocks = vec![PromptBlock {
            kind: PromptKind::System,
            label: "基础指令",
            sep: "",
            text: if tools {
                format!("{SYSTEM_PROMPT}\n{WORK_PROMPT}\n{TOOL_PROMPT}")
            } else {
                format!("{SYSTEM_PROMPT}\n{WORK_PROMPT}")
            },
        }];
        blocks.push(PromptBlock {
            kind: PromptKind::System,
            label: "运行环境",
            sep: "\n\n",
            text: environment_prompt(&workspace, model),
        });
        if tools {
            let sandbox = if chat.full_access {
                "不检查（完全权限）"
            } else {
                "只允许工作区"
            };
            blocks.push(PromptBlock {
                kind: PromptKind::System,
                label: "预算与权限",
                sep: "\n",
                text: format!(
                    "本次预算：最多 {} 轮模型往返、{} 次工具调用。当前权限：{}（{}）；命令路径沙箱：{sandbox}。",
                    chat.max_rounds,
                    chat.max_calls,
                    ToolMode::of(chat).label(),
                    ToolMode::of(chat).hint()
                ),
            });
        }
        // The oldest turns are already gone from the replayed history; keep
        // their opening request so a long conversation still has a task. When a
        // model-written checkpoint exists it says far more than the first lines
        // of those turns, so it replaces the recap instead of adding to it.
        let checkpoint = util::read_checkpoint(&workspace).filter(|entry| entry.usable());
        blocks.push(match checkpoint {
            Some(entry) => PromptBlock {
                kind: PromptKind::Injected,
                label: "早前对话摘要",
                sep: "\n\n",
                text: util::checkpoint_prompt(&entry),
            },
            None => PromptBlock {
                kind: PromptKind::Injected,
                label: "早前对话摘要",
                sep: "\n\n",
                text: history_recap(&chat.messages),
            },
        });
        blocks.push(PromptBlock {
            kind: PromptKind::Injected,
            label: "项目指令",
            sep: "\n\n",
            text: project_rules(&self.ai.workspace_dir(), &workspace),
        });
        // Enabled skills are prompt text, so the window can add them without a
        // round-trip to the engine. The conversation's mode narrows which of the
        // globally enabled skills this request carries; a broken folder is
        // skipped rather than failing the question.
        let selection = self.ai.mode_selection(&chat.mode);
        let skills = crate::skill::prompt_text_for(&self.ai, &selection.skills);
        if !skills.is_empty() {
            blocks.push(PromptBlock {
                kind: PromptKind::Skills,
                label: "启用技能",
                sep: "\n\n",
                text: skills,
            });
        }
        if tools {
            // A shallow map saves the first `tree` round-trip and keeps the
            // model from mistaking the (usually empty) chat folder for the repo.
            blocks.push(PromptBlock {
                kind: PromptKind::Injected,
                label: "工作区结构",
                sep: "\n\n",
                text: workspace_map(&self.ai.workspace_dir(), Some(&workspace), 2, 180, 4_000),
            });
            // A summary of the previous turn's tool output, injected into this
            // prompt: it is counted as injected content, not as this request's
            // tool traffic, so 「工具结果」 only lists the tools this question
            // itself called.
            blocks.push(PromptBlock {
                kind: PromptKind::Injected,
                label: "上一轮工具结果",
                sep: "\n\n",
                text: tool_recap(&chat.messages),
            });
        }
        blocks.retain(|block| !block.text.is_empty());
        blocks
    }

    /// What the conversation holds for the 上下文 tab.
    ///
    /// The request-by-request detail comes from `requests.jsonl`; this only
    /// gathers the figures the whole panel shares — the window, the counts, the
    /// replayed history, and one row per answer.
    pub(crate) fn context_report(&self) -> ContextReport {
        let chat = self.chat();
        let model = self.chat_model();
        // The same numbers the history window uses, so the limit shown here is
        // the one that actually trims.
        let (_, _, history_tokens, _) = history_stats(&chat.messages);
        let window = crate::ai::context_window(&model);
        let output = self
            .ai
            .active()
            .map_or(0, |connection| u64::from(connection.max_tokens));
        let history_budget = MAX_HISTORY_TOKENS;
        let stored = crate::chats::load_requests(&self.ai.data_dir(), &chat.dir);
        let (requests, sums) = request_rows(&chat.messages, &stored);
        let spend = chat.messages.iter().map(|message| message.tokens).sum();
        ContextReport {
            window,
            asked: chat
                .messages
                .iter()
                .filter(|message| matches!(message.role, Role::User))
                .count() as u64,
            history_tokens,
            steps: chat
                .messages
                .iter()
                .filter(|message| matches!(message.role, Role::Tool))
                .count() as u64,
            replies: chat
                .messages
                .iter()
                .filter(|message| matches!(message.role, Role::Assistant))
                .count() as u64,
            output,
            history_budget,
            growth: context_growth(&chat.messages),
            spend,
            total_composition: sums.composition,
            prompt_total: sums.prompt,
            answer_total: sums.answer,
            unreported: sums.unreported,
            requests,
        }
    }

    /// The tab strip under the title: the transcript, the context breakdown, or
    /// the conversation's background tasks.
    ///
    /// The labels are deliberately smaller than the conversation's name and are
    /// neutral grey: a light grey when idle, a darker grey when chosen, with the
    /// same grey underline. The accent stays for actions, not for navigation.
    pub(crate) fn render_chat_tabs(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let tabs = [
            ("tab-chat", "聊天".to_string(), ChatTab::Chat),
            ("tab-context", "上下文".to_string(), ChatTab::Context),
            ("tab-tasks", "后台任务".to_string(), ChatTab::Tasks),
        ];
        let running = self
            .tasks
            .iter()
            .any(|task| task.state == TaskState::Running);
        let mut strip = div()
            .flex()
            .flex_row()
            .items_end()
            .gap_3()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .px_5()
            .pt_1();
        for (id, text, tab) in tabs {
            let active = self.shown_chat_tab() == tab;
            strip = strip.child(
                div()
                    .id(id)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(24.))
                    .px_1()
                    // The idle underline is the hairline's own colour, so only
                    // the selected tab draws a line.
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
                    .on_click(cx.listener(move |view, _, _, cx| view.show_chat_tab(tab, cx)))
                    .child(text)
                    // The old header icon had a running colour; a tiny dot
                    // keeps that signal without tinting the label.
                    .when(tab == ChatTab::Tasks && running, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .size(px(5.))
                                .rounded_full()
                                .bg(rgb(palette.running)),
                        )
                    }),
            );
        }
        strip
    }

    /// The tab actually on screen: a conversation with no messages has no tab
    /// strip, so it must never be left showing the breakdown with no way back.
    pub(crate) fn shown_chat_tab(&self) -> ChatTab {
        if self.chat().messages.is_empty() {
            ChatTab::Chat
        } else {
            self.chat_tab
        }
    }

    /// Switch between the transcript, the context breakdown and the task list.
    pub(crate) fn show_chat_tab(&mut self, tab: ChatTab, cx: &mut Context<Self>) {
        if self.chat_tab == tab {
            return;
        }
        self.chat_tab = tab;
        if tab == ChatTab::Chat {
            self.context_open = None;
            self.context_item = None;
            self.context_round_menu = None;
        }
        if tab == ChatTab::Context {
            // The totals scrollbar is sized from its laid-out viewport, which is
            // still zero on the frame that opens the tab; one more paint lets it
            // appear without the reader having to click again.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(40))
                    .await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        if tab == ChatTab::Tasks {
            // The list is read when the tab opens, so it is current even after
            // the window sat on another view.
            self.task_stop_confirm = None;
            self.refresh_tasks();
        } else {
            // Leaving the task list drops its selection and log; the next visit
            // starts from the list itself instead of a stale log.
            self.task_selected = None;
            self.task_log.clear();
            self.task_stop_confirm = None;
        }
        cx.notify();
    }

    /// A thin scrollbar down the right edge of the totals table.
    ///
    /// The card has a fixed height, and the rows are read at a larger size than
    /// they used to be: when they no longer fit, this is what says so and lets
    /// them be pulled into view, instead of the last category being cut off.
    fn render_totals_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some((track, thumb, max, _)) = tool_bar_metrics(&self.totals_scroll) else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-self.totals_scroll.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = self.totals_drag.is_some();
        let handle = self.totals_scroll.clone();
        div()
            .id("context-table-bar")
            .w(px(8.))
            .flex_shrink_0()
            .h(track + px(4.))
            .flex()
            .flex_col()
            .items_center()
            .block_mouse_except_scroll()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    let Some((track, thumb, max, top)) = tool_bar_metrics(&handle) else {
                        return;
                    };
                    let travel = (track - thumb).max(px(1.));
                    let progress = if max > px(0.) {
                        (-handle.offset().y / max).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let thumb_top = top + travel * progress;
                    let on_thumb =
                        event.position.y >= thumb_top && event.position.y <= thumb_top + thumb;
                    if !on_thumb {
                        // A track press jumps there; the drag carries on from the
                        // new place, so click-and-pull works in one gesture.
                        let ratio = ((event.position.y - top) / travel).clamp(0., 1.);
                        let current = handle.offset();
                        handle.set_offset(point(current.x, -max * ratio));
                    }
                    view.totals_drag = Some((event.position.y, handle.offset().y));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(6.))
                    .h(thumb)
                    .rounded_full()
                    .bg(rgb(if dragging {
                        palette.muted
                    } else {
                        palette.border
                    }))
                    .mt(travel * progress),
            )
            .into_any_element()
    }

    /// Follow the pointer while the totals scrollbar is held.
    pub(crate) fn drag_totals_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((start_y, start_offset)) = self.totals_drag else {
            return;
        };
        let Some((track, thumb, max, _)) = tool_bar_metrics(&self.totals_scroll) else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = self.totals_scroll.offset();
        self.totals_scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// The 上下文 tab: what one request was made of, by category.
    ///
    /// It opens on the newest answer and follows whichever request is picked in
    /// the list below it. The pieces of a request come from `requests.jsonl`,
    /// written when the answer arrived, so a request can be opened again long
    /// after the conversation scrolled past it.
    pub(crate) fn render_context_view(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chat = self.chat();
        let signature = (
            self.active_chat,
            chat.updated_ms,
            chat.messages.len(),
            chat.messages
                .last()
                .map_or(0, |message| message.text.len() + message.output.len()),
            self.tool_schemas
                .get(&chat.dir)
                .map_or(0, |schemas| schemas_fingerprint(schemas)),
            self.plugin_prompts.len(),
        );
        let mut cache = self.context_cache.borrow_mut();
        if !matches!(cache.as_ref(), Some((cached, _)) if *cached == signature) {
            *cache = Some((signature, self.context_report()));
        }
        let report = &cache.as_ref().expect("just filled").1;
        // The panel opens on the newest answer and follows whichever request is
        // picked in the list below; picking it again goes back to the newest.
        let Some(row) = self
            .context_request
            .or_else(|| report.requests.first().map(|row| row.turn))
            .and_then(|turn| report.requests.iter().find(|row| row.turn == turn))
        else {
            return div()
                .id("context-view")
                .flex()
                .flex_col()
                .gap_3()
                .flex_1()
                .w_full()
                .min_h(px(0.))
                .px(px(self.chat_gutter))
                .pt_4()
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .rounded_lg()
                        .bg(rgb(palette.canvas))
                        .border_1()
                        .border_color(rgb(palette.border))
                        .shadow_md()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_4()
                        // The empty panel is still a card, so it names itself the
                        // same way the filled ones do.
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(palette.muted))
                                .child("上下文构成"),
                        )
                        .child(div().text_sm().text_color(rgb(palette.muted)).child(
                            "还没有请求；问一次之后，这里会记下它用了多少 token、花在哪些分类上。",
                        )),
                )
                .into_any_element();
        };
        // The pieces of a request live in `requests.jsonl`, one line per request,
        // so a past request can be opened again without keeping megabytes of
        // prompt text in the transcript. Read once per request, then reused.
        let dir = chat.dir.clone();
        let data = self.ai.data_dir();
        let mut detail = self.context_detail.borrow_mut();
        if !matches!(detail.as_ref(), Some((cached_dir, cached_turn, cached_context, cached_tokens, _))
            if *cached_dir == dir
                && *cached_turn == row.turn
                && *cached_context == row.context
                && *cached_tokens == row.tokens)
        {
            let mut pieces = stored_parts(&data, &dir, row.turn).unwrap_or_else(|| parts_of(&[]));
            // A request saved before the output was kept still has its answer in
            // the transcript, so the closing round can show the text anyway.
            if let Some(answer) = closing_answer(&chat.messages, row.turn)
                && let Some(part) = pieces
                    .iter_mut()
                    .find(|part| part.kind == ContextKind::Output)
                && part.items.is_empty()
            {
                part.tokens = estimate_tokens(&answer);
                part.items.push(ContextItem {
                    label: "最终输出".to_string(),
                    text: answer,
                    tokens: part.tokens,
                });
            }
            *detail = Some((dir, row.turn, row.context, row.tokens, pieces));
        }
        let detail_parts: &[ContextPart] = &detail.as_ref().expect("just filled").4;
        // A request is a run of rounds; the panel can break down one of them or
        // all of them together. A request the endpoint never split stays one
        // block, so callers never invent rounds it did not report.
        // The answer's own count is authoritative: an endpoint can
        // report a round without a per-round split, and the picker must
        // still offer every round the request actually took.
        let round_count = row
            .round_tokens
            .len()
            .max(row.round_composition.len())
            .max(row.rounds as usize);
        let round_pick = self
            .context_round
            .filter(|round| round_count > 0 && *round >= 1 && *round as usize <= round_count);
        let multi_round = round_count > 1;
        // A specific round shows only what it could have seen: everything the
        // request started with, plus the tool results earlier rounds produced.
        let mut round_parts: Vec<ContextPart>;
        let parts: &[ContextPart] = match round_pick {
            Some(round) => {
                let upto: usize = row
                    .round_tokens
                    .iter()
                    .take(round as usize - 1)
                    .map(|(_, _, calls)| *calls as usize)
                    .sum();
                round_parts = parts_up_to(detail_parts, upto);
                // The text the request wrote back belongs to the round that
                // wrote it: the closing round, not the tool rounds before it.
                if round < round_count as u32 {
                    for part in round_parts
                        .iter_mut()
                        .filter(|part| part.kind == ContextKind::Output)
                    {
                        part.items.clear();
                        part.tokens = 0;
                    }
                }
                &round_parts
            }
            None => detail_parts,
        };
        // A category opens only where the pieces of the round on show were
        // stored; a request added up over several rounds has no single copy of
        // the prompt to expand.
        let allow_open = round_pick.is_some() || row.round_composition.len() <= 1;
        // Raw per-category estimates: one round's own, or every round added up.
        let raw: Vec<usize> = match round_pick {
            Some(round) => {
                let pieces = row
                    .round_composition
                    .get(round as usize - 1)
                    .cloned()
                    .unwrap_or_default();
                if pieces.iter().any(|tokens| *tokens > 0) {
                    pieces
                } else {
                    recorded_totals(row).unwrap_or_default()
                }
            }
            None if row.round_composition.is_empty() => recorded_totals(row).unwrap_or_default(),
            None => {
                let pieces = summed_rounds(&row.round_composition);
                if pieces.iter().any(|tokens| *tokens > 0) {
                    pieces
                } else {
                    recorded_totals(row).unwrap_or_default()
                }
            }
        };
        // Two denominators, because they answer two different questions. The window
        // share must stay on the prompt alone: every round sends the whole prompt
        // again, so a billed total would climb past 100% on a tool-using question. The
        // categories, on the other hand, describe what was *billed*, answer tokens
        // included, so they add up to the same figure the "计费" chip shows.
        let prompts: u64 = row.round_tokens.iter().map(|(prompt, _, _)| *prompt).sum();
        let completions: u64 = row.round_tokens.iter().map(|(_, answer, _)| *answer).sum();
        let unreported: u64 = row
            .round_composition
            .iter()
            .skip(row.round_tokens.len())
            .map(|round| round.iter().sum::<usize>() as u64)
            .sum();
        let carried = prompts.saturating_add(unreported);
        let billed = carried.saturating_add(completions);
        // What the head line names as the endpoint's figure: the round's prompt,
        // or every round's prompt added up. Never the bill, so "端点" stays true.
        let shown = match round_pick {
            Some(round) => row
                .round_tokens
                .get(round as usize - 1)
                .map_or(0, |(prompt, _, _)| *prompt),
            None => carried,
        };
        let sizing = match round_pick {
            Some(round) => {
                let own = row.round_tokens.get(round as usize - 1);
                match own {
                    Some((prompt, answer, _)) => prompt.saturating_add(*answer),
                    None => raw.iter().sum::<usize>() as u64,
                }
            }
            None if billed > 0 => billed,
            None => raw.iter().sum::<usize>() as u64,
        };
        let target = if round_pick.is_some() && shown == 0 {
            raw.iter().sum::<usize>() as u64
        } else {
            shown
        };
        // The head line compares our estimate with the endpoint's *prompt*, so
        // it must leave the answers out again: they are billed but never ride
        // along in the next request's prompt.
        let answers_added = match round_pick {
            Some(round) => row
                .round_tokens
                .get(round as usize - 1)
                .map_or(0, |(_, answer, _)| *answer),
            None => completions,
        };
        let output_slot = ContextKind::ALL
            .iter()
            .position(|kind| *kind == ContextKind::Output)
            .unwrap_or(0);
        let prompt_estimate = raw
            .iter()
            .enumerate()
            .map(|(index, tokens)| {
                if index == output_slot {
                    tokens.saturating_sub(answers_added as usize)
                } else {
                    *tokens
                }
            })
            .sum::<usize>();
        let sized = if raw.len() == ContextKind::ALL.len() {
            anchor_parts(&raw, sizing)
        } else {
            vec![0usize; ContextKind::ALL.len()]
        };
        let percent = if report.window == 0 {
            0.0
        } else {
            target as f64 * 100.0 / report.window as f64
        };
        let tokens_of = |kind: ContextKind| {
            ContextKind::ALL
                .iter()
                .position(|entry| *entry == kind)
                .and_then(|index| sized.get(index).copied())
                .unwrap_or(0)
        };
        let muted = |text: String| {
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(text)
        };
        let chip = |value: u64, label: &'static str| {
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
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(short_count(value)),
                )
                .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
        };
        // The ring is the whole conversation, not the request picked below:
        // every request's pieces added up, scaled to the prompt total the
        // endpoints reported, so the arcs add up to the number in the middle.
        let ring_estimate: u64 = report
            .total_composition
            .iter()
            .map(|tokens| *tokens as u64)
            .sum();
        // The center shows what the endpoints billed for the whole conversation;
        // the ring keeps the prompt composition's proportions.
        let ring_target = if report.spend > 0 {
            report.spend
        } else {
            ring_estimate
        };
        let ring = if report.total_composition.len() == ContextKind::ALL.len() {
            anchor_parts(&report.total_composition, ring_target)
        } else {
            vec![0usize; ContextKind::ALL.len()]
        };
        let ring_sum = ring.iter().sum::<usize>().max(1);
        // The card's own header sits at the top-left, above the ring. Keeping it
        // inside the table pushed the title right of the ring, so the card
        // read as if it had no title at its left edge.
        let totals_header = div()
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
                    .child("整个对话合计"),
            )
            .child(div().flex_1().min_w(px(0.)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(format!("{} 次请求", report.replies)),
            );
        // The legend reads as a list, not as a grid of three equal cells: the
        // colour dot and the name lead, the two figures stay right-aligned so
        // digits line up, and a thin bar under each row shows the share. That
        // makes the eight categories scannable at a glance instead of leaving
        // the eye to compare centred numbers.
        let mut table = div()
            .flex()
            .flex_col()
            .gap_0p5()
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
                    .pb_1()
                    .border_b_1()
                    .border_color(rgb(palette.border))
                    // The dot column keeps the heading over the names.
                    .child(div().flex_shrink_0().w(px(7.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("分类"),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(60.))
                            .text_right()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("token"),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(48.))
                            .text_right()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("占比"),
                    ),
            );
        for (kind, tokens) in ContextKind::ALL.iter().zip(&ring) {
            let share = *tokens as f64 * 100.0 / ring_sum as f64;
            let ink = if *tokens > 0 {
                palette.text
            } else {
                palette.faint
            };
            table = table.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
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
                                    .flex_shrink_0()
                                    .size(px(7.))
                                    .rounded_sm()
                                    .bg(rgb(if *tokens > 0 {
                                        kind.color()
                                    } else {
                                        palette.border
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(ink))
                                    .child(kind.label()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .w(px(60.))
                                    .text_right()
                                    .text_xs()
                                    .text_color(rgb(if *tokens > 0 {
                                        palette.muted
                                    } else {
                                        palette.faint
                                    }))
                                    .child(short_count(*tokens as u64)),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .w(px(48.))
                                    .text_right()
                                    .text_xs()
                                    .text_color(rgb(if *tokens > 0 {
                                        kind.color()
                                    } else {
                                        palette.faint
                                    }))
                                    .child(format!("{share:.1}%")),
                            ),
                    )
                    .child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .min_w(px(0.))
                            .rounded_full()
                            .overflow_hidden()
                            .bg(rgb(palette.panel_soft))
                            .child(
                                div()
                                    .h_full()
                                    .w(relative((share as f32 / 100.0).clamp(0., 1.)))
                                    .rounded_full()
                                    .bg(rgb(kind.color())),
                            ),
                    ),
            );
        }
        let segments: Vec<(u32, f32)> = ContextKind::ALL
            .iter()
            .zip(&ring)
            .filter(|(_, tokens)| **tokens > 0)
            .map(|(kind, tokens)| (kind.color(), *tokens as f32 / ring_sum as f32))
            .collect();
        let ring_dial = div()
            .relative()
            .flex_shrink_0()
            .size(px(RING_SIZE))
            .child(consumption_ring(
                segments,
                palette.panel_soft,
                RING_SIZE,
                RING_THICKNESS,
            ))
            .child(
                div()
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .size(px(RING_SIZE))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(short_count(ring_target)),
                    ),
            );
        // How the window filled up: one bar per answer, newest on the right.
        // It sits beside the ring, so the conversation's fill and its
        // composition read together instead of one at each end of the panel.
        let mut growth = div()
            .flex()
            .flex_row()
            .items_end()
            .justify_start()
            .gap_1()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .min_w(px(0.));
        let peak = report.growth.iter().copied().max().unwrap_or(0).max(1);
        // Which bar belongs to the request on show: growth keeps the newest
        // answers, so the distance from the last one gives the position.
        let newest_turn = report.requests.first().map_or(0, |row| row.turn);
        let picked_bar = {
            let back = newest_turn.saturating_sub(row.turn) as usize;
            report.growth.len().checked_sub(back + 1)
        };
        for (index, size) in report.growth.iter().enumerate() {
            let share = (*size as f32 / peak as f32).clamp(0.04, 1.);
            // The bar of the request on show stands out.
            let colour = if picked_bar == Some(index) {
                palette.accent
            } else {
                palette.border
            };
            growth = growth.child(
                div()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .flex_1()
                    .min_w(px(2.))
                    .max_w(px(18.))
                    .h_full()
                    .child(
                        div()
                            .w_full()
                            .h(relative(share))
                            .rounded_sm()
                            .bg(rgb(colour)),
                    ),
            );
        }
        // The chart takes every pixel the right column has left, so its bars
        // grow with the ring beside it instead of sitting in a short strip.
        let growth_note = if report.growth.is_empty() {
            "还没有用量".to_string()
        } else {
            format!("最新在右 · 最高 {}", short_count(peak))
        };
        let growth_block = div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
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
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.muted))
                            .child("上下文增长"),
                    )
                    .child(div().flex_1().min_w(px(0.)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(growth_note),
                    ),
            )
            .child(growth);
        // The limits that decide what happens next, in one row under the ring.
        let chips = div()
            .flex()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .flex_wrap()
            .w_full()
            .min_w(px(0.))
            .child(chip(report.prompt_total, "提示合计"))
            .child(chip(report.answer_total, "回答合计"))
            .when(report.unreported > 0, |this| {
                this.child(chip(report.unreported, "未回报"))
            })
            .child(chip(report.spend, "计费"))
            .child(chip(ring_estimate, "分类估算"))
            .child(chip(report.output, "留给回答"))
            .child(chip(
                report.history_budget as u64,
                if report.history_full() {
                    "历史预算（已满）"
                } else {
                    "历史预算"
                },
            ))
            .child(chip(report.asked, "轮次"))
            .child(chip(report.replies, "回复"))
            .child(chip(report.steps, "工具调用"));
        // Six cards, each on its own rounded, shadowed surface: the first row
        // pairs the narrow ring card with the wider totals table, then the
        // grey chips strip, the growth chart, the request list, and the
        // breakdown of one request.
        // The top row is two cards: the ring on the left, narrower, and the
        // horizontal table on the right, wider. The grey chips belong to
        // neither, so they move into their own long strip under the pair.
        let chart_card = div()
            .flex()
            .flex_col()
            .items_center()
            .justify_start()
            .gap_3()
            .flex_shrink_0()
            .w(px(TOTALS_CHART_WIDTH))
            .h(px(TOTALS_CARD_HEIGHT))
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .p_4()
            // The card names itself at the top; the ring sits under that text, in
            // whatever room is left, so the figures are read before the shape.
            .child(totals_header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .child(ring_dial)
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("计费 token"),
                    ),
            );
        let table_card = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .h(px(TOTALS_CARD_HEIGHT))
            .gap_2()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .p_4()
            // The card names itself above the rows so it is read as a card, not
            // as a table that happens to sit beside the ring.
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
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.muted))
                            .child("分类占比"),
                    )
                    .child(div().flex_1().min_w(px(0.)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(format!("{} 类", ContextKind::ALL.len())),
                    ),
            )
            // The rows are taller than the card at the larger size, so they
            // scroll inside it; the bar is a real column beside them, so it never
            // covers a name or a figure.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .gap_2()
                    .child(
                        div()
                            .id("context-table-scroll")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&self.totals_scroll)
                            .child(table),
                    )
                    .child(self.render_totals_bar(palette, cx)),
            );
        let chips_card = div()
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
            .p_3()
            // The chips used to hang under the two cards with no heading of
            // their own, which made them look like a footnote to the table.
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
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.muted))
                            .child("用量与预算"),
                    )
                    .child(div().flex_1().min_w(px(0.)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("整个对话"),
                    ),
            )
            .child(chips);
        // Stacked under it, with a height of its own so the bars still reach the
        // card's bottom edge instead of shrinking to a strip.
        let growth_card = div()
            .flex()
            .flex_col()
            .w_full()
            .h(px(190.))
            .min_w(px(0.))
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .p_4()
            .child(growth_block);
        // The category rows double as the legend: the colour dot names the
        // category, while the figures and the bar stay neutral so the eye reads
        // the percentages instead of a wall of coloured numbers.
        let mut list = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_1()
            .w_full()
            .min_w(px(0.));
        for part in parts {
            // A category opens only where the pieces of the round on show were
            // stored; a request added up over several rounds has no single copy
            // of the prompt to expand. The final output is the exception: it was
            // written once, in the closing round, so it is readable even from
            // the summed view.
            let output_part = part.kind == ContextKind::Output;
            let can_open = !part.items.is_empty() && (allow_open || output_part);
            let open = can_open && self.context_open == Some(part.kind);
            let tokens = tokens_of(part.kind);
            let request_share = tokens as f64 * 100.0 / sizing.max(1) as f64;
            let mut row = div()
                .id(("context-part", part.kind as usize))
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(if open {
                    palette.panel_soft
                } else {
                    palette.panel
                }))
                .border_1()
                .border_color(rgb(palette.border))
                .when(can_open, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.bg(rgb(palette.panel_soft)))
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(if !can_open {
                            ""
                        } else if open {
                            "▾"
                        } else {
                            "▸"
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .size(px(8.))
                        .rounded_sm()
                        .bg(rgb(part.kind.color())),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(part.kind.label()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(match part.kind.empty_note() {
                            Some(note) => note.to_string(),
                            // A tool-result row is the accumulated set, not a
                            // per-round slice: the selected round only carries
                            // the results returned before it started.
                            None if part.kind == ContextKind::ToolResults => {
                                if round_pick.is_some() {
                                    format!("{} 项 · 进入本轮前", part.items.len())
                                } else {
                                    format!("{} 项 · 整个请求", part.items.len())
                                }
                            }
                            None if !allow_open && !output_part => {
                                format!("{round_count} 轮合计")
                            }
                            None => format!("{} 项", part.items.len()),
                        }),
                )
                .child(div().flex_1().min_w(px(0.)))
                // One neutral figure: the share of this request, then what
                // those tokens are, with no "占窗口" label in front.
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(140.))
                        .text_right()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{request_share:.1}% ≈ {}",
                            short_count(tokens as u64)
                        )),
                )
                // The bar is a neutral grey pair: a light track and a grey
                // fill, so the category colour stays on its dot.
                .child(
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .h(px(6.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(rgb(palette.border))
                        .child(
                            div()
                                .w(relative((request_share as f32 / 100.0).clamp(0., 1.)))
                                .h_full()
                                .bg(rgb(palette.muted)),
                        ),
                );
            if can_open {
                let kind = part.kind;
                row = row.on_click(cx.listener(move |view, _, _, cx| {
                    view.context_open = if view.context_open == Some(kind) {
                        None
                    } else {
                        Some(kind)
                    };
                    view.context_item = None;
                    cx.notify();
                }));
            }
            list = list.child(row);
            if open {
                let mut items = div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .pl(px(22.))
                    .pb_2();
                if part.kind == ContextKind::ToolResults {
                    items = items.child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .whitespace_normal()
                            .child(if round_pick.is_some() {
                                "这些是进入本轮提示前已经返回的工具结果，按发生顺序排列；本轮自己的调用结果出现在下一轮。"
                            } else {
                                "这是整个请求累计的工具返回，按发生顺序排列。"
                            }),
                    );
                }
                for (item_index, item) in part.items.iter().enumerate() {
                    let item_share = if sizing == 0 {
                        0.0
                    } else {
                        item.tokens as f64 * 100.0 / sizing as f64
                    };
                    let kind = part.kind;
                    let can_expand = !item.text.trim().is_empty();
                    let item_open = can_expand && self.context_item == Some((kind, item_index));
                    let mut header = div()
                        .id((kind.key(), item_index))
                        .flex()
                        .flex_row()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(if !can_expand {
                                    ""
                                } else if item_open {
                                    "▾"
                                } else {
                                    "▸"
                                }),
                        )
                        // Number the tool results so a round's slice can be read
                        // as "第 1–4 个结果" rather than as four rounds.
                        .when(part.kind == ContextKind::ToolResults, |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(format!("#{}", item_index + 1)),
                            )
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .max_h(px(18.))
                                .overflow_hidden()
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(palette.text))
                                .whitespace_normal()
                                .line_clamp(1)
                                .text_ellipsis()
                                .child(item.label.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{item_share:.0}%")),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(format!("≈ {}", short_count(item.tokens as u64))),
                        );
                    if can_expand {
                        header = header
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.context_item = if view.context_item == Some((kind, item_index))
                                {
                                    None
                                } else {
                                    Some((kind, item_index))
                                };
                                cx.notify();
                            }));
                    }
                    items = items.child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_shrink_0()
                            .gap_1()
                            .w_full()
                            .min_w(px(0.))
                            .p_2()
                            .overflow_hidden()
                            .rounded_sm()
                            .bg(rgb(palette.canvas))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .child(header)
                            .when(item_open, |this| {
                                this.child(
                                    div()
                                        .w_full()
                                        .min_w(px(0.))
                                        .max_h(px(DETAIL_MAX_HEIGHT))
                                        .min_h(px(0.))
                                        .flex_shrink_0()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "context-item-{}-{item_index}",
                                                    kind.key()
                                                )))
                                                .w_full()
                                                .min_w(px(0.))
                                                .max_h(px(DETAIL_MAX_HEIGHT - 2.))
                                                .min_h(px(0.))
                                                .flex_shrink_0()
                                                .overflow_y_scroll()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .whitespace_normal()
                                                .child(item.text.clone()),
                                        ),
                                )
                            }),
                    );
                }
                list = list.child(items);
            }
        }
        // Every question the endpoint reported on, newest first: what it was
        // billed beside what it filled, so a single request can be picked out of
        // the conversation instead of only the newest one being visible.
        let column = |width: f32, text: &'static str| {
            div()
                .flex_shrink_0()
                .w(px(width))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(text)
        };
        let mut table = div()
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
                    .px_3()
                    .child(column(36., "轮次"))
                    .child(column(52., "耗时"))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("模型"),
                    )
                    .child(column(48., "往返"))
                    .child(column(70., "Σ 每轮"))
                    .child(column(60., "回答"))
                    .child(column(72., "计费"))
                    .child(column(64., "占累计"))
                    .child(column(56., "工具"))
                    .child(column(84., "末轮上下文")),
            );
        let total_spend = report.spend.max(1) as f64;
        for row in &report.requests {
            let spent = row.tokens as f64 * 100.0 / total_spend;
            // The repeated part of the bill: every round sent the prompt again.
            let prompts = row.prompt_tokens;
            // Picking a row shows that request above, and picking it again goes
            // back to the one that is about to be sent.
            let picked = self.context_request == Some(row.turn);
            let turn = row.turn;
            table = table.child(
                div()
                    .id(("context-request", row.turn as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(if picked {
                        palette.panel_soft
                    } else {
                        palette.panel
                    }))
                    .border_1()
                    .border_color(rgb(if picked {
                        palette.accent
                    } else {
                        palette.border
                    }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.context_request = if view.context_request == Some(turn) {
                            None
                        } else {
                            Some(turn)
                        };
                        // A round belongs to the request that was on show.
                        view.context_round = None;
                        view.context_item = None;
                        view.context_round_menu = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(36.))
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(row.turn.to_string()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(52.))
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(if row.elapsed_ms == 0 {
                                "—".to_string()
                            } else {
                                format_duration(row.elapsed_ms)
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .line_clamp(1)
                            .child(if row.model.is_empty() {
                                "—".to_string()
                            } else {
                                row.model.clone()
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(48.))
                            .text_xs()
                            .text_color(rgb(if row.rounds > 1 {
                                palette.text
                            } else {
                                palette.faint
                            }))
                            .child(if row.rounds == 0 {
                                "—".to_string()
                            } else {
                                format!("{} 轮", row.rounds)
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(70.))
                            .text_xs()
                            .text_color(rgb(if prompts > 0 {
                                palette.text
                            } else {
                                palette.faint
                            }))
                            .child(if prompts == 0 {
                                "—".to_string()
                            } else {
                                short_count(prompts)
                            }),
                    )
                    // What that request wrote back: the other half of its bill,
                    // which the prompt column alone can never add up to.
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(60.))
                            .text_xs()
                            .text_color(rgb(if row.answer_tokens > 0 {
                                palette.text
                            } else {
                                palette.faint
                            }))
                            .child(if row.prompt_tokens == 0 && row.answer_tokens == 0 {
                                "—".to_string()
                            } else {
                                short_count(row.answer_tokens)
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(72.))
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(short_count(row.tokens)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(64.))
                            .text_xs()
                            .text_color(rgb(if picked {
                                palette.accent
                            } else {
                                palette.muted
                            }))
                            .child(format!("{spent:.1}%")),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(56.))
                            .text_xs()
                            .text_color(rgb(if row.tools > 0 {
                                palette.text
                            } else {
                                palette.faint
                            }))
                            .child(if row.tools == 0 {
                                "—".to_string()
                            } else {
                                format!("{} 次", row.tools)
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(84.))
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child(short_count(row.context)),
                    ),
            );
        }
        if report.requests.is_empty() {
            table = table.child(muted(
                "还没有回答过；问一次之后，每次请求的计费与上下文都会列在这里。".to_string(),
            ));
        } else if (report.requests.len() as u64) < report.replies {
            table = table.child(muted(format!(
                "只列最近 {} 次请求（这段对话共 {} 次）。",
                REQUEST_ROWS, report.replies
            )));
        }
        // What the categories below describe: the whole request, or one of its
        // rounds picked at the title.
        let detail_head = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap_1()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(palette.muted))
                    .child(format!("第 {} 次请求的构成", row.turn)),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child({
                        let estimate: usize = prompt_estimate;
                        let lead = if round_pick.is_some() {
                            "本轮提示"
                        } else if unreported > 0 || prompts == 0 {
                            "合计"
                        } else {
                            "端点"
                        };
                        let share = if round_pick.is_none() && multi_round {
                            format!("累计窗口占比 {percent:.1}%")
                        } else {
                            format!("窗口占比 {percent:.1}%")
                        };
                        let numbers = if estimate > 0 {
                            format!(
                                "{lead} {} · 估算 {} · {share}",
                                short_count(target),
                                short_count(estimate as u64)
                            )
                        } else {
                            format!(
                                "{lead} {} · 本次没有被记下内容 · {share}",
                                short_count(target)
                            )
                        };
                        // Why the bill is bigger than the window: every round
                        // sent the whole prompt again. A picked round shows its
                        // own two figures instead.
                        let round_total = row.rounds.max(round_count as u32).max(1);
                        // A request's tool calls are not its round trips: one
                        // round can issue several calls, and those results only
                        // appear in the following round's prompt. Naming both
                        // here keeps "7 次工具" from reading as "7 轮".
                        let tools_tail = if row.tools > 0 {
                            format!(" · {} 次工具", row.tools)
                        } else {
                            String::new()
                        };
                        let numbers = match round_pick {
                            Some(round) => match row.round_tokens.get(round as usize - 1) {
                                // The round was estimated from the stored
                                // pieces; the endpoint reported no counts.
                                None => numbers,
                                Some((_, completion, calls)) => {
                                    let tail = if *calls > 0 {
                                        format!(
                                            "回答 {} · {calls} 个工具",
                                            short_count(*completion)
                                        )
                                    } else {
                                        format!("回答 {}", short_count(*completion))
                                    };
                                    format!("{numbers} · {tail}")
                                }
                            },
                            None if prompts > 0 => format!(
                                "{numbers} · {round_total} 轮往返{tools_tail} · Σ 每轮 {} / 回答 {}",
                                short_count(prompts),
                                short_count(
                                    row.round_tokens
                                        .iter()
                                        .map(|(_, completion, _)| *completion)
                                        .sum()
                                )
                            ),
                            // The endpoint reported the bill but no per-round
                            // split: the request still has a cost to show.
                            None if row.tokens > 0 => format!(
                                "{numbers} · 计费 {}（端点未按轮回报提示 / 回答）{tools_tail}",
                                short_count(row.tokens)
                            ),
                            None if round_total > 1 => {
                                format!("{numbers} · {round_total} 轮往返{tools_tail}")
                            }
                            None => numbers,
                        };
                        if !allow_open && multi_round {
                            format!("{numbers} · 在下方轮次列表里选某一轮看全文")
                        } else {
                            numbers
                        }
                    }),
            );
        // A request with many tool rounds used to grow one chip per round here,
        // pushing the categories below out of the way. The picker keeps it to a
        // single row; the rounds themselves live in a popup list.
        let round_picker = if multi_round {
            let all_picked = round_pick.is_none();
            let tools_note = if row.tools > 0 {
                format!(" · {} 个工具", row.tools)
            } else {
                String::new()
            };
            let (label, note) = match round_pick {
                Some(round) => {
                    let (prompt, completion, calls) = row
                        .round_tokens
                        .get(round as usize - 1)
                        .map_or((0, 0, 0), |(prompt, completion, calls)| {
                            (*prompt, *completion, *calls)
                        });
                    (
                        format!("第 {round} 轮往返"),
                        if prompt > 0 {
                            format!(
                                "提示 {} / 回答 {} · {calls} 个工具",
                                short_count(prompt),
                                short_count(completion)
                            )
                        } else {
                            format!("未回报用量 · {calls} 个工具")
                        },
                    )
                }
                None => (
                    format!("合计 {round_count} 轮往返"),
                    if prompts > 0 {
                        format!("Σ 提示 {}{tools_note}", short_count(prompts))
                    } else {
                        format!("各轮估算{tools_note}")
                    },
                ),
            };
            Some(
                div()
                    .id("context-round-picker")
                    .flex()
                    .flex_row()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    // Keep the selector the same width as the category bars
                    // below, so a long label cannot stretch past them.
                    .w_full()
                    .min_w(px(0.))
                    .border_1()
                    .border_color(rgb(if all_picked {
                        palette.accent
                    } else {
                        palette.border
                    }))
                    .bg(rgb(if all_picked {
                        palette.panel_soft
                    } else {
                        palette.canvas
                    }))
                    .text_xs()
                    .text_color(rgb(if all_picked {
                        palette.accent
                    } else {
                        palette.text
                    }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    // Keep the press from reaching the root, which would close
                    // the list before the click could toggle it.
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(|view, event: &gpui::ClickEvent, _, cx| {
                        cx.stop_propagation();
                        view.category_menu = None;
                        view.card_menu = None;
                        view.mcp_menu = None;
                        view.mcp_skills = None;
                        view.plugin_menu = None;
                        view.skill_menu = None;
                        view.skill_delete_confirm = None;
                        let at = event.position();
                        view.context_round_menu = if view.context_round_menu.is_some() {
                            None
                        } else {
                            Some(point(at.x, at.y + px(24.)))
                        };
                        cx.notify();
                    }))
                    .child(label)
                    .child(div().text_color(rgb(palette.muted)).child(note))
                    .child(div().flex_1().min_w(px(0.)))
                    .child(div().text_color(rgb(palette.faint)).child("▾")),
            )
        } else {
            None
        };
        div()
            .id("context-view")
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .min_h(px(0.))
            .min_w(px(0.))
            .overflow_y_scroll()
            .track_scroll(&self.context_scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_4()
                    .w_full()
                    .min_w(px(0.))
                    // The same centred column the transcript uses: the spare
                    // room of a wide window is split evenly on both sides, so
                    // the panel reads at the conversation's width instead of
                    // stretching to the edges.
                    .px(px(self.chat_gutter))
                    .pt_3()
                    .pb_3()
                    .child(
                        // The ring and its table sit side by side: the ring
                        // card stays narrow, the table takes the spare width.
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap_4()
                            .w_full()
                            .min_w(px(0.))
                            .child(chart_card)
                            .child(table_card),
                    )
                    .child(chips_card)
                    .child(growth_card)
                    .child(
                        div()
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
                                    .px_1()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("请求列表（点一条看它的构成）"),
                            )
                            .child(table),
                    )
                    .child(
                        div()
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
                            .child(detail_head)
                            .when_some(round_picker, |this, picker| this.child(picker))
                            // The counts above name rounds and tool calls apart;
                            // this sentence explains why a round's tool results
                            // stop at the calls made before it.
                            .when(multi_round, |this| {
                                this.child(
                                    div()
                                        .px_1()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .whitespace_normal()
                                        .child(
                                            "一轮是一次模型往返，一轮可以发出多次工具调用；第 N 轮只包含进入它之前已返回的工具结果，本轮自己的结果出现在下一轮。",
                                        ),
                                )
                            })
                            .child(list),
                    ),
            )
            .into_any_element()
    }

    /// The popup behind the round picker: the whole request, then every round.
    ///
    /// It is drawn by the window root, not inside the context scroll box, so a
    /// long list is never clipped by the categories underneath.
    pub(crate) fn render_context_round_menu(
        &self,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let report = self.context_report();
        let Some(row) = self
            .context_request
            .or_else(|| report.requests.first().map(|row| row.turn))
            .and_then(|turn| report.requests.iter().find(|row| row.turn == turn))
        else {
            return div().into_any_element();
        };
        let round_count = row
            .round_tokens
            .len()
            .max(row.round_composition.len())
            .max(row.rounds as usize);
        if round_count == 0 {
            return div().into_any_element();
        }
        let picked = self
            .context_round
            .filter(|round| *round >= 1 && *round as usize <= round_count);
        let all_picked = picked.is_none();
        let prompts: u64 = row.round_tokens.iter().map(|(prompt, _, _)| *prompt).sum();
        let all_note = if prompts > 0 {
            format!("Σ 提示 {}", short_count(prompts))
        } else {
            "各轮估算".to_string()
        };
        let all_note = if row.tools > 0 {
            format!("{all_note} · {} 个工具", row.tools)
        } else {
            all_note
        };
        let mut items = div()
            .id("context-round-menu-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_1()
            .max_h(px(320.))
            .overflow_y_scroll()
            .child(
                div()
                    .id("context-round-menu-all")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(rgb(if all_picked {
                        palette.panel_soft
                    } else {
                        palette.canvas
                    }))
                    .text_xs()
                    .text_color(rgb(if all_picked {
                        palette.accent
                    } else {
                        palette.text
                    }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(|view, _, _, cx| {
                        cx.stop_propagation();
                        view.context_round = None;
                        view.context_item = None;
                        view.context_round_menu = None;
                        cx.notify();
                    }))
                    .child(format!("合计 {round_count} 轮往返"))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_right()
                            .text_color(rgb(palette.muted))
                            .child(all_note),
                    ),
            );
        for index in 0..round_count {
            let round = index as u32 + 1;
            let selected = picked == Some(round);
            let (prompt, completion, calls) = row
                .round_tokens
                .get(index)
                .map_or((0, 0, 0), |(prompt, completion, calls)| {
                    (*prompt, *completion, *calls)
                });
            items = items.child(
                div()
                    .id(("context-round-menu-item", round as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .px_2()
                    .py_1()
                    .rounded_sm()
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
                        cx.stop_propagation();
                        view.context_round = Some(round);
                        view.context_item = None;
                        view.context_round_menu = None;
                        cx.notify();
                    }))
                    .child(format!("第 {round} 轮往返"))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_right()
                            .text_color(rgb(palette.muted))
                            .child(if prompt > 0 {
                                format!(
                                    "提示 {} / 回答 {} · {calls} 个工具",
                                    short_count(prompt),
                                    short_count(completion)
                                )
                            } else {
                                format!("未回报用量 · {calls} 个工具")
                            }),
                    ),
            );
        }
        div()
            .id("context-round-menu")
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .w(px(300.))
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            // Presses inside the list must not reach the root, which closes it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(palette.muted))
                    .child("选择轮次"),
            )
            .child(items)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ChatMessage, ContextKind, Role};
    use super::{anchor_parts, context_growth};

    #[test]
    fn anchoring_spreads_a_measured_number_over_the_estimate() {
        // The provider reported 1000 tokens; the parts keep their proportions
        // and still add up to exactly that number.
        let scaled = anchor_parts(&[30, 90], 1000);
        assert_eq!(scaled, vec![250, 750]);
        assert_eq!(scaled.iter().sum::<usize>(), 1000);
        // Rounding goes to the largest part, so no token is invented or lost.
        let thirds = anchor_parts(&[1, 1, 1], 100);
        assert_eq!(thirds.iter().sum::<usize>(), 100);
        // Flooring first keeps the total exact even when it is smaller than the
        // number of parts.
        let thinned = anchor_parts(&[1, 1, 1, 1, 1, 1, 1], 3);
        assert_eq!(thinned.iter().sum::<usize>(), 3);
        assert_eq!(thinned.iter().filter(|tokens| **tokens > 0).count(), 3);
        // Nothing measured means nothing to scale.
        assert_eq!(anchor_parts(&[0, 0], 500), vec![0, 0]);
        assert_eq!(anchor_parts(&[10, 20], 0), vec![10, 20]);
    }

    #[test]
    fn the_growth_chart_keeps_the_newest_answers() {
        let mut messages = Vec::new();
        for context in 1..=20_u64 {
            let mut answer = ChatMessage::assistant("a");
            answer.context = context;
            messages.push(answer);
        }
        // A question carries no usage of its own.
        messages.push(ChatMessage::user("q"));
        let growth = context_growth(&messages);
        assert_eq!(growth.len(), 12);
        assert_eq!(growth.last().copied(), Some(20));
        assert_eq!(growth.first().copied(), Some(9));
        assert!(context_growth(&[ChatMessage::user("q")]).is_empty());
    }

    #[test]
    fn a_role_of_assistant_is_what_the_growth_chart_counts() {
        let mut note = ChatMessage::note("?? ? read", false);
        note.context = 500;
        assert!(context_growth(&[note]).is_empty());
        let mut answer = ChatMessage::assistant("done");
        answer.role = Role::Assistant;
        answer.context = 500;
        assert_eq!(context_growth(&[answer]), vec![500]);
    }
    #[test]
    fn the_request_table_lists_answers_newest_first() {
        use super::{REQUEST_ROWS, request_rows};
        let mut messages = Vec::new();
        let answers = REQUEST_ROWS + 6;
        for index in 1..=answers as u64 {
            messages.push(ChatMessage::user("q"));
            let mut answer = ChatMessage::assistant("a");
            answer.tokens = index * 10;
            answer.context = index * 5;
            answer.elapsed_ms = index;
            messages.push(answer);
        }
        let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
        // The newest answer is first, and only one page of them is listed.
        assert_eq!(rows.len(), REQUEST_ROWS);
        assert_eq!(rows[0].turn, answers as u64);
        assert_eq!(rows[0].tokens, answers as u64 * 10);
        assert_eq!(rows[0].context, answers as u64 * 5);
        assert_eq!(rows[REQUEST_ROWS - 1].turn, 7);
        // A question on its own has nothing to bill yet.
        assert!(
            request_rows(&[ChatMessage::user("q")], &std::collections::HashMap::new())
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_request_without_round_counts_still_has_a_bill() {
        use super::request_rows;
        // A question sent with the tools switched off reports its total only.
        // The panel must keep that bill and say the per-round split was never
        // reported, instead of making the request look free.
        let mut answer = ChatMessage::assistant("a");
        answer.tokens = 700;
        answer.context = 650;
        let (rows, sums) = request_rows(
            &[ChatMessage::user("q"), answer],
            &std::collections::HashMap::new(),
        );
        assert_eq!(rows[0].tokens, 700);
        assert_eq!(rows[0].prompt_tokens, 0);
        assert_eq!(rows[0].answer_tokens, 0);
        assert_eq!(rows[0].context, 650);
        assert_eq!(sums.prompt, 0);
        assert_eq!(sums.answer, 0);
        // 提示合计 + 回答合计 + 未回报 = 计费, so the chips can still add up.
        assert_eq!(sums.unreported, 700);
        assert_eq!(sums.prompt + sums.answer + sums.unreported, 700);
    }

    #[test]
    fn a_request_keeps_the_totals_its_answer_recorded() {
        use super::{recorded_totals, request_rows};
        use crate::app::context_view::ContextKind;
        let mut answer = ChatMessage::assistant("a");
        answer.context = 900;
        answer.composition = vec![100, 200, 300, 0, 0, 250, 50];
        let (rows, ..) = request_rows(&[answer], &std::collections::HashMap::new());
        let totals = recorded_totals(&rows[0]).expect("a legacy save still has a shape");
        // Same order as `ContextKind::ALL`, so the ring and the legend line up.
        // The old save has no output slot, so it is padded with a zero.
        assert_eq!(totals, vec![100, 200, 300, 0, 0, 250, 50, 0]);
        assert_eq!(totals.len(), ContextKind::ALL.len());
        // A current save carries the output slot itself.
        let mut current = ChatMessage::assistant("a");
        current.context = 900;
        current.composition = vec![100, 200, 300, 0, 0, 0, 50, 25];
        let (rows, ..) = request_rows(&[current], &std::collections::HashMap::new());
        assert_eq!(
            recorded_totals(&rows[0]).expect("eight figures were recorded"),
            vec![100, 200, 300, 0, 0, 0, 50, 25]
        );
        // An answer stored before the figures were kept has no shape to show.
        let mut old = ChatMessage::assistant("a");
        old.context = 900;
        assert!(
            recorded_totals(&request_rows(&[old], &std::collections::HashMap::new()).0[0])
                .is_none()
        );
        // A run of the wrong length is not a shape either.
        let mut broken = ChatMessage::assistant("a");
        broken.composition = vec![1, 2, 3];
        assert!(
            recorded_totals(&request_rows(&[broken], &std::collections::HashMap::new()).0[0])
                .is_none()
        );
    }

    #[test]
    fn an_old_save_gets_its_answers_counted_back_in() {
        use super::{ContextKind, RequestItem, request_rows, round_estimates};
        use crate::app::context_view::ContextKind as Kind;
        // A save from before the answers were counted: the assistant slot holds
        // only the earlier turns replayed into the prompt.
        let mut answer = ChatMessage::assistant("a");
        answer.context = 900;
        answer.rounds = 2;
        answer.round_tokens = vec![(200, 60, 1), (250, 40, 0)];
        answer.composition = vec![300, 0, 30, 0, 0, 0, 40];
        let (rows, ..) = request_rows(&[answer], &std::collections::HashMap::new());
        let slot = ContextKind::ALL
            .iter()
            .position(|kind| *kind == ContextKind::Assistant)
            .unwrap_or(0);
        // The 100 tokens the request itself wrote are back in the slot, so the
        // classified total no longer falls short of the bill by the answers.
        assert_eq!(rows[0].composition[slot], 100);
        assert_eq!(rows[0].composition.iter().sum::<u32>(), 470);
        assert_eq!(rows[0].composition.len(), ContextKind::ALL.len());
        // A current save counts them in the output slot, so nothing is added
        // twice there either.
        let mut current = ChatMessage::assistant("a");
        current.rounds = 2;
        current.round_tokens = vec![(200, 60, 1), (250, 40, 0)];
        current.composition = vec![300, 0, 30, 0, 0, 0, 40, 100];
        let (rows, ..) = request_rows(&[current], &std::collections::HashMap::new());
        let output = ContextKind::ALL
            .iter()
            .position(|kind| *kind == ContextKind::Output)
            .unwrap_or(0);
        assert_eq!(rows[0].composition[slot], 0);
        assert_eq!(rows[0].composition[output], 100);
        assert_eq!(rows[0].composition.iter().sum::<u32>(), 470);
        // The helper itself is what the fallback uses, so cover it directly too.
        let items = vec![RequestItem {
            kind: Kind::System,
            label: "system".to_string(),
            text: "system".to_string(),
            tokens: 100,
        }];
        let rounds = round_estimates(&items, &[(50, 25, 0)], 1);
        // What the round wrote is counted where the output lives, not beside
        // the replayed history.
        assert_eq!(rounds[0][slot], 0);
        assert_eq!(rounds[0][output], 25);
    }

    #[test]
    fn each_request_keeps_the_tool_calls_it_made() {
        use super::request_rows;
        let mut first = ChatMessage::assistant("a");
        first.tokens = 100;
        let mut read = ChatMessage::tool("read · a.txt — 已读取 2 行", false);
        read.output = "1\talpha\n2\tbeta\n".to_string();
        let mut write = ChatMessage::tool("write · b.txt — 已写入", false);
        write.output = "已写入 1 行".to_string();
        let mut second = ChatMessage::assistant("b");
        second.tokens = 200;
        second.rounds = 4;
        second.round_tokens = vec![(100, 10, 1), (120, 12, 2)];
        let messages = vec![
            ChatMessage::user("q1"),
            read,
            write,
            first,
            ChatMessage::user("q2"),
            ChatMessage::tool("list · . — 3 项", false),
            second,
        ];
        let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
        // The two calls before the first answer belong to it, the one after the
        // second question belongs to the second answer.
        assert_eq!(rows[1].turn, 1);
        assert_eq!(rows[1].tools, 2);
        assert_eq!(rows[0].turn, 2);
        assert_eq!(rows[0].tools, 1);
        // How many round trips it took, which explains the bill.
        assert_eq!(rows[0].rounds, 4);
        assert_eq!(rows[1].rounds, 0);
        // And each round's own counts, so the repeated prompts can be shown.
        assert_eq!(rows[0].round_tokens, vec![(100, 10, 1), (120, 12, 2)]);
        assert!(rows[1].round_tokens.is_empty());
        // A question that used no tools has nothing to report.
        let (plain, ..) = request_rows(
            &[ChatMessage::user("q"), ChatMessage::assistant("a")],
            &std::collections::HashMap::new(),
        );
        assert_eq!(plain[0].tools, 0);
    }

    #[test]
    fn a_round_only_keeps_the_tool_results_it_could_have_seen() {
        use super::{ContextItem, ContextKind, ContextPart, parts_up_to};
        let part = |kind: ContextKind, labels: &[(&str, usize)]| ContextPart {
            kind,
            tokens: labels.iter().map(|(_, tokens)| *tokens).sum(),
            items: labels
                .iter()
                .map(|(label, tokens)| ContextItem {
                    label: label.to_string(),
                    text: format!("body of {label}"),
                    tokens: *tokens,
                })
                .collect(),
        };
        let parts = vec![
            part(ContextKind::System, &[("prompt", 100)]),
            part(
                ContextKind::ToolResults,
                &[("r1", 10), ("r2", 20), ("r3", 30)],
            ),
        ];
        // The first round ran before any tool result existed.
        let first = parts_up_to(&parts, 0);
        assert_eq!(first[1].items.len(), 0);
        assert_eq!(first[1].tokens, 0);
        // The second round already carried the first round's result.
        let second = parts_up_to(&parts, 1);
        assert_eq!(second[1].items.len(), 1);
        assert_eq!(second[1].items[0].label, "r1");
        assert_eq!(second[1].tokens, 10);
        // Everything else is unchanged: a round does not drop the system prompt.
        assert_eq!(second[0].items.len(), 1);
        assert_eq!(second[0].tokens, 100);
        // Asking for more results than exist keeps them all.
        assert_eq!(parts_up_to(&parts, 9)[1].items.len(), 3);
    }

    #[test]
    fn the_tool_rows_of_the_run_become_part_of_its_request() {
        use super::posted_tool_items;
        let mut read = ChatMessage::tool("read · a.txt — 已读取 2 行", false);
        read.input = "{\"path\":\"a.txt\"}".to_string();
        read.output = "1\talpha\n".to_string();
        let mut empty = ChatMessage::tool("list · . — 空目录", false);
        empty.output = "   ".to_string();
        let messages = vec![
            ChatMessage::user("上一问"),
            ChatMessage::tool("read · old.txt — 已读取", false),
            ChatMessage::assistant("上一答"),
            ChatMessage::user("这一问"),
            ChatMessage::note("上下文超出上限", false),
            read,
            empty,
        ];
        let items = posted_tool_items(&messages);
        // Only this run's tool rows; the one with nothing to send is skipped.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "read · a.txt — 已读取 2 行");
        assert_eq!(items[0].kind, ContextKind::ToolResults);
        // What the call cost is both halves: the arguments the model wrote and
        // what came back, so neither side is missing from the total.
        assert_eq!(
            items[0].text,
            "参数：{\"path\":\"a.txt\"}\n\n返回：1\talpha"
        );
        assert_eq!(
            items[0].tokens,
            crate::app::util::estimate_tokens(&items[0].text)
        );
        assert!(items[0].tokens > crate::app::util::estimate_tokens("1\talpha"));
        // Nothing after the question: nothing to add.
        assert!(posted_tool_items(&[ChatMessage::user("q")]).is_empty());
    }

    #[test]
    fn the_closing_answer_of_a_turn_comes_from_the_transcript() {
        use super::closing_answer;
        let messages = vec![
            ChatMessage::user("q1"),
            ChatMessage::assistant("第一答"),
            ChatMessage::tool("read · a.txt", false),
            ChatMessage::user("q2"),
            ChatMessage::assistant("  第二答  "),
            ChatMessage::user("q3"),
            ChatMessage::assistant("   "),
        ];
        assert_eq!(closing_answer(&messages, 1).as_deref(), Some("第一答"));
        assert_eq!(closing_answer(&messages, 2).as_deref(), Some("第二答"));
        // A stopped answer has nothing to show, and so does a turn that has
        // not been answered yet.
        assert_eq!(closing_answer(&messages, 3), None);
        assert_eq!(closing_answer(&messages, 4), None);
        assert_eq!(closing_answer(&messages, 0), None);
    }

    #[test]
    fn the_text_a_request_wrote_is_kept_with_that_request() {
        use super::output_item;
        let item = output_item("  第一行\n第二行 \n").expect("an answer is kept");
        assert_eq!(item.kind, ContextKind::Output);
        assert_eq!(item.label, "最终输出");
        assert_eq!(item.text, "第一行\n第二行");
        assert_eq!(
            item.tokens,
            crate::app::util::estimate_tokens("第一行\n第二行")
        );
        // A stopped or empty answer has nothing to keep.
        assert!(output_item("   ").is_none());
    }

    #[test]
    fn the_first_question_of_a_conversation_gets_its_tool_schemas_later() {
        use super::{add_missing_tool_schemas, composition_of, request_items};
        let schemas = vec![("read".to_string(), "abcd".repeat(3))];
        let mut items = request_items(&[], &[], &[], &[], "q");
        // Recorded before the engine reported anything: no tool definition yet.
        assert_eq!(composition_of(&items)[1], 0);
        add_missing_tool_schemas(&mut items, &schemas);
        assert_eq!(composition_of(&items)[1], 3);
        assert_eq!(items.last().unwrap().label, "read");
        assert_eq!(items.last().unwrap().tokens, 3);
        // A request that already recorded its schemas is left alone.
        let before = composition_of(&items);
        add_missing_tool_schemas(&mut items, &schemas);
        assert_eq!(composition_of(&items), before);
        // No schemas reported (a chat with tools off) adds nothing.
        let mut bare = request_items(&[], &[], &[], &[], "q");
        add_missing_tool_schemas(&mut bare, &[]);
        assert_eq!(composition_of(&bare)[1], 0);
    }

    #[test]
    fn a_request_records_every_piece_under_its_category() {
        use super::{composition_of, parts_of, request_items};
        use crate::ai::{Speaker, Turn};
        use crate::app::context_view::{PromptBlock, PromptKind};
        let blocks = vec![
            PromptBlock {
                kind: PromptKind::System,
                label: "base",
                sep: "",
                text: "abcd".repeat(2),
            },
            PromptBlock {
                kind: PromptKind::Injected,
                label: "notes",
                sep: "\n\n",
                text: "efgh".repeat(3),
            },
            PromptBlock {
                kind: PromptKind::Injected,
                label: "done",
                sep: "\n\n",
                text: "ijkl".repeat(4),
            },
        ];
        let schemas = vec![("read".to_string(), "mnop".repeat(5))];
        let prompts = vec![("demo".to_string(), "qrst".repeat(6))];
        let history = vec![Turn {
            speaker: Speaker::Assistant,
            text: "yzab".repeat(8),
        }];
        let items = request_items(&blocks, &schemas, &prompts, &history, &"uvwx".repeat(7));
        // Four ASCII characters make one token, so the counts are the lengths
        // over four; the order is the legend's order: system, tools, user,
        // injected, skills, assistant, tool results, output. The two injected
        // blocks and the plugin prompt land in the same category (3 + 4 + 6),
        // and nothing is counted as a tool result here: the recap block is
        // injected content.
        assert_eq!(composition_of(&items), vec![2, 5, 7, 13, 0, 8, 0, 0]);
        let parts = parts_of(&items);
        assert_eq!(parts.len(), 8);
        assert_eq!(parts[2].tokens, 7);
        // The question is recorded as a piece of its own, not merged away.
        assert_eq!(parts[2].items.len(), 1);
        assert_eq!(parts[2].items[0].tokens, 7);
        assert_eq!(parts[5].items.len(), 1);
        assert!(parts[4].items.is_empty());
        // Nothing to send still gives a full row of zeroes, never a short list.
        assert_eq!(
            composition_of(&request_items(&[], &[], &[], &[], "")),
            vec![0; 8]
        );
        // Skills land in their own category rather than with injected content.
        let skill = request_items(
            &[PromptBlock {
                kind: PromptKind::Skills,
                label: "启用技能",
                sep: "\n\n",
                text: "efgh".repeat(2),
            }],
            &[],
            &[],
            &[],
            "",
        );
        assert_eq!(composition_of(&skill)[4], 2);
        assert_eq!(composition_of(&skill)[3], 0);
    }

    #[test]
    fn a_request_adds_every_rounds_pieces_together() {
        use super::{ContextKind, RequestItem, composition_counts, round_estimates, summed_rounds};
        let item = |kind: ContextKind, label: &str, tokens: usize| RequestItem {
            kind,
            label: label.to_string(),
            text: label.to_string(),
            tokens,
        };
        let items = vec![
            item(ContextKind::System, "system", 100),
            item(ContextKind::User, "question", 10),
            item(ContextKind::ToolResults, "r1", 10),
            item(ContextKind::ToolResults, "r2", 20),
        ];
        let rounds = round_estimates(&items, &[(200, 5, 1), (250, 6, 1), (300, 7, 0)], 3);
        assert_eq!(rounds.len(), 3);
        // The first round ran before any tool result existed. What it wrote is
        // in the output slot: billed, but never replayed into a prompt.
        assert_eq!(rounds[0], vec![100, 0, 10, 0, 0, 0, 0, 5]);
        // Each later round replays the results the earlier ones produced.
        assert_eq!(rounds[1], vec![100, 0, 10, 0, 0, 0, 10, 6]);
        assert_eq!(rounds[2], vec![100, 0, 10, 0, 0, 0, 30, 7]);
        // The request's own figures are the element-wise sum, not one copy.
        let totals = summed_rounds(&rounds);
        assert_eq!(totals, vec![300, 0, 30, 0, 0, 0, 40, 18]);
        assert_eq!(
            composition_counts(&totals),
            vec![300, 0, 30, 0, 0, 0, 40, 18]
        );
        // The categories now carry every round's answer as well: the tokens the old
        // version dropped were exactly the completions.
        let answers: usize = 5 + 6 + 7;
        let prompts_only = 3 * 100 + 3 * 10 + 40;
        assert_eq!(totals.iter().sum::<usize>(), prompts_only + answers);
        assert_eq!(totals[ContextKind::Assistant as usize], 0);
        assert_eq!(totals[ContextKind::Output as usize], answers);
        // With no per-round counts the whole request stays one entry, so an old
        // answer is never split by a guess.
        let single = round_estimates(&items, &[], 3);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0], vec![100, 0, 10, 0, 0, 0, 30, 0]);
        // A round the endpoint did not count still sent the whole request.
        let tail = round_estimates(&items, &[(200, 5, 1)], 3);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0], vec![100, 0, 10, 0, 0, 0, 0, 5]);
        assert_eq!(tail[2], vec![100, 0, 10, 0, 0, 0, 30, 0]);
    }

    #[test]
    fn a_round_can_make_two_calls_and_their_results_arrive_together() {
        use super::{ContextKind, RequestItem, round_estimates};
        let item = |kind: ContextKind, label: &str, tokens: usize| RequestItem {
            kind,
            label: label.to_string(),
            text: label.to_string(),
            tokens,
        };
        let mut items = vec![
            item(ContextKind::System, "system", 100),
            item(ContextKind::User, "question", 10),
        ];
        // Seven calls over four tool rounds: two, two, two, then one.
        for result in 1..=7 {
            items.push(item(
                ContextKind::ToolResults,
                &format!("result {result}"),
                1,
            ));
        }
        let rounds = round_estimates(
            &items,
            &[(100, 5, 2), (110, 6, 2), (120, 7, 2), (130, 8, 1), (140, 9, 0)],
            5,
        );
        assert_eq!(rounds.len(), 5);
        let tool_results = |round: &[usize]| round[ContextKind::ToolResults as usize];
        // Round one starts before any result exists; the two calls it makes
        // return together, so round two sees both — not one per round.
        assert_eq!(tool_results(&rounds[0]), 0);
        assert_eq!(tool_results(&rounds[1]), 2);
        assert_eq!(tool_results(&rounds[2]), 4);
        assert_eq!(tool_results(&rounds[3]), 6);
        assert_eq!(tool_results(&rounds[4]), 7);
    }

    #[test]
    fn a_request_row_uses_the_sum_of_its_rounds() {
        use super::{ContextKind, request_rows};
        use crate::chats::{StoredRequest, StoredRequestItem};
        let stored = StoredRequest {
            turn: 1,
            at_ms: 0,
            model: "m".to_string(),
            tokens: 900,
            context: 500,
            calls: 0,
            items: vec![
                StoredRequestItem {
                    kind: "system".to_string(),
                    label: "system".to_string(),
                    text: "system".to_string(),
                    tokens: 100,
                },
                StoredRequestItem {
                    kind: "user".to_string(),
                    label: "question".to_string(),
                    text: "question".to_string(),
                    tokens: 10,
                },
                StoredRequestItem {
                    kind: "toolResults".to_string(),
                    label: "r1".to_string(),
                    text: "r1".to_string(),
                    tokens: 10,
                },
                StoredRequestItem {
                    kind: "toolResults".to_string(),
                    label: "r2".to_string(),
                    text: "r2".to_string(),
                    tokens: 20,
                },
            ],
        };
        let mut details = std::collections::HashMap::new();
        details.insert(1, stored);
        let mut answer = ChatMessage::assistant("a");
        answer.model = "m".to_string();
        answer.tokens = 900;
        answer.context = 500;
        answer.rounds = 3;
        answer.round_tokens = vec![(200, 5, 1), (250, 6, 1), (300, 7, 0)];
        // What an older transcript recorded: the pieces counted once.
        answer.composition = vec![100, 0, 10, 0, 0, 0, 30];
        let (rows, ..) = request_rows(&[answer], &details);
        // The row now holds the sum of the three rounds: tool traffic included,
        // and each round's own answer in the output slot, so the categories
        // add up to the bill instead of stopping at the prompts.
        assert_eq!(rows[0].composition, vec![300, 0, 30, 0, 0, 0, 40, 18]);
        // The list shows both halves of the bill: what was sent each round and
        // what came back, which is what makes 计费 = Σ 每轮 + 回答 readable.
        assert_eq!(rows[0].prompt_tokens, 750);
        assert_eq!(rows[0].answer_tokens, 18);
        assert_eq!(rows[0].round_composition.len(), 3);
        assert_eq!(
            rows[0].round_composition[2][ContextKind::ToolResults as usize],
            30
        );
        // The categories carry each round's answer too, which is the 18 the older
        // version of this sum dropped.
        assert_eq!(rows[0].composition.iter().sum::<u32>(), 388);
        assert_eq!(rows[0].composition[ContextKind::Assistant as usize], 0);
        assert_eq!(rows[0].composition[ContextKind::Output as usize], 18);
    }

    #[test]
    fn the_tool_schema_fingerprint_follows_the_content() {
        use super::schemas_fingerprint;
        let one = vec![("read".to_string(), "{}".to_string())];
        let same = vec![("read".to_string(), "{}".to_string())];
        // Same number of tools, different names or schemas: the panel must notice.
        let renamed = vec![("grep".to_string(), "{}".to_string())];
        let changed = vec![("read".to_string(), "{\"a\":1}".to_string())];
        assert_eq!(schemas_fingerprint(&one), schemas_fingerprint(&same));
        assert_ne!(schemas_fingerprint(&one), schemas_fingerprint(&renamed));
        assert_ne!(schemas_fingerprint(&one), schemas_fingerprint(&changed));
        assert_eq!(schemas_fingerprint(&[]), 0xcbf2_9ce4_8422_2325);
    }
}

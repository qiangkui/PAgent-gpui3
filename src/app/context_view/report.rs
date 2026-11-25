//! Turning a transcript into rows, parts and figures.

use super::*;

/// Most round trips one request is laid out for.
///
/// The engine caps a question far below this; the limit only keeps a damaged
/// save, or an engine reporting a nonsensical `rounds` count, from asking the
/// panel for gigabytes of per-round slots.
pub(crate) const ROUND_LIMIT: u32 = 256;

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

/// Spread a measured number of tokens over the parts' own proportions.
///
/// The parts are floored first and the leftovers go to the parts with the
/// largest remainders, so the parts always add up to `target` exactly instead
/// of drifting by a few tokens.
pub(crate) fn anchor_parts(tokens: &[usize], target: u64) -> Vec<usize> {
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

/// Spread a category's measured figure over its entries, by their own estimates.
///
/// The row is the endpoint's number and the entries are the text behind it. Rounding
/// the shares down and giving the remainder to the largest remainders keeps the
/// expansion adding up to the row exactly, so a row and its expansion never
/// disagree, and the row's share of the request is the same figure as its number.
pub(crate) fn spread_tokens(estimates: &[usize], category: usize) -> Vec<u64> {
    if estimates.is_empty() {
        return Vec::new();
    }
    let estimated: usize = estimates.iter().sum();
    if estimated == 0 || category == 0 {
        return estimates.iter().map(|_| 0).collect();
    }
    let category = category as u128;
    let estimated = estimated as u128;
    let mut shares: Vec<u64> = estimates
        .iter()
        .map(|tokens| (category * *tokens as u128 / estimated) as u64)
        .collect();
    let mut left = u64::try_from(category)
        .unwrap_or(u64::MAX)
        .saturating_sub(shares.iter().sum());
    let mut order: Vec<usize> = (0..estimates.len()).collect();
    order.sort_by_key(|index| std::cmp::Reverse(category * estimates[*index] as u128 % estimated));
    for index in order {
        if left == 0 {
            break;
        }
        shares[index] += 1;
        left -= 1;
    }
    shares
}

/// The window bar's parts: the prompt's make-up, without the answer it bought.
///
/// The bar answers "how full was the window when this prompt went out", so the
/// answer tokens in the output slot — billed after the prompt had already gone —
/// are dropped before the parts are scaled to the prompt total. The categories
/// listed below the bar still describe the whole bill.
pub(crate) fn window_parts(sized: &[usize], target: u64) -> Vec<usize> {
    let mut parts = sized.to_vec();
    if let Some(slot) = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Output)
    {
        parts[slot] = 0;
    }
    anchor_parts(&parts, target)
}

/// The text of the answer that closed one question, read from the transcript.
///
/// `turn` counts answers from one, the same way [`request_rows`] numbers them,
/// so a request saved before the output text was kept can still show it.
pub(crate) fn closing_answer(messages: &[ChatMessage], turn: u64) -> Option<String> {
    let index = usize::try_from(turn.checked_sub(1)?).ok()?;
    messages
        .iter()
        .filter(|message| message.role == Role::Assistant)
        .nth(index)
        .map(|message| message.text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Which category a system-prompt block is counted under.
pub(crate) fn prompt_block_kind(kind: PromptKind) -> ContextKind {
    match kind {
        PromptKind::System => ContextKind::System,
        PromptKind::Injected => ContextKind::Injected,
        PromptKind::Skills => ContextKind::Skills,
    }
}

/// Which category one replayed turn is counted under.
pub(crate) fn turn_kind(speaker: Speaker) -> ContextKind {
    match speaker {
        Speaker::User => ContextKind::User,
        Speaker::Assistant => ContextKind::Assistant,
    }
}

/// The starting value of the FNV-1a hashes the panel fingerprints with.
pub(crate) const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a's multiply step.
pub(crate) const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over a byte sequence: cheap, allocation-free, and enough to notice
/// that a cached figure was built from different text.
pub(crate) fn fnv1a(hash: u64, bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut hash = hash;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A fingerprint of one piece of text, for the panel's cache keys.
pub(crate) fn text_fingerprint(text: &str) -> u64 {
    fnv1a(FNV_OFFSET, text.bytes())
}

/// A cheap fingerprint of one conversation's tool schemas.
///
/// The panel rebuilds its breakdown when the schemas change; hashing the names
/// and the length of each schema is enough to notice a different tool set
/// without walking every schema on each frame.
pub(crate) fn schemas_fingerprint(schemas: &[(String, String)]) -> u64 {
    let mut hash = FNV_OFFSET;
    for (name, schema) in schemas {
        hash = fnv1a(
            hash,
            name.as_bytes()
                .iter()
                .chain(&schema.len().to_le_bytes())
                .copied(),
        );
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

/// Whether a tool row is one the provider ran on its own servers.
///
/// Those run inside the round that asked for them, so their results are in the
/// prompt from that round on; local tools only reach the prompt in the next one.
pub(crate) fn is_server_result(label: &str) -> bool {
    label.starts_with("web_search") || label.starts_with("read_url")
}

/// The name of the measured remainder, named for where it came from.
pub(crate) fn gap_label(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::Injected => "引擎回放（本机未记录）",
        ContextKind::ToolResults => "服务端搜索读页正文（本机只存摘要）",
        _ => "端点回报与本机估算的差额",
    }
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
/// `None` for an answer stored before the figures were kept —there is no
/// honest way to split a total after the fact.
pub(crate) fn recorded_totals(row: &RequestRow) -> Option<Vec<usize>> {
    let mut totals: Vec<usize> = row
        .composition
        .iter()
        .map(|tokens| *tokens as usize)
        .collect();
    // Categories were appended over time (output, then reasoning); an old save
    // never recorded the newer tail, so those slots stay zero, not guessed.
    if totals.is_empty() {
        // Nothing was recorded at all; an empty row is not a shape.
        return None;
    }
    if totals.len() < ContextKind::ALL.len() && ContextKind::ALL.len() - totals.len() <= 2 {
        totals.resize(ContextKind::ALL.len(), 0);
    }
    (totals.len() == ContextKind::ALL.len()).then_some(totals)
}

/// The pieces a stored request was made of, with unknown categories skipped.
pub(crate) fn stored_items(stored: &crate::chats::StoredRequest) -> Vec<RequestItem> {
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
pub(crate) fn stored_parts(data: &Path, chat: &str, turn: u64) -> Option<Vec<ContextPart>> {
    let stored = crate::chats::load_request(data, chat, turn)?;
    let items = stored_items(&stored);
    let mut parts = parts_of(&items);
    // Thinking never rode in a prompt, so it is not one of the counted items;
    // it is still the request's own text, and the panel opens it beside the
    // tokens it cost instead of leaving the category unopenable.
    let thinking = stored.reasoning.trim();
    if !thinking.is_empty()
        && let Some(part) = parts
            .iter_mut()
            .find(|part| part.kind == ContextKind::Reasoning)
    {
        let tokens = stored.reasoning_tokens as usize;
        part.tokens = part.tokens.saturating_add(tokens);
        part.items.push(ContextItem {
            label: "思考过程".to_string(),
            text: thinking.to_string(),
            tokens,
        });
    }
    Some(parts)
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
/// the completion tokens —and the last round's answer, which no later request
/// ever carries, could never show up at all.
pub(crate) fn round_estimates(
    items: &[RequestItem],
    round_tokens: &[(u64, u64, u32)],
    rounds: u32,
) -> Vec<Vec<usize>> {
    if items.is_empty() {
        return Vec::new();
    }
    let output = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Output)
        .unwrap_or(0);
    let tool_results = ContextKind::ToolResults as usize;
    // Only the token counts matter here, so the text is never copied: a request
    // with many large tool results used to clone all of them once per round just
    // to add up nine numbers.
    let mut base = vec![0usize; ContextKind::ALL.len()];
    // Local tool results are replayed from the next round on; a server result
    // is already visible in the round that ran it, so it is always included.
    let mut local_prefix = vec![0usize];
    let mut local_tokens = 0usize;
    let mut server_tokens = 0usize;
    for item in items {
        base[item.kind as usize] = base[item.kind as usize].saturating_add(item.tokens);
        if item.kind == ContextKind::ToolResults {
            if is_server_result(&item.label) {
                server_tokens = server_tokens.saturating_add(item.tokens);
            } else {
                local_tokens = local_tokens.saturating_add(item.tokens);
                local_prefix.push(local_tokens);
            }
        }
    }
    let seen_tools = |upto: usize| {
        let local = local_prefix
            .get(upto.min(local_prefix.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0);
        server_tokens.saturating_add(local)
    };
    // The output text's own estimate is only a placeholder: once the endpoint
    // reported its completion count, that count is what the text cost, so the
    // estimate is dropped rather than counted twice.
    if round_tokens.is_empty() {
        // Nothing was counted per round, so the request stays one block rather
        // than being split by a guess.
        return vec![base];
    }
    let mut counted = Vec::with_capacity(round_tokens.len());
    let mut calls = 0usize;
    for (_, answer, made) in round_tokens {
        let mut round = base.clone();
        round[tool_results] = seen_tools(calls);
        // What the round wrote back, as the endpoint itself counted it.
        round[output] = usize::try_from(*answer).unwrap_or(usize::MAX);
        counted.push(round);
        calls = calls.saturating_add(*made as usize);
    }
    // The endpoint can report the number of rounds without a count for each
    // one. The rounds it did not count still sent everything the request had,
    // so the complete set of pieces is the best record of them.
    if rounds > counted.len() as u32 {
        let mut full = base;
        full[output] = 0;
        for _ in counted.len() as u32..rounds {
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

/// Add each round's thinking tokens to that round's make-up.
///
/// Thinking is output the bill includes, so it belongs in the same table as the
/// answer; the prompt pieces never carried it, which is why it is added here
/// instead of being stored as an item.
pub(crate) fn add_reasoning(rounds: &mut [Vec<usize>], reasoning: &[u32]) {
    let Some(slot) = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Reasoning)
    else {
        return;
    };
    for (round, tokens) in rounds.iter_mut().zip(reasoning) {
        if let Some(cell) = round.get_mut(slot) {
            *cell = cell.saturating_add(*tokens as usize);
        }
    }
}

/// Which round each thinking stretch belongs to, in tokens and in words.
///
/// The engine reports a figure per round; the transcript rows say which round
/// and hold the text. A row's own estimate is used only when the engine sent no
/// figure (an older save, or a gateway that omits the array), so the panel still
/// splits thinking the way it happened instead of repeating a total.
pub(crate) fn round_thinking(
    message: &ChatMessage,
    rounds: u32,
    thinking: &[(u32, String)],
) -> (Vec<u32>, Vec<String>) {
    let count = rounds.max(1) as usize;
    let mut tokens = vec![0u32; count];
    let mut words: Vec<String> = vec![String::new(); count];
    if !message.round_reasoning.is_empty() {
        for (index, figure) in message.round_reasoning.iter().enumerate().take(count) {
            tokens[index] = *figure;
        }
    } else {
        for (round, text) in thinking {
            let index = (*round as usize).saturating_sub(1).min(count - 1);
            let estimate = estimate_tokens(text).min(u32::MAX as usize) as u32;
            tokens[index] = tokens[index].saturating_add(estimate);
        }
    }
    for (round, text) in thinking {
        let index = (*round as usize).saturating_sub(1).min(count - 1);
        if text.trim().is_empty() {
            continue;
        }
        if !words[index].is_empty() {
            words[index].push_str("\n\n");
        }
        words[index].push_str(text);
    }
    (tokens, words)
}

pub(crate) fn request_composition(
    message: &ChatMessage,
    turn: u64,
    stored: &std::collections::HashMap<u64, crate::chats::StoredRequest>,
    reasoning: &[u32],
) -> (Vec<u32>, Vec<Vec<usize>>) {
    let mut round_composition = stored
        .get(&turn)
        .map(|request| {
            round_estimates(
                &stored_items(request),
                &message.round_tokens,
                message.rounds,
            )
        })
        .unwrap_or_default();
    if !round_composition.is_empty() {
        // Thinking rides with the round that produced it; the stored pieces
        // never held it, because it is output rather than prompt.
        add_reasoning(&mut round_composition, reasoning);
    }
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
pub(crate) fn recorded_with_answers(message: &ChatMessage) -> Vec<u32> {
    let mut recorded = message.composition.clone();
    // Saves from before the output category have two slots fewer and kept the
    // answers in `Assistant`; current saves keep them in `Output`, and the
    // newest tail slot (reasoning) simply was not recorded.
    let legacy = recorded.len() + 2 == ContextKind::ALL.len();
    if !recorded.is_empty()
        && recorded.len() < ContextKind::ALL.len()
        && ContextKind::ALL.len() - recorded.len() <= 2
    {
        recorded.resize(ContextKind::ALL.len(), 0);
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
pub(crate) struct RequestSums {
    /// Every request's closed make-up added up, in [`ContextKind::ALL`] order:
    /// the same figures the request card draws, so the table and the ring
    /// cannot disagree with it.
    pub(crate) composition: Vec<usize>,
    /// Every request's raw local estimate added up, for the 「分类估算」 chip.
    pub(crate) estimate: u64,
    /// Every round's prompt the endpoint reported.
    pub(crate) prompt: u64,
    /// Every round's answer the endpoint reported.
    pub(crate) answer: u64,
    /// Billed tokens no round accounted for: answers whose endpoint reported a
    /// total but no per-round counts.
    pub(crate) unreported: u64,
}

/// Close a request's parts on the endpoint's own figures.
///
/// The estimate cannot see everything the engine assembled — mainly replayed
/// history, and a server-side search's full pages (only their summary is kept
/// locally). That gap belongs where it came from: 工具结果 when a server call
/// ran, 注入 (the engine's replay) otherwise. The answer and its thinking come
/// from the endpoint itself, so they are never scaled. This is the split
/// dsh-context draws: measured figures on both sides, the message side
/// attributed to its categories, and no "other" row.
pub(crate) fn close_request_parts(
    raw: &[usize],
    prompt_target: u64,
    completion: u64,
    reasoning: usize,
    unreported: u64,
    gap_slot: ContextKind,
) -> (Vec<usize>, u64) {
    if raw.len() != ContextKind::ALL.len() {
        return (vec![0usize; ContextKind::ALL.len()], 0);
    }
    let output = ContextKind::Output as usize;
    let thinking = ContextKind::Reasoning as usize;
    let gap_index = gap_slot as usize;
    let mut parts = raw.to_vec();
    let measured = if completion > 0 || unreported > 0 {
        completion
            .max(raw[output] as u64)
            .saturating_add(unreported)
    } else {
        raw[output] as u64
    };
    parts[thinking] = reasoning.min(measured as usize);
    parts[output] = (measured as usize).saturating_sub(parts[thinking]);
    let estimated_prompt: usize = parts
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != output && *index != thinking)
        .map(|(_, tokens)| *tokens)
        .sum();
    let target = usize::try_from(prompt_target).unwrap_or(usize::MAX);
    let gap = target.saturating_sub(estimated_prompt);
    if gap > 0 {
        // The unrecorded part of the prompt belongs where it came from: a
        // server-side search's full pages, or the engine's replay when no web
        // tool ran.
        parts[gap_index] = parts[gap_index].saturating_add(gap);
        return (parts, gap as u64);
    }
    if target > 0 && estimated_prompt > target {
        // An over-estimate is scaled down so the prompt still matches the bill.
        let mut prompt_only = parts.clone();
        prompt_only[output] = 0;
        prompt_only[thinking] = 0;
        let scaled = anchor_parts(&prompt_only, prompt_target);
        for (index, slot) in parts.iter_mut().enumerate() {
            if index != output && index != thinking {
                *slot = scaled[index];
            }
        }
    }
    (parts, 0)
}

/// A request's make-up closed on the endpoint's figures, as the totals use it.
///
/// The request card closes whichever split it shows; the whole-conversation
/// table, the ring and the timeline must close the same way, or a server
/// search's full pages end up as 80% of one card and 10% of another. The
/// arithmetic is the card's own: pick the recorded split, then hand the gap
/// between the local estimate and the endpoint's prompt to the category it came
/// from.
pub(crate) fn closed_request_parts(row: &RequestRow) -> Vec<usize> {
    let raw = if row.round_composition.is_empty() {
        recorded_totals(row).unwrap_or_default()
    } else {
        let pieces = summed_rounds(&row.round_composition);
        // A split shorter than the round count cannot be the whole request's
        // make-up; the totals it recorded are.
        let complete = row.round_composition.len() >= row.rounds.max(1) as usize;
        if complete && pieces.iter().any(|tokens| *tokens > 0) {
            pieces
        } else if let Some(totals) = recorded_totals(row) {
            totals
        } else {
            pieces
        }
    };
    if raw.len() != ContextKind::ALL.len() {
        return raw;
    }
    // Only the endpoint's own prompt figure can close the split. A round it
    // said it took but never split is already inside `tokens`, and hence inside
    // `unreported` below; adding that round's local estimate to the prompt
    // target as well would count the same tokens twice and push the categories
    // past what the endpoint actually charged.
    let prompt_target = if row.round_tokens.is_empty() {
        0
    } else {
        row.prompt_tokens
    };
    let unreported = if row.round_tokens.is_empty() {
        0
    } else {
        row.tokens
            .saturating_sub(row.prompt_tokens.saturating_add(row.answer_tokens))
    };
    let thinking: u64 = row
        .round_reasoning
        .iter()
        .map(|tokens| u64::from(*tokens))
        .sum();
    let (parts, _) = close_request_parts(
        &raw,
        prompt_target,
        row.answer_tokens,
        thinking as usize,
        unreported,
        server_gap_slot(row),
    );
    parts
}

/// Which category the endpoint's otherwise-unattributed prompt tokens join.
///
/// A server-side search's full pages never reach the local estimate, so the gap
/// belongs to 工具结果; without a server call the gap is the engine replaying
/// history, which is 注入.
pub(crate) fn server_gap_slot(row: &RequestRow) -> ContextKind {
    if row.round_server_calls.iter().any(|calls| *calls > 0) {
        ContextKind::ToolResults
    } else {
        ContextKind::Injected
    }
}

/// The round a server-side call id names (`round-2-3` → round 2), if any.
///
/// Local tool calls carry provider ids (`call_00_…`) and are counted by the
/// engine's per-round figure; server tools get a positional id instead, which is
/// the only record of which round they ran in.
pub(crate) fn server_round(call: &str) -> Option<u32> {
    let rest = call.strip_prefix("round-")?;
    let (round, _) = rest.split_once('-')?;
    round.parse().ok()
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
pub(crate) fn request_rows(
    messages: &[ChatMessage],
    stored: &std::collections::HashMap<u64, crate::chats::StoredRequest>,
) -> (Vec<RequestRow>, RequestSums) {
    let mut rows: Vec<RequestRow> = Vec::new();
    let mut sums = RequestSums {
        composition: vec![0usize; ContextKind::ALL.len()],
        ..RequestSums::default()
    };
    let mut tools = 0u64;
    // Thinking rows of the request being counted, as `(round, text)`, so the
    // per-round figures follow where the thinking actually happened.
    let mut thinking: Vec<(u32, String)> = Vec::new();
    // Server-side calls of the request being counted, by the round their id names.
    let mut server_calls: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for message in messages {
        match message.role {
            Role::Thinking => thinking.push((message.round, message.text.clone())),
            Role::Tool => {
                tools += 1;
                if let Some(round) = server_round(&message.call) {
                    *server_calls.entry(round).or_insert(0) += 1;
                }
            }
            Role::Assistant => {
                let turn = rows.len() as u64 + 1;
                let rounds = message
                    .rounds
                    .max(message.round_tokens.len() as u32)
                    .max(thinking.iter().map(|(round, _)| *round).max().unwrap_or(0))
                    .clamp(1, ROUND_LIMIT);
                let (round_reasoning, round_reasoning_text) =
                    round_thinking(message, rounds, &thinking);
                let mut round_server_calls = vec![0u32; rounds as usize];
                for (round, count) in std::mem::take(&mut server_calls) {
                    if let Some(slot) = (round as usize)
                        .checked_sub(1)
                        .and_then(|index| round_server_calls.get_mut(index))
                    {
                        *slot = count;
                    }
                }
                let (composition, round_composition) =
                    request_composition(message, turn, stored, &round_reasoning);
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
                    model_ms: message.model_ms,
                    tool_ms: message.tool_ms,
                    tokens: message.tokens,
                    prompt_tokens: prompt,
                    answer_tokens: answer,
                    cache_hits: message.cache_hits,
                    context: message.context,
                    round_reasoning,
                    round_reasoning_text,
                    // Capped so a damaged save cannot size the round list for
                    // billions of rows; zero still means the engine said nothing.
                    rounds: message.rounds.min(ROUND_LIMIT),
                    round_tokens: message.round_tokens.clone(),
                    round_server_calls,
                    tools,
                    compaction_layer: message.compaction_layer.clone(),
                    composition,
                    closed_composition: Vec::new(),
                    round_composition,
                });
                // Close the split on the endpoint's figures once, and let the
                // whole-conversation table, the ring and the timeline read the
                // same numbers the request card shows for this answer.
                let row = rows.last_mut().expect("just pushed");
                let closed = closed_request_parts(row);
                row.closed_composition = closed;
                for (slot, tokens) in sums.composition.iter_mut().zip(&row.closed_composition) {
                    *slot = slot.saturating_add(*tokens);
                }
                sums.estimate = sums.estimate.saturating_add(
                    row.composition
                        .iter()
                        .map(|tokens| u64::from(*tokens))
                        .sum::<u64>(),
                );
                tools = 0;
                thinking.clear();
            }
            _ => {}
        }
    }
    rows.reverse();
    // Every row stays in the report: the panel draws the newest
    // [`REQUEST_ROWS`] unless the reader asks for the whole list, and the
    // timeline follows the same window.
    (rows, sums)
}

/// Where the engine starts compacting, as a share of the model window.
///
/// Mirrors `compactLimits` in the engine (`pressureTokens = window * 0.8`), so
/// the bar's marker is the line that actually triggers work instead of a guess.
pub(crate) const WINDOW_PRESSURE: f32 = 0.8;

/// How one request sat in the model window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowUsage {
    /// Share of the window the request filled, 0–1.
    pub(crate) filled: f32,
    /// Share left before the compaction line; zero once it is past it.
    pub(crate) headroom: f32,
    /// Share the connection reserved for the answer, pinned to the far right.
    pub(crate) reserve: f32,
    /// The compaction line itself, 0–1.
    pub(crate) pressure: f32,
}

/// Read one request against the model window; `None` when the window is unknown,
/// so a model this build has no size for shows no bar.
pub(crate) fn window_usage(filled_tokens: u64, window: u64, output: u64) -> Option<WindowUsage> {
    if window == 0 {
        return None;
    }
    let scale = window as f32;
    let filled = (filled_tokens.min(window) as f32 / scale).clamp(0., 1.);
    let reserve = (output.min(window) as f32 / scale).clamp(0., 1.);
    Some(WindowUsage {
        filled,
        headroom: (WINDOW_PRESSURE - filled).max(0.),
        reserve,
        pressure: WINDOW_PRESSURE,
    })
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
            Role::Note | Role::Approval | Role::Thinking => {}
            // Asking or submitting a plan does not cost tool traffic.
            Role::Question | Role::Plan => {}
        }
    }
    items.reverse();
    items
}

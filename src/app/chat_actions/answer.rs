//! Finishing one answer: the transcript row, the request line and the ledger.
//!
//! It lives apart from the event pump because it is the one branch that writes
//! three things (the message, `requests.jsonl` and the statistics ledger) and
//! touches nearly every figure the window shows.

use super::*;

impl DshFlowApp {
    /// Turn the engine's final event into the answer the reader keeps.
    ///
    /// Writes the assistant row, appends the request detail and the ledger record,
    /// then starts the compaction of turns the question pushed out of the window.
    /// @param slot - the pending answer this event belongs to.
    /// @param chat_index - the conversation it is being written into.
    /// @param event - the `Done` event, taken by value so its fields can move.
    /// @param cx - the window context, for the compaction task.
    pub(super) fn finish_answer(
        &mut self,
        slot: usize,
        chat_index: usize,
        event: AiEvent,
        cx: &mut Context<Self>,
    ) {
        let AiEvent::Done {
            text,
            usage,
            context,
            cache_hits,
            rounds,
            round_tokens,
            round_reasoning,
            compaction,
            model_ms,
            tool_ms,
            reasoning,
            web_searches,
            web_fetches,
            server_tool_calls,
        } = event
        else {
            return;
        };
        // Say what the engine's compaction did, above the answer, so
        // a suddenly shorter context is explained rather than
        // mysterious. An empty note means nothing happened.
        let note = compaction
            .as_ref()
            .map(|report| report.note())
            .unwrap_or_default();
        // The same fact, kept as data: the panel flags and the step
        // brief read these fields instead of matching the note's
        // prose, so a wording change cannot lose the event.
        let compaction_layer = compaction
            .as_ref()
            .map(|report| report.layer.clone())
            .unwrap_or_default();
        let compaction_saved = compaction
            .as_ref()
            .map(|report| report.before.saturating_sub(report.after))
            .unwrap_or(0);
        if !note.is_empty()
            && let Some(chat) = self.chats.get_mut(chat_index)
        {
            chat.messages.push(ChatMessage::note(note, false));
        }
        let pending = &self.pending[slot];
        let model = pending.model.clone();
        let elapsed = pending.request.elapsed_ms();
        let mut items = pending.items.clone();
        let schemas = pending.schemas.clone();
        let tool_names = pending.tool_names.clone();
        // The provider may report the whole thinking only with the
        // answer (no live fragments). No row means no readable
        // record, so one is written here in the same shape.
        let thinking_text = if reasoning.trim().is_empty() {
            pending.reasoning.clone()
        } else {
            reasoning.clone()
        };
        // The number the panel counts requests by: this answer's
        // place among the answers of this conversation. The same
        // figures feed the request line and the statistics ledger,
        // so they are gathered once, when the answer is built.
        struct Answered {
            turn: u64,
            tokens: u64,
            context: u64,
            calls: u64,
            cache_hits: u64,
            prompt_tokens: u64,
            model_ms: u64,
            tool_ms: u64,
            reasoning: String,
            reasoning_tokens: u32,
            web_searches: u32,
            web_fetches: u32,
            tool_names: Vec<String>,
        }
        let mut done: Option<Answered> = None;
        if let Some(chat) = self.chats.get_mut(chat_index) {
            // A provider that reports the thinking only with the
            // answer sends no live fragments; without this the text
            // would have no row to live in.
            if self.pending[slot].thinking_rows == 0 && !thinking_text.trim().is_empty() {
                chat.messages
                    .push(ChatMessage::thinking(thinking_text.clone(), rounds.max(1)));
            }
            // What the tools returned during this question is
            // replayed into its next round, so it belongs to what
            // this question cost — and `chat.json` only kept the
            // totals, so it has to come from the transcript.
            items.extend(posted_tool_items(&chat.messages));
            // The engine reports its tool schemas only once a run has
            // started, so the record was written with the previous run's
            // list: replace it with what this run actually sent (a server
            // or plugin switched off between two questions drops out here).
            set_tool_schemas(&mut items, &schemas);
            // The text this request finally wrote: no later prompt
            // replays it, so the request's own record has to keep it
            // or the panel would never show the answer.
            if let Some(item) = output_item(&text) {
                items.push(item);
            }
            // Every round sends the whole prompt again, so the
            // request's figures are the sum of its rounds, not the
            // pieces counted once.
            let round_parts = round_estimates(&items, &round_tokens, rounds);
            let (composition, tools_tokens) = if round_parts.is_empty() {
                let composition = composition_of(&items);
                let tools_tokens: usize = items
                    .iter()
                    .filter(|item| item.kind == ContextKind::ToolResults)
                    .map(|item| item.tokens)
                    .sum();
                (composition, tools_tokens)
            } else {
                let totals = summed_rounds(&round_parts);
                let tool_index = ContextKind::ALL
                    .iter()
                    .position(|kind| *kind == ContextKind::ToolResults)
                    .unwrap_or(0);
                (
                    composition_counts(&totals),
                    totals.get(tool_index).copied().unwrap_or(0),
                )
            };
            // Thinking tokens are output the bill includes, so they
            // join the request's make-up under their own category
            // instead of disappearing into the answer.
            let reasoning_total: u32 = round_reasoning
                .iter()
                .fold(0u32, |sum, tokens| sum.saturating_add(*tokens));
            let mut composition = composition;
            if reasoning_total > 0
                && let Some(slot) = ContextKind::ALL
                    .iter()
                    .position(|kind| *kind == ContextKind::Reasoning)
                && let Some(cell) = composition.get_mut(slot)
            {
                *cell = cell.saturating_add(reasoning_total);
            }
            let turn = chat
                .messages
                .iter()
                .filter(|message| message.role == Role::Assistant)
                .count() as u64
                + 1;
            // The per-round list counts the tool calls the question
            // made; the statistics page needs that total without
            // opening every stored item.
            let calls: u64 = round_tokens
                .iter()
                .map(|(_, _, calls)| u64::from(*calls))
                .sum();
            let mut message = ChatMessage::assistant(text);
            message.model = model.clone();
            message.elapsed_ms = elapsed;
            message.model_ms = model_ms;
            message.tool_ms = tool_ms;
            message.tokens = usage.map_or(0, |usage| usage.total());
            message.context = context;
            message.cache_hits = cache_hits;
            message.rounds = rounds;
            message.round_tokens = round_tokens;
            message.round_reasoning = round_reasoning;
            message.server_tool_calls = server_tool_calls;
            message.composition = composition;
            message.tools_tokens = tools_tokens.min(u32::MAX as usize) as u32;
            message.compaction_layer = compaction_layer;
            message.compaction_saved = compaction_saved;
            // Thinking is written as its own rows; the answer keeps only what it said.
            // Prompt tokens the question sent, summed over its
            // rounds: the denominator of its cache rate, and the
            // figure the ledger needs to report one later.
            let prompt_tokens: u64 = message
                .round_tokens
                .iter()
                .map(|(prompt, _, _)| *prompt)
                .sum();
            done = Some(Answered {
                turn,
                tokens: message.tokens,
                context: message.context,
                calls,
                cache_hits: message.cache_hits,
                prompt_tokens,
                model_ms: message.model_ms,
                tool_ms: message.tool_ms,
                reasoning: thinking_text.clone(),
                reasoning_tokens: reasoning_total,
                web_searches,
                web_fetches,
                tool_names,
            });
            chat.messages.push(message);
            settle_interactions(chat, "已失效");
            chat.touch();
        }
        self.pending.remove(slot);
        if let (Some(answered), Some((folder, title))) = (
            done,
            self.chats
                .get(chat_index)
                .map(|chat| (chat.dir.clone(), chat.title.clone())),
        ) {
            let at_ms = now_ms();
            // A line per request: the transcript keeps the totals and
            // the full text lives here, so the request can be opened
            // again later without bloating `chat.json`.
            let request = crate::chats::StoredRequest {
                turn: answered.turn,
                at_ms,
                model,
                tokens: answered.tokens,
                context: answered.context,
                calls: answered.calls,
                cache_hits: answered.cache_hits,
                prompt_tokens: answered.prompt_tokens,
                model_ms: answered.model_ms,
                tool_ms: answered.tool_ms,
                reasoning: answered.reasoning,
                reasoning_tokens: answered.reasoning_tokens,
                web_searches: answered.web_searches,
                web_fetches: answered.web_fetches,
                tool_names: answered.tool_names,
                items: items
                    .iter()
                    .map(|item| crate::chats::StoredRequestItem {
                        kind: item.kind.key().to_string(),
                        label: item.label.clone(),
                        text: item.text.clone(),
                        tokens: item.tokens as u64,
                    })
                    .collect(),
            };
            if let Err(error) = crate::chats::append_request(&self.ai.data_dir(), &folder, &request)
            {
                self.banner = Some(Banner::error(error));
            }
            // The statistics page reads this independent ledger, so a
            // conversation deleted later no longer removes its figures.
            let record = super::super::stats::ledger_record(&folder, &title, &request);
            if let Err(error) =
                crate::stats_store::append(&crate::stats_store::ledger_dir(), &record)
            {
                self.banner = Some(Banner::error(error));
            } else {
                self.stats_generation = self.stats_generation.saturating_add(1);
            }
        }
        self.save_chats();
        // A question may have written or updated a workflow folder; rescans here
        // so the card appears without a restart.
        self.refresh_workflows();
        // A connection made during this question writes its skill files; refresh the
        // page if it is the one on screen.
        if self.view == View::Mcp {
            self.refresh_mcp(cx);
        }
        // The turns this question pushed past the window are what
        // the last compaction layer condenses; it runs on a worker
        // thread, so the reader's next question is not held up.
        let _ = self.compact_history(chat_index, cx);
    }
}

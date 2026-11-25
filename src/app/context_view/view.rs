//! The page itself: tabs, its two scroll bars, and the layout that ties it together.

use super::*;

/// Width of the narrow ring card beside the totals table.
pub(crate) const TOTALS_CHART_WIDTH: f32 = 300.0;

/// Height the ring card and the totals table share.
pub(crate) const TOTALS_CARD_HEIGHT: f32 = 280.0;

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
                    "本次预算：最多{} 轮模型往返、{} 次工具调用。当前权限：{}（{}）；命令路径沙箱：{sandbox}。",
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
            // tool traffic, so 「工具结果」only lists the tools this question
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

    /// What the conversation holds for the 上下文tab.
    ///
    /// The request-by-request detail comes from `requests.jsonl`; this only
    /// gathers the figures the whole panel shares —the window, the counts, the
    /// replayed history, and one row per answer.
    pub(crate) fn context_report(&self) -> ContextReport {
        let chat = self.chat();
        let model = self.chat_model();
        // The same numbers the history window uses, so the limit shown here is
        // the one that actually trims.
        let (_, _, history_tokens, _) = history_stats(&chat.messages);
        // An explicit connection setting wins over the name table, so the panel
        // and the engine compact at the same point.
        let window = self.ai.active().map_or_else(
            || crate::ai::context_window(&model),
            |connection| connection.window_tokens(),
        );
        let output = self
            .ai
            .active()
            .map_or(0, |connection| u64::from(connection.max_tokens));
        let history_budget = MAX_HISTORY_TOKENS;
        let stored = crate::chats::load_requests(&self.ai.data_dir(), &chat.dir);
        let (requests, sums) = request_rows(&chat.messages, &stored);
        // One pass for the bill and the three counts the chips show, instead of
        // four scans over the transcript.
        let mut spend = 0u64;
        let mut asked = 0u64;
        let mut steps = 0u64;
        let mut replies = 0u64;
        for message in &chat.messages {
            spend = spend.saturating_add(message.tokens);
            match message.role {
                Role::User => asked += 1,
                Role::Tool => steps += 1,
                Role::Assistant => replies += 1,
                _ => {}
            }
        }
        ContextReport {
            window,
            asked,
            history_tokens,
            steps,
            replies,
            output,
            history_budget,
            spend,
            total_composition: sums.composition,
            estimate_total: sums.estimate,
            prompt_total: sums.prompt,
            answer_total: sums.answer,
            unreported: sums.unreported,
            requests,
            noted_compactions: noted_compactions(&chat.messages),
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

    /// The handle one of the context panel's bars scrolls.
    pub(crate) fn bar_handle(&self, kind: BarKind) -> &ScrollHandle {
        match kind {
            BarKind::Totals => &self.totals_scroll,
            BarKind::Item => &self.context_item_scroll,
        }
    }

    /// The live drag of one of the context panel's bars, if any.
    pub(crate) fn bar_drag(&self, kind: BarKind) -> Option<(Pixels, Pixels)> {
        match kind {
            BarKind::Totals => self.totals_drag,
            BarKind::Item => self.context_item_drag,
        }
    }

    /// Remember (or clear) the live drag of one of the bars.
    pub(crate) fn set_bar_drag(&mut self, kind: BarKind, drag: Option<(Pixels, Pixels)>) {
        match kind {
            BarKind::Totals => self.totals_drag = drag,
            BarKind::Item => self.context_item_drag = drag,
        }
    }

    /// A thin scrollbar beside one of the panel's fixed-height boxes.
    ///
    /// The totals table and an expanded item both hold content that can outgrow
    /// their height, so they share this bar: it says the rest is there, carries
    /// the wheel, and lets the rows be pulled into view instead of the last one
    /// being cut off.
    pub(crate) fn render_bar(
        &self,
        kind: BarKind,
        id: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scroll = self.bar_handle(kind).clone();
        let Some((track, thumb, max, _)) = tool_bar_metrics(&scroll) else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-scroll.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = self.bar_drag(kind).is_some();
        let handle = scroll.clone();
        let wheel = scroll;
        div()
            .id(id)
            .w(px(8.))
            .flex_shrink_0()
            .h(track + px(4.))
            .flex()
            .flex_col()
            .items_center()
            .block_mouse_except_scroll()
            .cursor_pointer()
            // A wheel over the bar scrolls the rows it represents; the bar is
            // part of the box, so the panel behind never takes the event.
            .on_scroll(cx.listener(move |_view, event: &ScrollEvent, window, cx| {
                if wheel_scroll(&wheel, event, window) {
                    cx.notify();
                }
                cx.stop_propagation();
            }))
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
                    view.set_bar_drag(kind, Some((event.position.y, handle.offset().y)));
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

    /// Follow the pointer while one of the panel's bars is held.
    pub(crate) fn drag_bar(&mut self, kind: BarKind, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((start_y, start_offset)) = self.bar_drag(kind) else {
            return;
        };
        let scroll = self.bar_handle(kind).clone();
        let Some((track, thumb, max, _)) = tool_bar_metrics(&scroll) else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = scroll.offset();
        scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// Follow the pointer while the totals table's bar is held.
    pub(crate) fn drag_totals_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        self.drag_bar(BarKind::Totals, pointer_y, cx);
    }

    /// Follow the pointer while the expanded item's bar is held.
    pub(crate) fn drag_item_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        self.drag_bar(BarKind::Item, pointer_y, cx);
    }

    /// The 上下文tab: what one request was made of, by category.
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
        // The report is cached, so everything it reads has to be in the key:
        // the transcript's own state, the tool set, and the model and answer
        // reserve the active connection contributes.
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
            text_fingerprint(&self.chat_model()),
            self.ai
                .active()
                .map_or(0, |connection| u64::from(connection.max_tokens)),
        );
        let mut cache = self.context_cache.borrow_mut();
        if !matches!(cache.as_ref(), Some((cached, _)) if *cached == signature) {
            *cache = Some((signature, self.context_report()));
        }
        let report = &cache.as_ref().expect("just filled").1;
        // The table and the timeline share one window: the newest rows, or the
        // whole conversation once the reader asks for it.
        let shown_requests: &[RequestRow] = if self.context_all_requests {
            &report.requests
        } else {
            &report.requests[..report.requests.len().min(REQUEST_ROWS)]
        };
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
            *detail = Some((dir.clone(), row.turn, row.context, row.tokens, pieces));
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
        let upto = round_pick.map_or(0, |round| {
            row.round_tokens
                .iter()
                .take(round as usize - 1)
                .map(|(_, _, calls)| *calls as usize)
                .sum()
        });
        // Where thinking sits in every category vector; the picked round's own
        // figure and words replace whatever a shared part carried.
        let reasoning_slot = ContextKind::ALL
            .iter()
            .position(|kind| *kind == ContextKind::Reasoning)
            .unwrap_or(0);
        // A category opens only where the pieces of the round on show were
        // stored; a request added up over several rounds has no single copy of
        // the prompt to expand.
        let allow_open = round_pick.is_some() || row.round_composition.len() <= 1;
        // The split on show is chosen below, once the endpoint's prompt target
        // is known: a picked round closes on its own figures, the whole request
        // borrows the figures already closed for the totals table.
        // Two denominators, because they answer two different questions. The window
        // share must stay on the prompt alone: every round sends the whole prompt
        // again, so a billed total would climb past 100% on a tool-using question. The
        // categories, on the other hand, describe what was *billed*, answer tokens
        // included, so they add up to the same figure the "计费" chip shows.
        // What the head line names as the endpoint's figure: the round's prompt,
        // or every round's prompt added up. Never the bill, so "端点" stays true.
        //
        // A request whose endpoint reported no split at all falls back to the
        // local estimate, the only evidence of how big its prompt was. Once the
        // endpoint did report, its own figure stands alone: a round it said it
        // took but never split is already inside the bill, so counting its
        // estimate here as well would make the request look bigger than it was.
        let estimate: u64 = if row.round_tokens.is_empty() {
            row.round_composition
                .iter()
                .map(|round| round.iter().sum::<usize>() as u64)
                .sum()
        } else {
            0
        };
        let shown = match round_pick {
            Some(round) => row
                .round_tokens
                .get(round as usize - 1)
                .map_or(0, |(prompt, _, _)| *prompt),
            None => row.prompt_tokens.max(estimate),
        };
        // The rows follow the endpoint's own figures, the way dsh-context closes
        // its breakdown: the prompt side is measured, the answer side is measured,
        // and whatever the estimate could not see (the engine's replay) joins the
        // category it came from instead of becoming an "other" row.
        let reported_answer = match round_pick {
            Some(round) => row
                .round_tokens
                .get(round as usize - 1)
                .map_or(0, |(_, answer, _)| *answer),
            None => row.answer_tokens,
        };
        let reported_thinking: u64 = match round_pick {
            Some(round) => row
                .round_reasoning
                .get(round as usize - 1)
                .map_or(0, |tokens| u64::from(*tokens)),
            None => row
                .round_reasoning
                .iter()
                .map(|tokens| u64::from(*tokens))
                .sum(),
        };
        // A bill with no per-round split has nowhere to put its remainder; the
        // round list names it instead (`端点未分轮的用量`), so it is not lost.
        let unreported = if round_pick.is_none() && !row.round_tokens.is_empty() {
            row.tokens
                .saturating_sub(row.prompt_tokens.saturating_add(row.answer_tokens))
        } else {
            0
        };
        // The split on show: the picked round's own raw estimate closed on that
        // round's figures, or the whole request already closed — the same
        // figures the totals table, the ring and the timeline read. The whole
        // request is borrowed: cloning it on every paint was pure duplication.
        let (sized, raw_sum): (std::borrow::Cow<'_, [usize]>, u64) = match round_pick {
            None => (std::borrow::Cow::Borrowed(&row.closed_composition), 0),
            Some(round) => {
                let index = round as usize - 1;
                let thinking = row.round_reasoning.get(index).copied().unwrap_or(0) as usize;
                let mut pieces = row
                    .round_composition
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                // The picked round keeps its own thinking even when the shared
                // pieces carried something else.
                if pieces.len() == ContextKind::ALL.len() {
                    pieces[reasoning_slot] = thinking;
                }
                let raw = if pieces.iter().any(|tokens| *tokens > 0) {
                    pieces
                } else if thinking > 0 {
                    // The endpoint reported no split for this round: show what is
                    // known (its thinking), never the whole request's make-up.
                    let mut only = vec![0usize; ContextKind::ALL.len()];
                    only[reasoning_slot] = thinking;
                    only
                } else {
                    Vec::new()
                };
                let raw_sum = raw.iter().sum::<usize>() as u64;
                let closed = if raw.len() != ContextKind::ALL.len() {
                    std::borrow::Cow::Owned(vec![0usize; ContextKind::ALL.len()])
                } else {
                    // A picked round closes on its own figures: what the estimate
                    // could not see came from the provider's own search when one
                    // ran, otherwise from the engine's replay. The bill remainder
                    // belongs to the whole request, not to one of its rounds.
                    std::borrow::Cow::Owned(
                        close_request_parts(
                            &raw,
                            shown,
                            reported_answer,
                            reported_thinking as usize,
                            0,
                            server_gap_slot(row),
                        )
                        .0,
                    )
                };
                (closed, raw_sum)
            }
        };
        let sized = sized.as_ref();
        // The window card names the prompt that filled it; a round with no
        // reported prompt falls back to the size of the split on show.
        let target = if round_pick.is_some() && shown == 0 {
            raw_sum
        } else {
            shown
        };
        // The stored pieces are shared by every view. A round narrows them — the
        // tool results it could see, the answer it had not written yet, its own
        // thinking — instead of copying every label and body on every paint.
        let reasoning_item = round_pick.and_then(|round| {
            let index = round as usize - 1;
            let thinking = row.round_reasoning.get(index).copied().unwrap_or(0) as usize;
            let words = row.round_reasoning_text.get(index)?;
            (!words.trim().is_empty()).then(|| ContextItem {
                label: "思考过程".to_string(),
                text: words.clone(),
                tokens: thinking,
            })
        });
        let mut views: Vec<PartView<'_>> = Vec::with_capacity(ContextKind::ALL.len());
        for part in detail_parts {
            let items = match round_pick {
                None => PartItems::All(&part.items),
                Some(round) => match part.kind {
                    // A server result is visible in the round that ran it; a local
                    // one only reaches the prompt from the next round on.
                    ContextKind::ToolResults => PartItems::ToolResults {
                        items: &part.items,
                        upto,
                    },
                    // The closing answer was not written in the earlier rounds.
                    ContextKind::Output if round < round_count as u32 => PartItems::All(&[]),
                    ContextKind::Reasoning => match reasoning_item.as_ref() {
                        Some(item) => PartItems::One(item),
                        None => PartItems::All(&[]),
                    },
                    _ => PartItems::All(&part.items),
                },
            };
            let (count, estimated) = items.count_and_estimate();
            let tokens = sized.get(part.kind as usize).copied().unwrap_or(0);
            // Every category that had text keeps its expansion adding up: what
            // the endpoint measured beyond the estimate is one more entry.
            let gap = (count > 0 && tokens > estimated).then_some(tokens - estimated);
            views.push(PartView {
                kind: part.kind,
                entries: count + usize::from(gap.is_some()),
                items,
                tokens,
                gap,
            });
        }
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
        // every request's closed make-up added up, so the arcs are the same
        // shares the request card shows. Scaling only matters for old saves
        // whose closed figures do not reach the bill.
        let ring_total: u64 = report
            .total_composition
            .iter()
            .map(|tokens| *tokens as u64)
            .sum();
        // The center shows what the endpoints billed for the whole conversation;
        // the ring keeps that composition's proportions.
        let ring_target = if report.spend > 0 {
            report.spend
        } else {
            ring_total
        };
        let ring = if report.total_composition.len() == ContextKind::ALL.len() {
            anchor_parts(&report.total_composition, ring_target)
        } else {
            vec![0usize; ContextKind::ALL.len()]
        };
        let ring_sum = ring.iter().sum::<usize>().max(1);
        // The card names itself at the top-left, above the ring. The figures
        // that used to trail the heading (request count, the conversation's
        // clock and its cache share) are gone: at this card's width they ran
        // past its edge, and the ring, the chips below it and the table all
        // carry those numbers.
        let totals_header = div()
            .w_full()
            .min_w(px(0.))
            .text_xs()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(rgb(palette.muted))
            .child("整个对话合计");
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
                            .child(div().flex_shrink_0().size(px(7.)).rounded_sm().bg(rgb(
                                if *tokens > 0 {
                                    kind.color()
                                } else {
                                    palette.border
                                },
                            )))
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
        // The timeline: one stacked bar per plotted request, newest on the right,
        // each bar the make-up of what that request filled. It sits beside the
        // ring so the conversation's make-up and its growth read together, and a
        // bar can be clicked to open the request it belongs to.
        let plotted = &shown_requests[..shown_requests.len().min(TIMELINE_BARS)];
        let points = timeline_points(plotted, &report.noted_compactions);
        let peak = points
            .iter()
            .map(|point| point.context)
            .max()
            .unwrap_or(0)
            .max(1);
        let mut bars = div()
            .flex()
            .flex_row()
            .items_end()
            .gap_1()
            .flex_1()
            .min_h(px(0.))
            .w_full()
            .min_w(px(0.));
        for point in &points {
            let turn = point.turn;
            let height = (point.context as f32 / peak as f32).clamp(0.04, 1.) * TIMELINE_BAR_HEIGHT;
            let mut stack = div()
                .flex()
                .flex_col()
                .justify_end()
                .w_full()
                .h(px(height.max(3.)))
                .overflow_hidden()
                .rounded_sm();
            if point.flat {
                // No recorded make-up (an older save): one neutral block, so the
                // bar still shows the size without inventing categories.
                stack = stack.child(div().w_full().h_full().bg(rgb(palette.border)));
            } else {
                for (colour, share) in &point.segments {
                    let part = (height * share).max(1.);
                    stack = stack.child(div().w_full().h(px(part)).bg(rgb(*colour)));
                }
            }
            let mut column =
                div()
                    .id(format!("timeline-{turn}"))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .flex_1()
                    .min_w(px(2.))
                    .max_w(px(18.))
                    .h(px(TIMELINE_BAR_HEIGHT))
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
                        view.context_item_scroll
                            .set_offset(gpui::point(px(0.), px(0.)));
                        cx.notify();
                    }))
                    // Flags ride above the bar they belong to; at most three, so a
                    // busy request cannot push its bar down.
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_0p5()
                            .flex_shrink_0()
                            .children(point.flags.iter().take(3).map(|flag| {
                                div().size(px(4.)).rounded_full().bg(rgb(flag.color()))
                            })),
                    )
                    .child(stack);
            if turn == row.turn {
                // The request on show gets a soft column behind its bar, so it is
                // found without reading turn numbers.
                column = column.bg(rgb(palette.panel_soft)).rounded_sm();
            }
            bars = bars.child(column);
        }
        // The chart takes every pixel the right column has left, so its bars
        // grow with the ring beside it instead of sitting in a short strip.
        let newest = report.requests.first().map_or(0, |row| row.turn);
        let hidden_bars = shown_requests.len().saturating_sub(TIMELINE_BARS);
        let timeline_note = if points.is_empty() {
            "还没有请求".to_string()
        } else {
            let mut note = format!(
                "最新在右（第{newest} 次）· 峰值{} / 窗口 {}",
                short_count(peak),
                short_count(report.window)
            );
            if hidden_bars > 0 {
                note.push_str(&format!(" · 只画最近{TIMELINE_BARS} 次"));
            }
            note
        };
        // What the dots above the bars mean, and how many bars carry each. Only
        // the flags present in this window are listed, so the legend never grows
        // into a wall of names that do not occur.
        let mut legend = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_wrap()
            .gap_2()
            .w_full()
            .min_w(px(0.));
        for flag in TimelineFlag::ALL {
            let count = points
                .iter()
                .filter(|point| point.flags.contains(&flag))
                .count();
            if count == 0 {
                continue;
            }
            legend = legend
                .child(div().size(px(5.)).rounded_full().bg(rgb(flag.color())))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(format!("{} {count}", flag.label())),
                );
        }
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
                            .child("上下文时间线"),
                    )
                    .child(div().flex_1().min_w(px(0.)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(timeline_note),
                    ),
            )
            .child(bars)
            .child(legend);
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
            .child(chip(report.estimate_total, "分类估算"))
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
                            .child(format!("{} 项", ContextKind::ALL.len())),
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
                            .contained_wheel()
                            .child(table),
                    )
                    .child(self.render_bar(BarKind::Totals, "context-table-bar", palette, cx)),
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
            .h(px(200.))
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
        // The rows carry the endpoint's figures now, so their percentages are
        // shares of this request's bill: the category figures add up to it, each
        // expansion adds up to its category, and the rounds add up to the bill.
        let share_base = sized.iter().sum::<usize>().max(1);
        let note = if row.round_tokens.is_empty() {
            "分类按条目的文本估算；这次请求没有端点回报的分轮用量。".to_string()
        } else {
            let tail = if unreported > 0 {
                format!("；另有{} 未按轮回报，已计入回答侧", short_count(unreported))
            } else {
                String::new()
            };
            // The gap is not always the replay: when a server-side search ran,
            // its full pages are missing locally and the gap is tool results.
            let gap = if row.round_server_calls.iter().any(|calls| *calls > 0) {
                "服务端搜索读页的完整正文本机只存摘要，差额计入「工具结果」"
            } else {
                "本机没有记录的引擎回放计入「注入」"
            };
            format!(
                "分类以端点回报为准：提示侧合计{}，回答{}（含思考{}）{tail}；{gap}。",
                short_count(shown),
                short_count(reported_answer),
                short_count(reported_thinking)
            )
        };
        if share_base > 1 {
            list = list.child(
                div()
                    .px_1()
                    .pb_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child(note),
            );
        }
        for view in &views {
            // A category opens only where the pieces of the round on show were
            // stored; a request added up over several rounds has no single copy
            // of the prompt to expand. The final output is the exception: it was
            // written once, in the closing round, so it is readable even from
            // the summed view. Thinking is the same: one text for the whole
            // question, not a piece per round.
            let output_part = matches!(view.kind, ContextKind::Output | ContextKind::Reasoning);
            let can_open = view.entries > 0 && (allow_open || output_part);
            let open = can_open && self.context_open == Some(view.kind);
            let tokens = view.tokens;
            let request_share = tokens as f64 * 100.0 / share_base as f64;
            let mut row = div()
                .id(("context-part", view.kind as usize))
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
                        .bg(rgb(view.kind.color())),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(view.kind.label()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(match view.kind.empty_note() {
                            Some(note) => note.to_string(),
                            // A tool-result row is the accumulated set, not a
                            // per-round slice: the selected round only carries
                            // the results returned before it started.
                            None if view.kind == ContextKind::ToolResults => {
                                if round_pick.is_some() {
                                    format!("{} 项 · 进入本轮", view.entries)
                                } else {
                                    format!("{} 项 · 整个请求", view.entries)
                                }
                            }
                            None if !allow_open && !output_part => {
                                format!("{round_count} 轮合计")
                            }
                            None => format!("{} 项", view.entries),
                        }),
                )
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
                            "{request_share:.1}% ≈{}",
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
                let kind = view.kind;
                row = row.on_click(cx.listener(move |view, _, _, cx| {
                    view.context_open = if view.context_open == Some(kind) {
                        None
                    } else {
                        Some(kind)
                    };
                    view.context_item = None;
                    view.context_item_scroll
                        .set_offset(gpui::point(px(0.), px(0.)));
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
                if view.kind == ContextKind::ToolResults {
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
                let estimates: Vec<usize> = view
                    .items
                    .iter()
                    .map(|item| item.tokens)
                    .chain(view.gap)
                    .collect();
                let shares = spread_tokens(&estimates, view.tokens);
                // The expansion lists the pieces on show, then the measured gap
                // as one textless entry, exactly as the old pushed item did.
                let mut listed: Vec<Option<&ContextItem>> = view.items.iter().map(Some).collect();
                if view.gap.is_some() {
                    listed.push(None);
                }
                for (item_index, slot) in listed.iter().enumerate() {
                    let (label, text) = match *slot {
                        Some(item) => (item.label.as_str(), item.text.as_str()),
                        None => (gap_label(view.kind), ""),
                    };
                    let shown_tokens = shares.get(item_index).copied().unwrap_or(0);
                    let item_share = if share_base == 0 {
                        0.0
                    } else {
                        shown_tokens as f64 * 100.0 / share_base as f64
                    };
                    let kind = view.kind;
                    let can_expand = !text.trim().is_empty();
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
                        // as "第 1 个结果" rather than as four rounds.
                        .when(view.kind == ContextKind::ToolResults, |this| {
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
                                .child(label.to_string()),
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
                                .child(format!(
                                    "{} token · {} 字",
                                    short_count(shown_tokens),
                                    text.chars().count()
                                )),
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
                                view.context_item_scroll
                                    .set_offset(gpui::point(px(0.), px(0.)));
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
                                this.child(appear(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_2()
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
                                                .flex()
                                                .flex_col()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .max_h(px(DETAIL_MAX_HEIGHT - 2.))
                                                .min_h(px(0.))
                                                .overflow_y_scroll()
                                                .track_scroll(&self.context_item_scroll)
                                                .contained_wheel()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .whitespace_normal()
                                                .child(text.to_string()),
                                        )
                                        .child(self.render_bar(
                                            BarKind::Item,
                                            "context-item-bar",
                                            palette,
                                            cx,
                                        )),
                                    SharedString::from(format!(
                                        "context-item-body-{}-{item_index}",
                                        kind.key()
                                    )),
                                    4.,
                                ))
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
        // A request picked earlier can sit outside the newest page — the switch
        // went back to 30, or a new answer pushed it out. The detail card above
        // still shows it, so the list says where it went instead of looking like
        // a selection that vanished.
        let off_window = self
            .context_request
            .filter(|turn| !shown_requests.iter().any(|row| row.turn == *turn));
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
                    .child(column(52., "缓存"))
                    .child(column(60., "回答"))
                    .child(column(72., "计费"))
                    .child(column(64., "占累计"))
                    .child(column(56., "工具"))
                    .child(column(84., "末轮上下文")),
            );
        if let Some(turn) = off_window {
            table = table.child(muted(format!(
                "正在看第 {turn} 次请求，它不在当前列出的范围内；点右上角「全部」可以在列表里看到它。"
            )));
        }
        let total_spend = report.spend.max(1) as f64;
        for row in shown_requests {
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
                        view.context_item_scroll
                            .set_offset(gpui::point(px(0.), px(0.)));
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
                    // Cached prompt tokens: how much of this request the endpoint could serve
                    // from its own cache, which is what makes it cheap.
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(52.))
                            .text_xs()
                            .text_color(rgb(if row.cache_hits > 0 {
                                palette.text
                            } else {
                                palette.faint
                            }))
                            .child(match (row.cache_hits, prompts) {
                                (0, _) => "—".to_string(),
                                (hits, 0) => short_count(hits),
                                (hits, prompts) => format!("{}%", hits * 100 / prompts),
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
        } else if !self.context_all_requests && report.requests.len() > REQUEST_ROWS {
            table = table.child(muted(format!(
                "只列最近{REQUEST_ROWS} 次请求（这段对话共{} 次）；点右上角「全部」看完整列表。",
                report.requests.len()
            )));
        }
        // What the categories below describe: the whole request, or one of its
        // rounds picked at the title.
        // How the picked request sat in the model window: the same eight figures the
        // list below shows, then the room left before compaction and the output
        // the connection reserved. The bar is drawn only when the window size is
        // known, and the marker is the engine's own 80% line.
        let window_card = window_usage(target, report.window, report.output).map(|usage| {
            // The bar is the prompt's make-up, not the bill: answer tokens never
            // filled the window, so the output slot is left out of the scale.
            let parts = window_parts(sized, target);
            let scale = parts.iter().sum::<usize>().max(1) as f32;
            let mut categories: Vec<(u32, f32)> = ContextKind::ALL
                .iter()
                .zip(&parts)
                .map(|(kind, tokens)| (kind.color(), (*tokens as f32 / scale) * usage.filled))
                .collect();
            // Pieces the list below could not attribute still filled the window:
            // an older save, or an estimate short of the endpoint's own count.
            // Painting the remainder keeps the filled edge where the number says.
            let covered: f32 = categories.iter().map(|(_, share)| *share).sum();
            if covered + 0.0001 < usage.filled {
                categories.push((palette.faint, usage.filled - covered));
            }
            render_window_usage(
                &categories,
                usage,
                target,
                report.window,
                report.output,
                palette,
            )
        });
        let detail_head = div()
            .w_full()
            .min_w(px(0.))
            .text_xs()
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(rgb(palette.muted))
            .child(format!("第{} 次请求的构成", row.turn));
        // Every round is listed in the card itself, the total on top: a popup
        // turned comparing rounds into a two-step action, and the engine caps a
        // question's round trips at a short list anyway.
        let round_list = if round_count >= 1 {
            let tools_note = if row.tools > 0 {
                format!(" · {} 个工具", row.tools)
            } else {
                String::new()
            };
            let mut rounds: Vec<AnyElement> = Vec::new();
            rounds.push(context_round_row(
                0,
                format!("合计 {round_count} 轮往返"),
                if row.prompt_tokens > 0 {
                    format!("Σ 提示 {}{tools_note}", short_count(row.prompt_tokens))
                } else {
                    format!("各轮估算{tools_note}")
                },
                round_pick.is_none(),
                None,
                palette,
                cx,
            ));
            for index in 0..round_count {
                let round = index as u32 + 1;
                let (prompt, completion, calls) = row
                    .round_tokens
                    .get(index)
                    .map_or((0, 0, 0), |(prompt, completion, calls)| {
                        (*prompt, *completion, *calls)
                    });
                let server = row.round_server_calls.get(index).copied().unwrap_or(0);
                let tools_note = match (calls, server) {
                    (_, 0) => format!("{calls} 个工具"),
                    (0, server) => format!("{server} 个服务端调用"),
                    (calls, server) => {
                        format!(
                            "{} 个工具（{} 本地 + {server} 服务端）",
                            calls + server,
                            calls
                        )
                    }
                };
                rounds.push(context_round_row(
                    round as usize,
                    format!("第{round} 轮往返"),
                    if prompt > 0 {
                        format!(
                            "提示 {} / 回答 {} · {tools_note}",
                            short_count(prompt),
                            short_count(completion)
                        )
                    } else {
                        format!("未回报用量· {tools_note}")
                    },
                    round_pick == Some(round),
                    Some(round),
                    palette,
                    cx,
                ));
            }
            // The endpoint sometimes reports only a total: that remainder is a
            // row of its own, so the rounds still add up to the request's bill.
            let unreported = row
                .tokens
                .saturating_sub(row.prompt_tokens.saturating_add(row.answer_tokens));
            if unreported > 0 {
                rounds.push(context_round_row(
                    round_count + 1,
                    "端点未分轮的用量".to_string(),
                    format!("合计 {}（已计入请求计费）", short_count(unreported)),
                    false,
                    None,
                    palette,
                    cx,
                ));
            }
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .w_full()
                    .min_w(px(0.))
                    .children(rounds),
            )
        } else {
            None
        };
        // What the selected request actually did, read from its own tool rows: the
        // table's tool count and this list cover the same span.
        //
        // Reading them parses every call's arguments, and a single `write` can
        // carry megabytes of file text; scrolling the panel repaints on every
        // frame, so the list is kept until the transcript grows past it instead
        // of being rebuilt from scratch each time.
        let mut actions_cache = self.context_actions.borrow_mut();
        let actions = {
            let fresh = matches!(
                actions_cache.as_ref(),
                Some((cached_dir, cached_turn, cached_len, _))
                    if *cached_dir == dir
                        && *cached_turn == row.turn
                        && *cached_len == chat.messages.len()
            );
            if !fresh {
                *actions_cache = Some((
                    dir.clone(),
                    row.turn,
                    chat.messages.len(),
                    request_actions(&chat.messages, row.turn),
                ));
            }
            &actions_cache.as_ref().expect("just filled").3
        };
        let actions_card =
            (!actions.is_empty()).then(|| render_actions_card(row.turn, actions, palette));
        // The engine's own clock, split into model calls and tool runs; the rest
        // of the answer's time is the leftover the card names as 其他.
        let timing_card = request_timing(row.model_ms, row.tool_ms, row.elapsed_ms)
            .map(|timing| render_timing_card(row.turn, timing, palette));
        // Plugin panels are built once per request, before its first round, so
        // they describe the whole request. The newest request still has them —
        // the next question rebuilds them — and a single-round view must not
        // claim they belong to that round. While a question is in flight its
        // panels arrive before its request row does, so the card is shown under
        // the previous row and its heading says which request it describes.
        let newest_turn = report.requests.first().map(|row| row.turn);
        let asking = self.active_pending().is_some();
        let panels_card = self
            .plugin_panels
            .get(&chat.dir)
            .filter(|panels| {
                !panels.is_empty()
                    && round_pick.is_none()
                    && (asking || newest_turn == Some(row.turn))
            })
            .map(|panels| render_panels_card(panels, palette, asking));
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
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .w_full()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .px_1()
                                            .text_xs()
                                            .font_weight(gpui::FontWeight::BOLD)
                                            .text_color(rgb(palette.muted))
                                            .child("请求列表（点一条看它的构成）"),
                                    )
                                    .child(
                                        div()
                                            .id("context-requests-window")
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded_full()
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.context_all_requests =
                                                    !view.context_all_requests;
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(if self
                                                        .context_all_requests
                                                    {
                                                        palette.accent
                                                    } else {
                                                        palette.muted
                                                    }))
                                                    .child(if self.context_all_requests {
                                                        format!("全部 {} 次", report.requests.len())
                                                    } else {
                                                        format!("最近{REQUEST_ROWS} 次")
                                                    }),
                                            ),
                                    ),
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
                            .when_some(window_card, |this, bar| this.child(bar))
                            .when_some(round_list, |this, list| this.child(list))
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
                                            "一轮是一次模型往返，一轮可以发出多次工具调用；第N 轮只包含进入它之前已返回的工具结果，本轮自己的结果出现在下一轮。",
                                        ),
                                )
                            })
                            .child(list),
                    )
                    .when_some(timing_card, |this, card| this.child(card))
                    .when_some(actions_card, |this, card| this.child(card))
                    .when_some(panels_card, |this, card| this.child(card)),
            )
            .into_any_element()
    }
}

//! The conversation view: header, transcript scrolling, selection, and copying.

use super::*;

impl DshFlowApp {
    /// The assistant: transcript and the floating composer.
    pub(crate) fn render_chat(
        &self,
        palette: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let chat = self.chat();
        let mut rows: Vec<AnyElement> = Vec::new();
        // Long conversations paint only their newest window; the chip above it
        // adds one row, which the item indices below account for.
        let from = self.chat_window_start();
        // The find bar paints its hits on the message it is pointing at: one
        // entry per frame, not every match in the conversation.
        let find_needle = self
            .chat_find_open
            .then(|| self.chat_find.text.trim().to_lowercase())
            .filter(|needle| !needle.is_empty());
        let find_message = find_needle
            .as_ref()
            .and_then(|_| self.current_find_message());
        if chat.messages.is_empty() {
            rows.push(self.render_chat_empty(palette));
        } else {
            if from > 0 {
                rows.push(self.render_earlier_chip(from, chat.messages.len(), palette, cx));
            }
            for (index, message) in chat.messages.iter().enumerate().skip(from) {
                let find = (find_message == Some(index))
                    .then_some(find_needle.as_deref())
                    .flatten();
                rows.push(self.render_message(index, message, palette, find, window, cx));
            }
        }
        let active_dir = self.chat().dir.clone();
        if let Some(pending) = self.active_pending() {
            // While a tool runs the transcript's last row is the tool call, so
            // say what is happening instead of a flat "thinking".
            let last = self
                .chats
                .get(self.active_chat)
                .and_then(|chat| chat.messages.last());
            let calling = last.is_some_and(|message| matches!(message.role, Role::Tool));
            let waiting_approval = last.is_some_and(|message| {
                matches!(message.role, Role::Approval)
                    && message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.approval.as_deref() == Some(message.call.as_str())
                    })
            });
            let waiting_question = last.is_some_and(|message| {
                matches!(message.role, Role::Question)
                    && message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.question.as_deref() == Some(message.call.as_str())
                    })
            });
            let waiting_plan = last.is_some_and(|message| {
                matches!(message.role, Role::Plan)
                    && message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.plan.as_deref() == Some(message.call.as_str())
                    })
            });
            if !pending.draft.trim().is_empty() {
                let on_drag: Rc<dyn Fn(usize, DragEvent, &mut Window, &mut App)> = {
                    let view = cx.entity();
                    Rc::new(move |block, event, _window, cx| {
                        view.update(cx, |app, cx| {
                            app.apply_text_drag(DRAFT_MESSAGE, block, event, cx)
                        });
                    })
                };
                let copy_handler: Rc<dyn Fn(usize, SharedString, &mut Window, &mut App)> = {
                    let view = cx.entity();
                    Rc::new(move |block, body, _window, cx| {
                        view.update(cx, |app, cx| {
                            app.copy_code((DRAFT_MESSAGE, block), body.to_string(), cx)
                        });
                    })
                };
                let toggle_handler: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
                    let view = cx.entity();
                    Rc::new(move |block, _window, cx| {
                        view.update(cx, |app, cx| app.toggle_details(DRAFT_MESSAGE, block, cx));
                    })
                };
                let check_handler: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
                    let view = cx.entity();
                    Rc::new(move |block, _window, cx| {
                        view.update(cx, |app, cx| app.toggle_check(DRAFT_MESSAGE, block, cx));
                    })
                };
                rows.push(
                    div()
                        .flex()
                        .flex_col()
                        // Centred like a finished answer, so the text does not
                        // jump sideways when the answer lands.
                        .items_start()
                        .gap_1()
                        .w_full()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                // The same width as a finished answer, so its text
                                // starts and wraps exactly where the answer will.
                                .w(px(self.answer_width + 2. * BUBBLE_PAD))
                                .px_3()
                                .py_2()
                                .text_sm()
                                .text_color(rgb(palette.text))
                                .whitespace_normal()
                                .children(markdown_blocks(
                                    &self.parsed_blocks(DRAFT_MESSAGE, &pending.draft),
                                    &pending.draft,
                                    palette,
                                    DRAFT_MESSAGE,
                                    self.answer_width,
                                    self.selected_text.as_ref(),
                                    &on_drag,
                                    self.copied_code
                                        .filter(|(message, _)| *message == DRAFT_MESSAGE)
                                        .map(|(_, block)| block),
                                    &copy_handler,
                                    self.open_details(),
                                    &toggle_handler,
                                    self.open_checks(),
                                    &check_handler,
                                    &self.chat_dir(self.active_chat),
                                    &|key| self.block_scroll_handle(key),
                                    None,
                                ))
                                // The same closing line a finished answer gets; the
                                // token count only arrives with the last event, and
                                // the row keeps the height the copy button will take
                                // so the text does not jump when the answer lands.
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .min_h(px(24.))
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(answer_label(
                                            &pending.model,
                                            pending.request.elapsed_ms(),
                                            0,
                                        )),
                                ),
                        )
                        .into_any_element(),
                );
            }
            let label = if waiting_approval {
                "等待确认".to_string()
            } else if waiting_question {
                "等待回答".to_string()
            } else if waiting_plan {
                "等待批准计划".to_string()
            } else if calling {
                "正在调用工具".to_string()
            } else if pending.draft.trim().is_empty() {
                "正在思考".to_string()
            } else {
                "正在输出".to_string()
            };
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(status_dot(palette.running))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{label} · {}",
                                format_duration(pending.request.elapsed_ms())
                            )),
                    )
                    .child(
                        div()
                            .id("stop-answer")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .cursor_pointer()
                            .hover(move |style| {
                                style
                                    .bg(rgb(palette.panel_soft))
                                    .text_color(rgb(palette.text))
                            })
                            .on_click(cx.listener(|view, _, _, cx| view.cancel_answer(cx)))
                            .child("停止"),
                    )
                    .into_any_element(),
            );
        }
        // Conversations answered in the background keep a status row of their
        // own, so the stop control always names the chat it stops.
        for pending in self
            .pending
            .iter()
            .filter(|pending| pending.dir != active_dir)
        {
            let Some(index) = self.chats.iter().position(|chat| chat.dir == pending.dir) else {
                continue;
            };
            let title = header_title(&self.chats[index].title);
            let target = pending.dir.clone();
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .child(status_dot(palette.running))
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "「{title}」{} · {}",
                                if pending.approval.is_some() {
                                    "等待确认"
                                } else if pending.question.is_some() {
                                    "等待回答"
                                } else if pending.plan.is_some() {
                                    "等待批准计划"
                                } else {
                                    "正在回答"
                                },
                                format_duration(pending.request.elapsed_ms())
                            )),
                    )
                    .child(
                        div()
                            .id(("stop-other", index))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .cursor_pointer()
                            .hover(move |style| {
                                style
                                    .bg(rgb(palette.panel_soft))
                                    .text_color(rgb(palette.text))
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if let Some(index) =
                                    view.chats.iter().position(|chat| chat.dir == target)
                                {
                                    view.cancel_chat(index, cx);
                                }
                            }))
                            .child("停止"),
                    )
                    .into_any_element(),
            );
        }

        // Follow the answer while it streams in, but only while the reader is
        // at the end: scrolling up must win over the next delta, and the ↓
        // control is the way back. Nothing to follow in an empty conversation.
        if !chat.messages.is_empty() && !self.scrolled_away_from_bottom() {
            self.chat_scroll.scroll_to_bottom();
        }

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .w_full()
            .h_full()
            .min_w(px(0.))
            // Files dropped anywhere over the conversation become attachments;
            // the composer carries its own copy of this for drops on the card.
            .on_drop(cx.listener(|view, paths: &ExternalPaths, _window, cx| {
                view.attach_files(paths.paths(), cx);
            }))
            .children(self.render_banner(palette, cx))
            .when(!chat.messages.is_empty(), |this| {
                this.child(self.render_chat_header(palette, cx))
            })
            .when(self.chat_find_open, |this| {
                this.child(self.render_chat_find(palette, cx))
            })
            // The transcript and its scrollbar share one box, so the bar is
            // measured against the same area the thumb is drawn in: below the
            // header, not from the top of the pane. The 上下文 and 后台任务 tabs
            // take the same box, so switching never moves the header.
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .when(self.shown_chat_tab() == ChatTab::Chat, |this| {
                        this.child(
                            div()
                                .id("transcript")
                                .flex()
                                .flex_col()
                                .flex_1()
                                .w_full()
                                .min_h(px(0.))
                                .min_w(px(0.))
                                .gap_3()
                                .px(px(self.chat_gutter))
                                .pt_3()
                                .pb(self.composer_clearance())
                                .overflow_y_scroll()
                                .track_scroll(&self.chat_scroll)
                                .children(rows),
                        )
                        // Anything below the fold gets a way back to the newest
                        // message; it appears whenever the transcript overflows, and
                        // jumping is harmless when it is already at the end.
                        .when(self.chat_scroll.max_offset().y > px(8.), |this| {
                            this.child(self.render_scrollbar(palette, cx))
                        })
                    })
                    .when(self.shown_chat_tab() == ChatTab::Context, |this| {
                        this.child(appear(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .w_full()
                                .min_h(px(0.))
                                .min_w(px(0.))
                                .child(self.render_context_view(palette, cx)),
                            "chat-tab-context",
                            0.,
                        ))
                    })
                    .when(self.shown_chat_tab() == ChatTab::Tasks, |this| {
                        this.child(appear(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .w_full()
                                .min_h(px(0.))
                                .min_w(px(0.))
                                .child(self.render_tasks_view(palette, cx)),
                            "chat-tab-tasks",
                            0.,
                        ))
                    }),
            )
            .when(self.shown_chat_tab() == ChatTab::Chat, |this| {
                this
                    // The composer floats over the transcript, so the last message
                    // can scroll clear of it instead of being pushed around by its
                    // height.
                    .child(self.render_composer(palette, cx))
                    // A hint fades out on its own, just above the composer.
                    .children(self.render_toast(palette))
                    // At the bottom the button would do nothing but cover the last line.
                    .when(self.scrolled_away_from_bottom(), |this| {
                        this.child(self.render_to_bottom(palette, cx))
                    })
            })
    }

    /// First message drawn in the active conversation.
    ///
    /// The window is the newest [`CHAT_WINDOW`] entries, cut further while those
    /// hold more than [`CHAT_BLOCK_BUDGET`] markdown blocks: rebuilding every one
    /// of them per frame is what makes typing in a content-heavy chat feel slow.
    /// An explicit start (the reader asked for more) is honoured as it is.
    pub(crate) fn chat_window_start(&self) -> usize {
        let chat = self.chat();
        let total = chat.messages.len();
        let explicit = self.chat_from.get(&chat.dir).copied();
        if explicit.is_some() {
            return window_start(explicit, total);
        }
        let first = total.saturating_sub(CHAT_WINDOW);
        let counts: Vec<usize> = (first..total)
            .map(|index| {
                let message = &chat.messages[index];
                match message.role {
                    Role::Assistant => self.parsed_blocks(index, &message.text).len().max(1),
                    _ => 1,
                }
            })
            .collect();
        first + window_start_with_budget(&counts, None, CHAT_BLOCK_BUDGET, CHAT_WINDOW_MIN)
    }

    /// The chip that brings back older messages, above the visible window.
    fn render_earlier_chip(
        &self,
        from: usize,
        total: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .w_full()
            .child(
                div()
                    .id("show-earlier")
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(palette.panel))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .hover(move |style| {
                        style
                            .bg(rgb(palette.panel_soft))
                            .text_color(rgb(palette.text))
                    })
                    .on_click(cx.listener(|view, _, _, cx| view.show_earlier(cx)))
                    .child(format!(
                        "显示更早的 {} 条（共 {} 条）",
                        CHAT_EARLIER_STEP.min(from),
                        total
                    )),
            )
            .into_any_element()
    }

    /// Reveal one more step of older messages.
    pub(crate) fn show_earlier(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.dir.clone())
        else {
            return;
        };
        let from = self.chat_window_start();
        let start = from.saturating_sub(CHAT_EARLIER_STEP);
        self.chat_from.insert(dir, start);
        // Land on the oldest message that just appeared: row 0 is the chip
        // while anything is still hidden, so the first message is row 1.
        self.chat_scroll
            .scroll_to_item(if start > 0 { 1 } else { 0 });
        cx.notify();
    }

    /// The line above a conversation: its name, and the buttons that act on the
    /// whole transcript. Only drawn once the conversation has messages, so the
    /// empty state keeps its centered introduction. The figures that used to sit
    /// here (the billed total and the context share) moved under the composer,
    /// where they read as one status line with the rest of the request.
    pub(crate) fn render_chat_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let chat = self.chat();
        let row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_5()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .px_5()
            .pt_3()
            .pb_1()
            .child(
                // Clicking the name is the shortest way to rename: same dialog as
                // the card's "…" menu, without hunting for the card.
                div()
                    .id("chat-title")
                    .flex_shrink_0()
                    .px_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.commit_rename();
                        view.start_rename(view.active_chat, window, cx);
                    }))
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(header_title(&chat.title)),
                    ),
            )
            .child(div().flex_1().min_w(px(0.)))
            // The header's right end: find, copy, save and open the folder. All
            // are icons only and sit close together, away from the title.
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        div()
                            .id("find-chat")
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(24.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.start_chat_find(window, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/search.svg")
                                    .w(px(16.))
                                    .h(px(16.))
                                    .text_color(rgb(palette.faint)),
                            ),
                    )
                    .child(
                        div()
                            .id("copy-chat")
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(24.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| view.copy_transcript(cx)))
                            .child(
                                svg()
                                    .path("icons/copy.svg")
                                    .w(px(16.))
                                    .h(px(16.))
                                    .text_color(rgb(if self.copied_chat {
                                        palette.ok
                                    } else {
                                        palette.faint
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("save-chat")
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(24.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| view.save_transcript(cx)))
                            .child(
                                svg()
                                    .path("icons/save.svg")
                                    .w(px(16.))
                                    .h(px(16.))
                                    .text_color(rgb(palette.faint)),
                            ),
                    )
                    .child(
                        div()
                            .id("open-chat-dir")
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(24.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                let dir = view.chat_dir(view.active_chat);
                                let _ = std::fs::create_dir_all(&dir);
                                let _ = open_path(&dir);
                                cx.notify();
                            }))
                            .child(
                                svg()
                                    .path("icons/folder-open.svg")
                                    .w(px(16.))
                                    .h(px(16.))
                                    .text_color(rgb(palette.faint)),
                            ),
                    ),
            );

        // A hairline under the title row, so the header reads as its own band
        // above the transcript. It takes the sidebar's colour: the header sits
        // on the chrome surface, so a border-coloured line would read as a
        // separate box rather than a seam.
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .child(row)
            // The two views of the conversation, directly under its name.
            .child(self.render_chat_tabs(palette, cx))
            .child(self.render_todos(palette, cx))
            // One device pixel: `1. / scale` logical px, the thinnest line the
            // display can draw.
            .child(
                div()
                    .w_full()
                    .h(px(1. / self.scale_factor.max(1.)))
                    .bg(rgb(palette.chrome)),
            )
    }

    /// The conversation's todo list, under the tab strip.
    ///
    /// It stays out of the transcript so it does not scroll away, and folds to
    /// one line when nobody is reading it. The model writes the whole list on
    /// every `todo_write`, so the last write always replaces the card.
    pub(crate) fn render_todos(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let chat = self.chat();
        if chat.todos.is_empty() {
            return div();
        }
        let total = chat.todos.len();
        let done = chat
            .todos
            .iter()
            .filter(|todo| todo.status == TodoStatus::Done)
            .count();
        let mut card = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .min_w(px(0.))
            .px_5()
            .pb_1();
        card =
            card.child(
                div()
                    .id("todos-head")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.todos_open = !view.todos_open;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.muted))
                            .child(format!("待办 {done}/{total}")),
                    )
                    .child(div().text_xs().text_color(rgb(palette.faint)).child(
                        if self.todos_open {
                            "▾ 收起"
                        } else {
                            "▸ 展开"
                        },
                    )),
            );
        if self.todos_open {
            let mut list = div()
                .flex()
                .flex_col()
                .gap_0p5()
                .w_full()
                .min_w(px(0.))
                .pt_1();
            for (index, todo) in chat.todos.iter().enumerate().take(8) {
                let color = match todo.status {
                    TodoStatus::Done => palette.ok,
                    TodoStatus::Active => palette.accent,
                    TodoStatus::Pending => palette.faint,
                };
                list = list.child(
                    div()
                        .id(("todo", index))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex_shrink_0()
                                .size(px(6.))
                                .rounded_full()
                                .bg(rgb(color)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(px(52.))
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(todo.status.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .text_color(rgb(if todo.status == TodoStatus::Done {
                                    palette.faint
                                } else {
                                    palette.text
                                }))
                                .child(todo.text.clone()),
                        ),
                );
            }
            if total > 8 {
                list = list.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(format!("…另有 {} 项", total - 8)),
                );
            }
            card = card.child(list);
        }
        card
    }

    /// The model a conversation is using: the last answer's, else the active
    /// connection's, else empty.
    pub(crate) fn chat_model(&self) -> String {
        self.chat()
            .messages
            .iter()
            .rev()
            .find(|message| !message.model.is_empty())
            .map_or_else(
                || {
                    self.ai
                        .active()
                        .map_or_else(String::new, |connection| connection.model.clone())
                },
                |message| message.model.clone(),
            )
    }

    /// Scroll the transcript by a fraction of a screen; `1.0` is one screen further down.
    /// The offset is negative while scrolled down, so scrolling down subtracts.
    pub(crate) fn scroll_transcript(&mut self, screens: f32, cx: &mut Context<Self>) {
        let viewport = self.chat_scroll.bounds().size.height;
        let max = self.chat_scroll.max_offset().y;
        if max <= px(0.) {
            return;
        }
        let current = self.chat_scroll.offset();
        let offset = (current.y - viewport * screens).clamp(-max, px(0.));
        self.chat_scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// Jump the transcript to its start or end.
    pub(crate) fn scroll_transcript_to(&mut self, end: bool, cx: &mut Context<Self>) {
        let max = self.chat_scroll.max_offset().y;
        let current = self.chat_scroll.offset();
        let offset = if end { -max } else { px(0.) };
        self.chat_scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// Open the in-conversation find bar and give it the keyboard.
    pub(crate) fn start_chat_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_find_open = true;
        self.chat_find_at = 0;
        self.chat_find_query.clear();
        self.focused = Some(Field::ChatFind);
        if let Some(field) = self.field_mut(Field::ChatFind) {
            field.select_all();
        }
        if let Some(field) = self.field(Field::ChatFind) {
            window.focus(&field.focus_handle().clone(), cx);
        }
        self.start_caret_blink(cx);
        cx.notify();
    }

    /// Close the find bar and give the keyboard back to the content.
    pub(crate) fn close_chat_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat_find_open = false;
        self.chat_find.clear();
        self.chat_find_at = 0;
        self.chat_find_query.clear();
        if self.focused == Some(Field::ChatFind) {
            self.focused = None;
            window.focus(&self.content_focus, cx);
        }
        cx.notify();
    }

    /// Messages whose text contains the current query, oldest first.
    pub(crate) fn chat_find_matches(&self) -> Vec<usize> {
        find_messages(&self.chat().messages, &self.chat_find.text)
    }

    /// The message the find bar is pointing at, if a query is active.
    ///
    /// A fresh query points at its first match; later jumps follow
    /// `chat_find_at`, exactly like the counter in the bar.
    pub(crate) fn current_find_message(&self) -> Option<usize> {
        let needle = self.chat_find.text.trim().to_lowercase();
        if needle.is_empty() {
            return None;
        }
        let matches = self.chat_find_matches();
        if matches.is_empty() {
            return None;
        }
        let at = if needle != self.chat_find_query {
            0
        } else {
            self.chat_find_at.min(matches.len() - 1)
        };
        matches.get(at).copied()
    }

    /// Jump to the next (or previous) message that matches the find query.
    ///
    /// The first Enter after typing starts at the first match instead of
    /// skipping it; later presses cycle through the list.
    pub(crate) fn search_chat(&mut self, step: isize, cx: &mut Context<Self>) {
        let needle = self.chat_find.text.trim().to_lowercase();
        if needle.is_empty() {
            self.chat_find_at = 0;
            cx.notify();
            return;
        }
        let matches = self.chat_find_matches();
        let fresh = needle != self.chat_find_query;
        if matches.is_empty() {
            self.chat_find_at = 0;
            self.chat_find_query = needle;
            cx.notify();
            return;
        }
        let next = if fresh {
            if step < 0 { matches.len() - 1 } else { 0 }
        } else {
            let base = self.chat_find_at.min(matches.len() - 1);
            if step < 0 {
                (base + matches.len() - 1) % matches.len()
            } else {
                (base + 1) % matches.len()
            }
        };
        self.chat_find_at = next;
        self.chat_find_query = needle;
        let target = matches[next];
        let mut from = self.chat_window_start();
        if target < from {
            // The hit is older than what is drawn: open the window there.
            if let Some(dir) = self
                .chats
                .get(self.active_chat)
                .map(|chat| chat.dir.clone())
            {
                self.chat_from.insert(dir, target);
            }
            from = target;
        }
        // One row per message, shifted by the chip while anything is hidden.
        self.chat_scroll
            .scroll_to_item(target - from + usize::from(from > 0));
        cx.notify();
    }

    /// The find bar under the conversation header.
    pub(crate) fn render_chat_find(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let matches = self.chat_find_matches();
        let needle = self.chat_find.text.trim().to_lowercase();
        let fresh = needle != self.chat_find_query;
        let current = if matches.is_empty() || fresh {
            0
        } else {
            self.chat_find_at.min(matches.len() - 1)
        };
        let count = if matches.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", current + 1, matches.len())
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded_md()
                .text_xs()
                .text_color(rgb(palette.muted))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                .child(label)
        };
        div()
            .id("chat-find")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .px_5()
            .py_1()
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                svg()
                    .path("icons/search.svg")
                    .w(px(14.))
                    .h(px(14.))
                    .text_color(rgb(palette.faint)),
            )
            .child(div().flex_1().min_w(px(0.)).child(self.render_field(
                Field::ChatFind,
                "在对话里查找…",
                palette,
                cx,
            )))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(count),
            )
            .child(
                button("find-prev", "上一个")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.search_chat(-1, cx))),
            )
            .child(
                button("find-next", "下一个")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.search_chat(1, cx))),
            )
            .child(
                div()
                    .id("find-close")
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(|view, _, window, cx| view.close_chat_find(window, cx)))
                    .child("×"),
            )
    }

    /// Track height, thumb height and maximum offset of the transcript scrollbar.
    ///
    /// The bar is drawn, dragged and clicked from these numbers only, so the three
    /// can never disagree about where the thumb is.
    pub(crate) fn scrollbar_metrics(&self) -> Option<(Pixels, Pixels, Pixels)> {
        let viewport = self.chat_scroll.bounds().size.height;
        if viewport <= px(0.) {
            return None;
        }
        let max = self.chat_scroll.max_offset().y;
        let content = viewport + max;
        // The track stops above the floating composer.
        let track = (viewport - px(8.) - self.composer_clearance() - px(6.)).max(px(24.));
        let thumb = (track * (viewport / content).max(0.08))
            .max(px(24.))
            .min(track);
        Some((track, thumb, max))
    }

    /// Follow the pointer while the scrollbar is held.
    pub(crate) fn drag_scrollbar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((start_y, start_offset)) = self.scroll_drag else {
            return;
        };
        let Some((track, thumb, max)) = self.scrollbar_metrics() else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        // `offset.y` is negative while scrolled down, so dragging the thumb up
        // (a smaller pointer y) has to bring the offset back towards zero.
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = self.chat_scroll.offset();
        self.chat_scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// A thin scrollbar down the right edge of the transcript.
    ///
    /// gpui has no scrollbar element, so the thumb is worked out from the scroll
    /// handle's own numbers. Clicking the track jumps to that part of the
    /// conversation; the wheel, the keyboard and the "↓" button still work as before.
    pub(crate) fn render_scrollbar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some((track, thumb, max)) = self.scrollbar_metrics() else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-self.chat_scroll.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = self.scroll_drag.is_some();

        div()
            .absolute()
            .right(px(4.))
            .top(px(8.))
            .bottom(self.composer_clearance() + px(6.))
            .w(px(8.))
            .id("chat-scrollbar")
            // The transcript underneath would otherwise take the press for a
            // text-selection drag and the bar would never react. Scrolling still
            // goes through, so the wheel works over the bar too.
            .block_mouse_except_scroll()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, cx| {
                    let Some((track, thumb, max)) = view.scrollbar_metrics() else {
                        return;
                    };
                    let travel = (track - thumb).max(px(1.));
                    // The bar is drawn from the top of the transcript, which is
                    // the same box the thumb is measured in.
                    let top = view.chat_scroll.bounds().origin.y + px(8.);
                    let progress = if max > px(0.) {
                        (-view.chat_scroll.offset().y / max).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let thumb_top = top + travel * progress;
                    let on_thumb =
                        event.position.y >= thumb_top && event.position.y <= thumb_top + thumb;
                    if !on_thumb {
                        // A click on the track jumps there; the drag then carries on
                        // from the new place, so click-and-pull works in one gesture.
                        let ratio = ((event.position.y - top) / travel).clamp(0., 1.);
                        let current = view.chat_scroll.offset();
                        view.chat_scroll.set_offset(point(current.x, -max * ratio));
                    }
                    view.scroll_drag = Some((event.position.y, view.chat_scroll.offset().y));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .top(travel * progress)
                    .w(px(6.))
                    .h(thumb)
                    .rounded_full()
                    .bg(rgb(if dragging {
                        palette.muted
                    } else {
                        palette.border
                    })),
            )
            .into_any_element()
    }

    /// Land on the newest message, or on the top when there is none.
    ///
    /// A short window cannot fit the introduction, and scrolling it to the
    /// bottom would hide the line that says what the page is for.
    pub(crate) fn scroll_chat_to_newest(&self) {
        if self.chat().messages.is_empty() {
            self.chat_scroll.set_offset(point(px(0.), px(0.)));
        } else {
            // `scroll_to_bottom` only moves the offset at paint time, so a click
            // would rebuild the tree from the old offset and leave the jump
            // button on screen. Jump now that the layout is known, and keep the
            // deferred request for the case where it is not (first render).
            let max = self.chat_scroll.max_offset().y;
            if max > px(0.) {
                self.chat_scroll.set_offset(point(px(0.), -max));
            }
            self.chat_scroll.scroll_to_bottom();
        }
    }

    /// Whether the transcript is scrolled away from the newest message.
    ///
    /// The "jump to newest" control is meaningless at the bottom, and it would
    /// sit on top of the last line there, so it only shows once the reader has
    /// moved up by more than a line or two.
    pub(crate) fn scrolled_away_from_bottom(&self) -> bool {
        // The empty state has no newest message to go back to; it can overflow a
        // short window, but the button would only jump to the end of the hints.
        if self.chat().messages.is_empty() {
            return false;
        }
        let max = self.chat_scroll.max_offset().y;
        max > px(8.) && self.chat_scroll.offset().y > -max + px(24.)
    }

    /// A small floating "jump to the newest message" control.
    pub(crate) fn render_to_bottom(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .right(px(30.))
            .bottom(self.composer_clearance() + px(6.))
            .id("to-bottom")
            .flex()
            .items_center()
            .justify_center()
            .size(px(30.))
            .rounded_full()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel_soft)))
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .text_size(px(16.))
                    .child("↓"),
            )
            .on_click(cx.listener(|view, _, _, cx| {
                view.scroll_chat_to_newest();
                cx.notify();
            }))
    }

    /// How much room the transcript must leave for the composer.
    ///
    /// The card grows with the draft — one row per manual line, and one more for
    /// every line too long to fit — so a fixed clearance would let the last
    /// message hide behind it. An attachment row adds its own height the same
    /// way.
    pub(crate) fn composer_clearance(&self) -> Pixels {
        let lines = self.composer_rows().clamp(1, MAX_COMPOSER_LINES);
        let extra = (lines - 1) as f32 * COMPOSER_LINE_HEIGHT;
        let attachments = if self.attachments.is_empty() {
            0.
        } else {
            ATTACHMENT_ROW_HEIGHT
        };
        px(COMPOSER_CLEARANCE + extra + attachments)
    }

    /// How many rows the draft occupies once folded into the card's width.
    ///
    /// Falls back to the manual line count until the first frame has been laid
    /// out and a text system is at hand.
    pub(crate) fn composer_rows(&self) -> usize {
        let Some(text_system) = self.window_text.as_deref() else {
            return self.composer.text.split('\n').count().max(1);
        };
        crate::app::fields::field_rows(
            text_system,
            &self.composer.text,
            self.field_text_width(Field::Composer),
            true,
        )
        .len()
        .max(1)
    }

    /// Copy one message to the clipboard.
    ///
    /// gpui-pre 0.3.14 has no selectable text, so the only way to take a message away
    /// is to copy the whole of it. The button is an icon beside the bubble: left
    /// of a question, and in the closing row of an answer, next to what it cost.
    /// It turns green while it confirms the copy.
    pub(crate) fn copy_button(
        &self,
        index: usize,
        message: &ChatMessage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copied = self.copied == Some(index);
        let text = message.text.clone();
        div()
            .id(("copy", index))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(px(24.))
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel_soft)))
            .on_click(cx.listener(move |view, _, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                view.copied = Some(index);
                cx.notify();
                // The icon returns to its colour after a moment; a later copy only
                // wins if it is still the one being shown.
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(1200))
                        .await;
                    let _ = this.update(cx, |view, cx| {
                        if view.copied == Some(index) {
                            view.copied = None;
                            cx.notify();
                        }
                    });
                })
                .detach();
            }))
            .child(
                svg()
                    .path("icons/copy.svg")
                    .w(px(16.))
                    .h(px(16.))
                    .text_color(rgb(if copied { palette.ok } else { palette.faint })),
            )
            .into_any_element()
    }

    /// The `<details>` sections the open conversation has expanded.
    ///
    /// Read through [`Self::chat`], which clamps a stale index, so a card that
    /// was just deleted cannot turn a repaint into a panic.
    pub(crate) fn open_details(&self) -> &std::collections::HashSet<(usize, usize)> {
        &self.chat().details
    }

    /// The task boxes the reader has clicked in the open conversation.
    pub(crate) fn open_checks(&self) -> &std::collections::HashSet<(usize, usize)> {
        &self.chat().checks
    }

    /// The scroll handle of one code block or table, created the first time it is
    /// drawn.
    pub(crate) fn block_scroll_handle(&self, key: (usize, usize)) -> ScrollHandle {
        self.block_scroll
            .borrow_mut()
            .entry(key)
            .or_default()
            .clone()
    }

    /// The parsed blocks of one message, re-parsed only when its text changed.
    pub(crate) fn parsed_blocks(
        &self,
        message: usize,
        text: &str,
    ) -> Rc<Vec<crate::markdown::Rich>> {
        cached_parse(&mut self.parsed.borrow_mut(), message, text)
    }

    /// The `<details>` sections the run transcript has opened.
    pub(crate) fn transcript_details(&self) -> &std::collections::HashSet<(usize, usize)> {
        &self.transcript_details
    }

    /// Open or close one section of the run transcript.
    pub(crate) fn toggle_transcript_details(
        &mut self,
        message: usize,
        block: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.transcript_details.remove(&(message, block)) {
            self.transcript_details.insert((message, block));
        }
        cx.notify();
    }

    /// Open or close a `<details>` section of an answer.
    pub(crate) fn toggle_details(&mut self, message: usize, block: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat)
            && !chat.details.remove(&(message, block))
        {
            chat.details.insert((message, block));
        }
        // The open sections belong to the conversation; a toggle on the answer
        // still streaming is transient and is not written.
        if message != DRAFT_MESSAGE {
            self.save_chats();
        }
        cx.notify();
    }

    /// Open or close the thinking block above one answer.
    ///
    /// Saved with the conversation, like a `<details>` section, so the reader's
    /// choice survives a restart.
    pub(crate) fn toggle_reasoning(&mut self, message: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .chats
            .get_mut(self.active_chat)
            .and_then(|chat| chat.messages.get_mut(message))
        {
            entry.open = !entry.open;
        }
        if message != DRAFT_MESSAGE {
            self.save_chats();
        }
        cx.notify();
    }

    /// Expand or fold one tool row; the choice is saved with the conversation.
    pub(crate) fn toggle_tool_row(
        &mut self,
        message: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self
            .chats
            .get_mut(self.active_chat)
            .and_then(|chat| chat.messages.get_mut(message))
        {
            entry.open = !entry.open;
        }
        if message != DRAFT_MESSAGE {
            self.save_chats();
        }
        cx.notify();
        // The detail only learns its scroll range while it is painted, so its
        // scrollbar would miss the frame that opened it; ask for one more. Not
        // `window.request_animation_frame()`: that reads `Window::current_view`,
        // which panics outside a paint phase (this runs from a click listener).
        cx.on_next_frame(window, |_, _, cx| cx.notify());
    }

    /// Follow the pointer while a tool detail's scrollbar is held.
    pub(crate) fn drag_tool_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((row, start_y, start_offset)) = self.tool_drag else {
            return;
        };
        let handle = self.block_scroll_handle((row, usize::MAX));
        let Some((track, thumb, max, _)) = tool_bar_metrics(&handle) else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = handle.offset();
        handle.set_offset(point(current.x, offset));
        cx.notify();
    }

    pub(crate) fn toggle_check(&mut self, message: usize, block: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat)
            && !chat.checks.remove(&(message, block))
        {
            chat.checks.insert((message, block));
        }
        // A click on the answer still streaming is transient and is not written.
        if message != DRAFT_MESSAGE {
            self.save_chats();
        }
        cx.notify();
    }

    pub(crate) fn copy_code(&mut self, key: (usize, usize), body: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(body));
        self.copied_code = Some(key);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.copied_code == Some(key) {
                    view.copied_code = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Copy the whole conversation as Markdown, confirming on the icon for a moment.
    pub(crate) fn copy_transcript(&mut self, cx: &mut Context<Self>) {
        let text = {
            let chat = self.chat();
            transcript_text(&chat.title, &chat.messages)
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied_chat = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1200))
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.copied_chat {
                    view.copied_chat = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Write the conversation as Markdown into its own folder.
    ///
    /// There is no native save dialog, so the file lands where the conversation
    /// already keeps its files: `<工作目录>\workspace\chat-N\<标题>.md`. The name
    /// comes from the title, so exporting again replaces the previous file; the
    /// floating hint says which file it was.
    pub(crate) fn save_transcript(&mut self, cx: &mut Context<Self>) {
        let index = self.active_chat;
        let (title, body) = {
            let chat = self.chat();
            (
                chat.title.clone(),
                transcript_text(&chat.title, &chat.messages),
            )
        };
        let dir = self.chat_dir(index);
        let name = format!("{}.md", export_name(&title));
        let path = dir.join(&name);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            self.show_toast(format!("无法创建 {}：{error}", dir.display()), true, cx);
            return;
        }
        match std::fs::write(&path, body) {
            Ok(()) => self.show_toast(format!("已保存 {name}"), false, cx),
            Err(error) => self.show_toast(format!("保存失败：{error}"), true, cx),
        }
    }

    /// The regenerate icon, shown beside the answer it would ask again.
    pub(crate) fn regenerate_button(
        &self,
        index: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(("regenerate", index))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(px(24.))
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel_soft)))
            .on_click(cx.listener(|view, _, _, cx| view.regenerate(cx)))
            .child(
                svg()
                    .path("icons/regenerate2.svg")
                    .w(px(16.))
                    .h(px(16.))
                    .text_color(rgb(palette.faint)),
            )
            .into_any_element()
    }

    /// Ask the last question again, dropping the answer it got.
    ///
    /// The question goes back through the normal send path, so the transcript, the
    /// conversation folder and the saved file stay consistent, and the model still
    /// sees whatever came before the question as history.
    pub(crate) fn regenerate(&mut self, cx: &mut Context<Self>) {
        if self.chat_busy(self.active_chat) {
            return;
        }
        let index = self.active_chat;
        let Some(question) = self
            .chats
            .get_mut(index)
            .and_then(|chat| take_last_question(&mut chat.messages))
        else {
            return;
        };
        self.selected_text = None;
        self.text_dragging = false;
        // The dropped answer's code blocks are gone; their scroll state with it.
        self.block_scroll.borrow_mut().clear();
        self.parsed.borrow_mut().clear();
        self.composer.set_text(question);
        self.save_chats();
        self.send_message(cx);
    }

    /// Take the newest question back into the composer instead of asking it again.
    ///
    /// The answer it got is dropped exactly as a retry would drop it, but nothing
    /// is sent: the question waits in the composer so it can be changed first.
    pub(crate) fn edit_last_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat_busy(self.active_chat) {
            return;
        }
        let index = self.active_chat;
        let Some(question) = self.chats.get_mut(index).and_then(|chat| {
            let question = take_last_question(&mut chat.messages);
            if question.is_some() {
                chat.touch();
            }
            question
        }) else {
            return;
        };
        self.selected_text = None;
        self.text_dragging = false;
        // The dropped answer's code blocks are gone; their scroll state with it.
        self.block_scroll.borrow_mut().clear();
        self.parsed.borrow_mut().clear();
        self.composer.set_text(question);
        self.composer_recall = None;
        self.focused = Some(Field::Composer);
        window.focus(&self.composer.focus_handle().clone(), cx);
        self.start_caret_blink(cx);
        self.save_chats();
        cx.notify();
    }

    /// Start the caret blinking, once, while a field has the focus.
    ///
    /// The task stops itself as soon as nothing is focused, so an idle window does
    /// not wake up twice a second for a caret nobody can see.
    pub(crate) fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        self.caret_on = true;
        if self.caret_blink {
            return;
        }
        self.caret_blink = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(530))
                    .await;
                let finished = this.update(cx, |view, cx| {
                    if view.focused.is_none() {
                        view.caret_blink = false;
                        view.caret_on = true;
                        cx.notify();
                        return true;
                    }
                    view.caret_on = !view.caret_on;
                    cx.notify();
                    false
                });
                if finished.unwrap_or(true) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Record a drag reported by one selectable block.
    pub(crate) fn apply_text_drag(
        &mut self,
        message: usize,
        block: usize,
        event: DragEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            DragEvent::Start { index } => {
                let point = TextPoint {
                    message,
                    block,
                    index,
                };
                self.selected_text = Some(TextSelection {
                    anchor: point,
                    focus: point,
                });
                self.text_dragging = true;
            }
            DragEvent::Extend { index } => {
                if !self.text_dragging {
                    return;
                }
                let Some(selection) = self.selected_text.as_mut() else {
                    return;
                };
                // A drag may run into another message: each block reports its
                // own place, and the selection keeps both ends in reading order.
                selection.focus = TextPoint {
                    message,
                    block,
                    index,
                };
            }
            DragEvent::Word { start, end } => {
                // A double click is a finished selection: the pointer is still
                // down, but moving it must not stretch the word.
                self.selected_text = Some(TextSelection {
                    anchor: TextPoint {
                        message,
                        block,
                        index: start,
                    },
                    focus: TextPoint {
                        message,
                        block,
                        index: end,
                    },
                });
                self.text_dragging = false;
            }
            DragEvent::End => {
                self.text_dragging = false;
                if self
                    .selected_text
                    .as_ref()
                    .is_some_and(|selection| selection.is_empty())
                {
                    self.selected_text = None;
                }
            }
        }
        cx.notify();
    }

    /// Select every question and answer in the transcript.
    ///
    /// `Ctrl+A` has nothing to select while no field has the keyboard, so it
    /// takes the conversation: the same bytes a drag from the first character to
    /// the last would cover, ready for `Ctrl+C`.
    pub(crate) fn select_whole_transcript(&mut self, cx: &mut Context<Self>) -> bool {
        let entries: Vec<(usize, Vec<usize>)> = self
            .selectable_messages()
            .into_iter()
            .map(|message| {
                let lengths = self
                    .block_texts(message)
                    .iter()
                    .map(|text| text.len())
                    .collect();
                (message, lengths)
            })
            .collect();
        let Some(selection) = TextSelection::whole_transcript(&entries) else {
            return false;
        };
        self.selected_text = Some(selection);
        self.text_dragging = false;
        cx.notify();
        true
    }

    /// The selected text, one line per block it crosses.
    ///
    /// A drag can take in several messages, so the range is walked entry by
    /// entry; questions and answers are the only entries drawn as text blocks,
    /// and the answer still being written is the last one.
    pub(crate) fn selected_slice(&self) -> Option<String> {
        let selection = self.selected_text.as_ref()?;
        let (start, end) = selection.ordered();
        let mut parts: Vec<String> = Vec::new();
        for message in self.selectable_messages() {
            if message < start.message || message > end.message {
                continue;
            }
            for (block, body) in self.block_texts(message).iter().enumerate() {
                let key = (message, block);
                if key < (start.message, start.block) || key > (end.message, end.block) {
                    continue;
                }
                let from = if key == (start.message, start.block) {
                    start.index
                } else {
                    0
                };
                let to = if key == (end.message, end.block) {
                    end.index
                } else {
                    body.len()
                };
                let slice = body.get(from..to).unwrap_or_default();
                if !slice.is_empty() {
                    parts.push(slice.to_string());
                }
            }
        }
        let text = parts.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    /// The transcript entries a drag can start or end in, in drawing order.
    ///
    /// Tool rows and notes are not drawn as selectable text, so a selection that
    /// runs past them picks up only the questions and answers around them.
    fn selectable_messages(&self) -> Vec<usize> {
        let mut entries: Vec<usize> = self
            .chat()
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| matches!(message.role, Role::User | Role::Assistant))
            .map(|(index, _)| index)
            .collect();
        // The answer being written is drawn after every saved entry.
        if self.active_pending().is_some() {
            entries.push(DRAFT_MESSAGE);
        }
        entries
    }

    /// The text of every selectable block of one message, in drawing order.
    pub(crate) fn block_texts(&self, message: usize) -> Vec<String> {
        if message == DRAFT_MESSAGE {
            let Some(pending) = self.active_pending() else {
                return Vec::new();
            };
            return answer_block_texts(&pending.draft);
        }
        let Some(entry) = self
            .chats
            .get(self.active_chat)
            .and_then(|chat| chat.messages.get(message))
        else {
            return Vec::new();
        };
        match entry.role {
            Role::Assistant => answer_block_texts(&entry.text),
            // A question (or a note) is drawn as one block of its own text.
            _ => vec![entry.text.clone()],
        }
    }

    /// Copy the selection; true when something went to the clipboard.
    pub(crate) fn copy_selected(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.selected_slice() else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        true
    }

    /// Drop the selection.
    pub(crate) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected_text = None;
        self.text_dragging = false;
        cx.notify();
    }
}

/// Parse one message's Markdown, reusing the previous parse while the text is
/// unchanged.
pub(crate) fn cached_parse(
    cache: &mut std::collections::HashMap<usize, (String, Rc<Vec<crate::markdown::Rich>>)>,
    message: usize,
    text: &str,
) -> Rc<Vec<crate::markdown::Rich>> {
    if let Some((cached, blocks)) = cache.get(&message)
        && cached == text
    {
        return blocks.clone();
    }
    let blocks = Rc::new(crate::markdown::parse_rich(text));
    cache.insert(message, (text.to_string(), blocks.clone()));
    blocks
}

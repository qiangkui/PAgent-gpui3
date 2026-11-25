//! One message bubble, the empty state, and the markdown the answers are drawn from.

use super::*;

impl DshFlowApp {
    /// What a blank conversation shows: hints and example questions.
    pub(crate) fn render_chat_empty(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let examples = [
            "引擎和示例目录里大概有多少文件？",
            "在这个对话目录里写一个 hello.txt，再读回来确认内容",
            "照 repo-health 的结构，在对话目录写一个统计行数的工作流草稿",
            "解释一下 repo-health 这个流程做了什么",
        ];
        let mut chips: Vec<AnyElement> = Vec::new();
        for example in examples {
            chips.push(
                div()
                    .id(example)
                    .px_3()
                    .py_2()
                    .rounded_md()
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
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.composer.set_text(example);
                        view.composer_recall = None;
                        view.focused = Some(Field::Composer);
                        window.focus(&view.composer.focus_handle().clone(), cx);
                        cx.notify();
                    }))
                    .child(example)
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .items_start()
            .gap_3()
            .child(div().text_xl().text_color(rgb(palette.text)).child("开始和助手对话"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child("助手会通过 dsh-flow 引擎回答问题；左侧的工作流卡片可以直接运行，结果回到这里。"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("助手可以直接读写工作区里的文件并保留撤销；「仅可查看」下每次写入先询问。「仅可查看」与「工作区内修改」下命令、移动 / 删除都先询问；「完全权限」可以读写整台电脑，也不再询问、不检查命令路径。前两档写入不会碰工作区以外的文件，命令引用的路径也必须在工作区内，否则直接拒绝。常驻进程（dev server 等）可以作为后台任务运行，用完用 task 工具结束。"),
            )
            // Without a tested connection nothing can answer, so the first thing
            // the empty state does is point at where to fix that.
            .when(!self.ai.usable(), |this| {
                this.child(
                    div()
                        .id("empty-open-settings")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(palette.panel))
                        .border_1()
                        .border_color(rgb(palette.failed))
                        .text_xs()
                        .text_color(rgb(palette.failed))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(palette.panel_soft)))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_settings(cx);
                        }))
                        .child("还没有可用的模型：到「设置 → AI」填好 Key 并点「测试」")
                        .child("→"),
                )
            })
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child(format!(
                        "这个对话的工作目录：{}",
                        self.chat_dir(self.active_chat).display()
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("Enter 发送 · Shift+Enter 换行 · 拖入文本文件作为附件 · 回答里的文字可以直接拖选复制"),
            )
            .child(div().flex().flex_col().gap_2().children(chips))
            .into_any_element()
    }

    /// The thinking block above an answer: a toggle line, and the reasoning itself
    /// once it is open.
    ///
    /// Thinking is output the endpoint billed for, but it is not what the model
    /// said; showing it in the answer's own text would make the transcript read
    /// as if the model replied with its inner monologue.
    fn render_reasoning(
        &self,
        index: usize,
        message: &ChatMessage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = message.open;
        let mut block = div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .min_w(px(0.))
            .child(
                div()
                    .id(("reasoning-toggle", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.toggle_reasoning(index, cx);
                    }))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(if open { "▾" } else { "▸" }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("思考过程"),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(format!("{} 字", message.reasoning.chars().count())),
                    )
                    .child(div().flex_1().min_w(px(0.))),
            );
        if open {
            block = block.child(
                div()
                    .id(("reasoning-text", index))
                    .w_full()
                    .min_w(px(0.))
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .contained_wheel()
                    .rounded_md()
                    .bg(rgb(palette.panel_soft))
                    .p_2()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(message.reasoning.clone()),
            );
        }
        block.into_any_element()
    }

    /// One transcript entry; `find` is the lower-cased query when this entry is
    /// the search hit being shown, so its matches can be painted.
    pub(crate) fn render_message(
        &self,
        index: usize,
        message: &ChatMessage,
        palette: Palette,
        find: Option<&str>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let messages = &self.chat().messages;
        // Retrying acts on the newest question, so the control belongs on the last
        // thing that is not a note: an answer, or the failure note itself.
        let idle = !self.chat_busy(self.active_chat);
        let retry_slot = idle.then(|| retry_message(messages)).flatten();
        let retryable = retry_slot == Some(index);
        // Old saves carry no model, time or cost: an answer would then get an
        // empty line saying only "助手", so it is dropped instead.
        let has_meta = !message.model.is_empty() || message.elapsed_ms > 0 || message.tokens > 0;
        // One muted line carries both the signature and what this answer
        // changed, so a wide diff never pushes the action icons out of the row.
        let change = if matches!(message.role, Role::Assistant) {
            turn_change(messages, index)
        } else {
            None
        };
        let change_text = |change: TurnChange| {
            if change.files > 0 {
                format!(
                    "本轮 {} 个文件 · +{} −{}",
                    change.files, change.added, change.removed
                )
            } else {
                format!("本轮 +{} −{}", change.added, change.removed)
            }
        };
        let signature = match (has_meta, change) {
            (true, Some(change)) => Some(format!(
                "{} · {}",
                answer_label(&message.model, message.elapsed_ms, message.tokens),
                change_text(change)
            )),
            (true, None) => Some(answer_label(
                &message.model,
                message.elapsed_ms,
                message.tokens,
            )),
            (false, Some(change)) => Some(change_text(change)),
            (false, None) => None,
        };
        // Only the newest question can be taken back for editing: everything after
        // it is the answer that editing would replace.
        let editable = idle
            && matches!(message.role, Role::User)
            && last_question(messages).is_some_and(|(position, _)| position == index);
        // Writes made for this answer can be rolled back together; the count is
        // what the button says, so it is only computed for an answer.
        let turn_undo = if idle && matches!(message.role, Role::Assistant) {
            let count = turn_writes(messages, index).len();
            (count > 0).then_some(count)
        } else {
            None
        };
        // The files this answer touched, in write order: the chip says how many
        // and the list opens one of them, while the button above rolls the whole
        // turn back.
        let turn_files: Vec<(String, bool)> = if idle && matches!(message.role, Role::Assistant) {
            turn_writes(messages, index)
                .into_iter()
                .filter_map(|at| messages.get(at))
                .filter(|row| !row.path.trim().is_empty())
                .map(|row| (row.path.clone(), row.existed))
                .collect()
        } else {
            Vec::new()
        };
        let changes_open = self.changes_open == Some(index);

        let selection = self.selected_text.as_ref();
        let highlight: Hsla = rgb(palette.selection).into();
        let on_drag: Rc<dyn Fn(usize, DragEvent, &mut Window, &mut App)> = {
            let view = cx.entity();
            Rc::new(move |block, event, _window, cx| {
                view.update(cx, |app, cx| app.apply_text_drag(index, block, event, cx));
            })
        };
        let copy_handler: Rc<dyn Fn(usize, SharedString, &mut Window, &mut App)> = {
            let view = cx.entity();
            Rc::new(move |block, body, _window, cx| {
                view.update(cx, |app, cx| {
                    app.copy_code((index, block), body.to_string(), cx)
                });
            })
        };
        let toggle_handler: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let view = cx.entity();
            Rc::new(move |block, _window, cx| {
                view.update(cx, |app, cx| app.toggle_details(index, block, cx));
            })
        };
        // A task box is a control of its own: clicking it ticks or unticks the
        // item, which is remembered with the conversation.
        let check_handler: Rc<dyn Fn(usize, &mut Window, &mut App)> = {
            let view = cx.entity();
            Rc::new(move |block, _window, cx| {
                view.update(cx, |app, cx| app.toggle_check(index, block, cx));
            })
        };
        // One selectable block of this message's text, block 0 for a question.
        // Questions are drawn as plain text: no runs, so no links either.
        let find_color: Hsla = rgb(palette.find).into();
        let selectable = |block: usize,
                          body: SharedString,
                          wrap: f32,
                          spans: &[markdown::Span]|
         -> SelectableText {
            let range = selection.and_then(|chosen| chosen.part_of(index, block, body.len()));
            let marks = find.map_or_else(Vec::new, |needle| {
                find_ranges(&body, needle, MAX_FIND_MARKS)
            });
            let on_drag = on_drag.clone();
            let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> =
                Rc::new(move |event, window, cx| on_drag(block, event, window, cx));
            SelectableText::new(
                body,
                spans,
                &[],
                rgb(palette.accent).into(),
                px(wrap),
                range,
                highlight,
                handler,
            )
            .with_marks(marks, find_color)
        };

        // A question is a bubble in the answer's column, so it has to leave the
        // icons beside it inside that column too.
        let question_width = question_text_width(self.answer_width);

        match message.role {
            Role::User => div()
                .id(("message", index))
                .flex()
                .flex_row()
                .items_center()
                .justify_end()
                .gap_2()
                .w_full()
                .when(editable, |this| {
                    this.child(
                        div()
                            .id(("edit-question", index))
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .size(px(24.))
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.edit_last_question(window, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/rename.svg")
                                    .w(px(14.))
                                    .h(px(14.))
                                    .text_color(rgb(palette.faint)),
                            ),
                    )
                })
                .child(self.copy_button(index, message, palette, cx))
                .child(
                    div()
                        .max_w(px(question_width + 2. * BUBBLE_PAD))
                        .px_3()
                        .py_2()
                        .rounded_lg()
                        .bg(rgb(palette.panel_soft))
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .whitespace_normal()
                        // Shaped at the width the bubble is capped to, so a long
                        // question wraps inside it instead of spilling past its
                        // edge and losing the start of every line.
                        .child(selectable(
                            0,
                            message.text.clone().into(),
                            question_width,
                            &[],
                        )),
                )
                .into_any_element(),
            Role::Assistant => div()
                .id(("message", index))
                .flex()
                .flex_col()
                .gap_2()
                // A definite width, measured from the pane: the answer uses the
                // whole chat column, so the wrap width passed to
                // `markdown_blocks` is this minus the bubble's own padding.
                .w(px(self.answer_width + 2. * BUBBLE_PAD))
                .min_w(px(0.))
                .px_3()
                .py_2()
                .text_sm()
                .text_color(rgb(palette.text))
                .whitespace_normal()
                // A reasoner's thinking sits above its answer, closed by
                // default: the reply is what the reader came for, and the
                // reasoning is one click away when they want it.
                .when(!message.reasoning.trim().is_empty(), |this| {
                    this.child(self.render_reasoning(index, message, palette, cx))
                })
                .children(markdown_blocks(
                    &self.parsed_blocks(index, &message.text),
                    &message.text,
                    palette,
                    index,
                    self.answer_width,
                    selection,
                    &on_drag,
                    self.copied_code
                        .filter(|(message, _)| *message == index)
                        .map(|(_, block)| block),
                    &copy_handler,
                    self.open_details(),
                    &toggle_handler,
                    self.open_checks(),
                    &check_handler,
                    &self.chat_dir(self.active_chat),
                    &|key| self.block_scroll_handle(key),
                    find,
                ))
                // Who wrote it and what it cost closes the answer, with the
                // actions beside it: the row is the answer's signature, not a
                // column of its own.
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_h(px(24.))
                        .when(signature.is_some(), |this| {
                            this.child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(signature.clone().unwrap_or_default()),
                            )
                        })
                        .child(self.copy_button(index, message, palette, cx))
                        .when(retryable, |this| {
                            this.when(message.interrupted, |this| {
                                this.child(
                                    // The answer stopped mid-stream: pick it up
                                    // where it left off instead of asking again.
                                    div()
                                        .id(("continue-answer", index))
                                        .flex_shrink_0()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .text_xs()
                                        .text_color(rgb(palette.accent))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.continue_answer(cx)),
                                        )
                                        .child("继续"),
                                )
                            })
                            .child(self.regenerate_button(index, palette, cx))
                        })
                        .when(turn_undo.is_some(), |this| {
                            let count = turn_undo.unwrap_or(0);
                            this.child(
                                div()
                                    .id(("undo-turn", index))
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(rgb(palette.warn))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.undo_turn_writes(index, cx);
                                    }))
                                    .child(format!("撤销本轮 {count} 次写入")),
                            )
                        }),
                )
                .when(!turn_files.is_empty(), |this| {
                    let label = format!(
                        "{}本轮改动 {} 个文件",
                        if changes_open { "▾" } else { "▸" },
                        turn_files.len()
                    );
                    let mut block = this.child(
                        div()
                            .id(("changes-toggle", index))
                            .mt_1()
                            .flex_shrink_0()
                            .px_1()
                            .py_0p5()
                            .rounded_sm()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.changes_open =
                                    (view.changes_open != Some(index)).then_some(index);
                                cx.notify();
                            }))
                            .child(label),
                    );
                    if changes_open {
                        for (position, (path, existed)) in turn_files.iter().enumerate() {
                            let path_for_open = path.clone();
                            block = block.child(
                                div()
                                    .id(("changed-file", index * 64 + position))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .mt_1()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(format!(
                                                "{} {}",
                                                if *existed { "改" } else { "建" },
                                                path
                                            )),
                                    )
                                    .child(
                                        div()
                                            .id(("open-changed", index * 64 + position))
                                            .flex_shrink_0()
                                            .px_1()
                                            .py_0p5()
                                            .rounded_sm()
                                            .text_xs()
                                            .text_color(rgb(palette.accent))
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.open_changed_file(&path_for_open, cx);
                                            }))
                                            .child("打开"),
                                    ),
                            );
                        }
                    }
                    block
                })
                .into_any_element(),
            Role::Thinking => {
                // One row per stretch of thinking: a sky-blue prefix, the first
                // line while closed, the whole text once opened — the same
                // shape as a tool row, because it is the same kind of record.
                let expanded = message.open;
                let first = message
                    .text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let width = tool_row_text_width(self.answer_width);
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .min_w(px(0.))
                    .child(
                        div()
                            .id(("thinking-row", index))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .min_w(px(0.))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.toggle_reasoning(index, cx);
                            }))
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(rgb(palette.panel_soft))
                                    .text_xs()
                                    .text_color(rgb(palette.sky))
                                    .child("思考"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .w(px(width))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(first),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(rgb(palette.faint))
                                    .child(if expanded { "▾ 收起" } else { "▸ 展开" }),
                            ),
                    )
                    .when(expanded, |this| {
                        this.child(
                            div()
                                .id(("thinking-detail", index))
                                .ml(px(DETAIL_INDENT))
                                .w(px(width))
                                .max_h(px(DETAIL_MAX_HEIGHT))
                                .overflow_y_scroll()
                                .rounded_md()
                                .bg(rgb(palette.background))
                                .border_1()
                                .border_color(rgb(palette.border))
                                .p(px(DETAIL_PAD))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .whitespace_normal()
                                .contained_wheel()
                                .child(message.text.clone()),
                        )
                    })
                    .into_any_element()
            }
            Role::Tool => {
                // The engine keeps a capped copy of the arguments and result;
                // the row shows the summary until the reader asks for them.
                let detailed = !message.input.is_empty() || !message.output.is_empty();
                let expanded = detailed && message.open;
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .child(
                        div()
                            .id(("tool-row", index))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .when(detailed, |this| {
                                this.cursor_pointer().on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        view.toggle_tool_row(index, window, cx)
                                    },
                                ))
                            })
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_1()
                                    .rounded_sm()
                                    // The badge sits straight on the panel, where
                                    // `panel` would be invisible in the light
                                    // theme: both states keep the same soft
                                    // surface and only the text differs.
                                    .bg(rgb(palette.panel_soft))
                                    .text_xs()
                                    .text_color(rgb(if message.failed {
                                        palette.failed
                                    } else {
                                        palette.ok
                                    }))
                                    .child("工具"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    // A definite width: a long command would
                                    // otherwise measure at its full length and
                                    // carry the row (and the column) past the
                                    // pane, where the window cuts it off.
                                    .w(px(tool_row_text_width(self.answer_width)))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(if message.failed {
                                        palette.failed
                                    } else {
                                        palette.muted
                                    }))
                                    .child(message.text.clone()),
                            )
                            .when(detailed, |this| {
                                this.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(if expanded { "▾ 收起" } else { "▸ 详情" }),
                                )
                            }),
                    )
                    .when(expanded, |this| {
                        // The detail keeps its own scroll box and its own bar: a
                        // very long result stays compact, and the wheel over the
                        // detail is stopped below so it cannot carry the
                        // transcript along with it.
                        let vertical = self.block_scroll_handle((index, usize::MAX));
                        let (width, text_width) = tool_detail_widths(self.answer_width);
                        // The bar is placed from the detail's own bounds, which
                        // only exist once it has been painted: ask for one more
                        // frame, or a row restored as open would keep its detail
                        // without a bar until something else redraws.
                        if vertical.bounds().size.height <= px(0.) {
                            let entity = cx.entity().entity_id();
                            window.on_next_frame(move |_, cx| cx.notify(entity));
                        }
                        this.child(
                            div()
                                .id(("tool-detail", index))
                                .flex()
                                .flex_row()
                                .ml(px(DETAIL_INDENT))
                                // An explicit width, like every other block of
                                // text in the transcript: a long unbroken line
                                // would otherwise measure at its full length and
                                // push the whole column past the window.
                                .w(px(width))
                                .max_h(px(DETAIL_MAX_HEIGHT))
                                .min_h(px(0.))
                                // Nothing may paint outside the box: the body is
                                // shaped at a width of its own, and a line a few
                                // pixels too long would otherwise run past the
                                // panel and over the scrollbar.
                                .overflow_hidden()
                                .rounded_md()
                                .bg(rgb(palette.background))
                                .border_1()
                                .border_color(rgb(palette.border))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                // Bubble order is innermost first, so the detail
                                // has already scrolled by the time this runs;
                                // what is left to stop is the transcript's wheel.
                                .contained_wheel()
                                .child(
                                    div()
                                        .id(("tool-scroll", index))
                                        .flex()
                                        .flex_col()
                                        .w(px(width - DETAIL_BAR - 2.))
                                        .max_h(px(DETAIL_MAX_HEIGHT - 2.))
                                        .min_h(px(0.))
                                        .gap_1()
                                        .px(px(DETAIL_PAD))
                                        .py_1()
                                        .overflow_y_scroll()
                                        .track_scroll(&vertical)
                                        .when(!message.input.is_empty(), |this| {
                                            this.child(tool_detail_section(
                                                "参数",
                                                &message.input,
                                                palette,
                                                text_width,
                                            ))
                                        })
                                        .when(
                                            !message.output.is_empty() || is_web_row(&message.text),
                                            |this| {
                                                if is_web_row(&message.text) {
                                                    this.child(web_sources_section(
                                                        &message.text,
                                                        &message.output,
                                                        index,
                                                        palette,
                                                        text_width,
                                                    ))
                                                } else {
                                                    this.child(tool_detail_section(
                                                        "返回",
                                                        &message.output,
                                                        palette,
                                                        text_width,
                                                    ))
                                                }
                                            },
                                        ),
                                )
                                .child(self.render_tool_bar(index, palette, &vertical, cx)),
                        )
                    })
                    .into_any_element()
            }
            Role::Approval => {
                // A write, command or destructive file action waits here until
                // the reader allows or refuses it; a card whose request is gone
                // (restart, stop) says so instead.
                let live = message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.approval.as_deref() == Some(message.call.as_str())
                    });
                let state = message.output.trim();
                // A card opens with its diff so the reader can review it, and
                // folds once the decision is made; the header brings it back.
                let detailed = !message.input.is_empty();
                let expanded = detailed && message.open;
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(if live { palette.warn } else { palette.border }))
                            .child(
                                div()
                                    .id(("approval-head", index))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgb(palette.panel_soft))
                                            .text_xs()
                                            .text_color(rgb(palette.warn))
                                            .child(approval_label(&message.tool)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_sm()
                                            .text_color(rgb(palette.text))
                                            .child(message.text.clone()),
                                    )
                                    .when(detailed, |this| {
                                        this.cursor_pointer().on_click(cx.listener(
                                            move |view, _, window, cx| {
                                                view.toggle_tool_row(index, window, cx)
                                            },
                                        ))
                                    })
                                    .when(detailed, |this| {
                                        this.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_xs()
                                                .text_color(rgb(palette.faint))
                                                .child(if expanded {
                                                    "▾ 收起"
                                                } else {
                                                    "▸ 详情"
                                                }),
                                        )
                                    }),
                            )
                            .when(expanded, |this| {
                                // Same scroll box and bar as an expanded tool
                                // detail: the diff keeps its own scroll handle,
                                // the bar drags through the shared tool-drag
                                // path, and the wheel stops at this box instead
                                // of carrying the transcript along.
                                let vertical = self.block_scroll_handle((index, usize::MAX));
                                let (width, text_width) = approval_detail_widths(self.answer_width);
                                if vertical.bounds().size.height <= px(0.) {
                                    let entity = cx.entity().entity_id();
                                    window.on_next_frame(move |_, cx| cx.notify(entity));
                                }
                                let drag: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> =
                                    Rc::new(|_, _, _| {});
                                this.child(
                                    div()
                                        .id(("approval-detail", index))
                                        .flex()
                                        .flex_row()
                                        .w(px(width))
                                        .max_h(px(APPROVAL_MAX_HEIGHT))
                                        .min_h(px(0.))
                                        .overflow_hidden()
                                        .rounded_md()
                                        .bg(rgb(palette.background))
                                        .border_1()
                                        .border_color(rgb(palette.border))
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .contained_wheel()
                                        .child(
                                            div()
                                                .id(("approval-scroll", index))
                                                .flex()
                                                .flex_col()
                                                .w(px(width - DETAIL_BAR - 2.))
                                                .max_h(px(APPROVAL_MAX_HEIGHT - 2.))
                                                .min_h(px(0.))
                                                .px(px(DETAIL_PAD))
                                                .py_1()
                                                .overflow_y_scroll()
                                                .track_scroll(&vertical)
                                                .child(
                                                    div()
                                                        .w(px(text_width))
                                                        .font_family(MONO_FONT)
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .whitespace_normal()
                                                        .child(SelectableText::new(
                                                            SharedString::from(
                                                                message.input.clone(),
                                                            ),
                                                            &[],
                                                            &[],
                                                            rgb(palette.accent).into(),
                                                            px(text_width),
                                                            None,
                                                            rgb(palette.selection).into(),
                                                            drag,
                                                        )),
                                                ),
                                        )
                                        .child(self.render_tool_bar(index, palette, &vertical, cx)),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .when(live, |this| {
                                        this.child(
                                            div()
                                                .id(("approve", index))
                                                .px_3()
                                                .py_1()
                                                .rounded_md()
                                                .bg(rgb(palette.accent))
                                                .text_xs()
                                                .text_color(rgb(palette.accent_text))
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.answer_approval(index, true, cx);
                                                }))
                                                .child("允许"),
                                        )
                                        .child(
                                            div()
                                                .id(("deny", index))
                                                .px_3()
                                                .py_1()
                                                .rounded_md()
                                                .border_1()
                                                .border_color(rgb(palette.border))
                                                .bg(rgb(palette.panel_soft))
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.answer_approval(index, false, cx);
                                                }))
                                                .child("拒绝"),
                                        )
                                        .when(
                                            allow_session_button(&message.tool, &message.input),
                                            |this| {
                                                this.child(
                                                    div()
                                                        .id(("allow-session", index))
                                                        .px_2()
                                                        .py_1()
                                                        .rounded_md()
                                                        .border_1()
                                                        .border_color(rgb(palette.border))
                                                        .bg(rgb(palette.panel_soft))
                                                        .text_xs()
                                                        .text_color(rgb(palette.muted))
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(
                                                            move |view, _, _, cx| {
                                                                view.allow_session_call(index, cx);
                                                            },
                                                        ))
                                                        .child("本次会话都允许"),
                                                )
                                            },
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.faint))
                                                .child(approval_hint(&message.tool)),
                                        )
                                    })
                                    .when(!live, |this| {
                                        this.child(
                                            div().text_xs().text_color(rgb(palette.faint)).child(
                                                if state.is_empty() {
                                                    "已失效".to_string()
                                                } else {
                                                    state.to_string()
                                                },
                                            ),
                                        )
                                        .when(
                                            message.output == "已写入"
                                                && message.undoable
                                                && !message.path.is_empty(),
                                            |this| {
                                                this.child(
                                                    div()
                                                        .id(("undo-write", index))
                                                        .px_2()
                                                        .py_1()
                                                        .rounded_md()
                                                        .border_1()
                                                        .border_color(rgb(palette.border))
                                                        .bg(rgb(palette.panel_soft))
                                                        .text_xs()
                                                        .text_color(rgb(palette.warn))
                                                        .cursor_pointer()
                                                        .hover(move |style| {
                                                            style.bg(rgb(palette.panel_soft))
                                                        })
                                                        .on_mouse_down(
                                                            MouseButton::Left,
                                                            cx.listener(|_, _, _, cx| {
                                                                cx.stop_propagation()
                                                            }),
                                                        )
                                                        .on_click(cx.listener(
                                                            move |view, _, _, cx| {
                                                                view.undo_write(index, cx);
                                                            },
                                                        ))
                                                        .child("撤销这次写入"),
                                                )
                                            },
                                        )
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            Role::Attachment => {
                // A file the person handed over: the card keeps it in the
                // transcript, folded until asked for, and the model reads it
                // through the history.
                let chars = message.text.chars().count();
                let preview: String = message.text.chars().take(MAX_ATTACHMENT_PREVIEW).collect();
                let cut = chars > MAX_ATTACHMENT_PREVIEW;
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .child(
                                div()
                                    .id(("attachment-row", index))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(move |view, _, window, cx| {
                                        view.toggle_tool_row(index, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgb(palette.panel_soft))
                                            .text_xs()
                                            .text_color(rgb(palette.sky))
                                            .child(if message.kind == AttachmentKind::Text {
                                                "附件".to_string()
                                            } else {
                                                message.kind.label().to_string()
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .truncate()
                                            .text_sm()
                                            .text_color(rgb(palette.text))
                                            .child(message.path.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(if message.kind == AttachmentKind::Text {
                                                format!("{} 字符", short_count(chars as u64))
                                            } else {
                                                "随本轮发送".to_string()
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(if message.open { "收起" } else { "展开" }),
                                    ),
                            )
                            .when(message.open, |this| {
                                this.child(
                                    div()
                                        .id(("attachment-body", index))
                                        .max_h(px(260.))
                                        .overflow_y_scroll()
                                        .contained_wheel()
                                        .rounded_md()
                                        .bg(rgb(palette.background))
                                        .border_1()
                                        .border_color(rgb(palette.border))
                                        .px_2()
                                        .py_1()
                                        .font_family(MONO_FONT)
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .whitespace_normal()
                                        .child(if cut {
                                            format!(
                                                "{preview}\n…（仅显示前 {} 个字符，共 {} 个）",
                                                MAX_ATTACHMENT_PREVIEW, chars
                                            )
                                        } else {
                                            preview
                                        }),
                                )
                            }),
                    )
                    .into_any_element()
            }
            Role::Question => {
                // A structured question is answered by picking one of its
                // options; the card folds once a choice has been sent.
                let live = message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.question.as_deref() == Some(message.call.as_str())
                    });
                let state = message.output.trim();
                let payload: serde_json::Value =
                    serde_json::from_str(&message.input).unwrap_or_default();
                let options: Vec<String> = payload
                    .get("options")
                    .and_then(|options| options.as_array())
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| option.as_str())
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut buttons = div().flex().flex_row().flex_wrap().gap_1();
                for (choice_index, option) in options.iter().enumerate() {
                    let choice = option.clone();
                    buttons = buttons.child(
                        div()
                            .id(("question-option", index * 16 + choice_index))
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(palette.accent))
                            .text_xs()
                            .text_color(rgb(palette.accent_text))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.answer_question(index, choice.clone(), cx);
                            }))
                            .child(option.clone()),
                    );
                }
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(if live { palette.warn } else { palette.border }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgb(palette.panel_soft))
                                            .text_xs()
                                            .text_color(rgb(palette.warn))
                                            .child("提问"),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .text_sm()
                                            .text_color(rgb(palette.text))
                                            .whitespace_normal()
                                            .child(message.text.clone()),
                                    ),
                            )
                            .when(live, |this| this.child(buttons))
                            .when(!live, |this| {
                                this.child(div().text_xs().text_color(rgb(palette.faint)).child(
                                    if state.is_empty() {
                                        "已失效".to_string()
                                    } else {
                                        state.to_string()
                                    },
                                ))
                            }),
                    )
                    .into_any_element()
            }
            Role::Plan => {
                // A plan is shown whole: it is the thing being approved.
                let live = message.output.trim().is_empty()
                    && self.active_pending().is_some_and(|pending| {
                        pending.plan.as_deref() == Some(message.call.as_str())
                    });
                let state = message.output.trim();
                div()
                    .id(("message", index))
                    .flex()
                    .flex_col()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(if live { palette.warn } else { palette.border }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(rgb(palette.panel_soft))
                                            .text_xs()
                                            .text_color(rgb(palette.warn))
                                            .child("计划"),
                                    )
                                    .child(div().text_xs().text_color(rgb(palette.faint)).child(
                                        if live {
                                            "等待批准".to_string()
                                        } else if state.is_empty() {
                                            "已失效".to_string()
                                        } else {
                                            state.to_string()
                                        },
                                    )),
                            )
                            .child(
                                div()
                                    .id(("plan-body", index))
                                    .w_full()
                                    .max_h(px(300.))
                                    .overflow_y_scroll()
                                    .contained_wheel()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .whitespace_normal()
                                    .child(message.text.clone()),
                            )
                            .when(live, |this| {
                                this.child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id(("plan-approve", index))
                                                .px_3()
                                                .py_1()
                                                .rounded_md()
                                                .bg(rgb(palette.accent))
                                                .text_xs()
                                                .text_color(rgb(palette.accent_text))
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.answer_plan(index, true, cx);
                                                }))
                                                .child("批准并执行"),
                                        )
                                        .child(
                                            div()
                                                .id(("plan-revise", index))
                                                .px_3()
                                                .py_1()
                                                .rounded_md()
                                                .border_1()
                                                .border_color(rgb(palette.border))
                                                .bg(rgb(palette.panel_soft))
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    view.answer_plan(index, false, cx);
                                                }))
                                                .child("继续修改"),
                                        ),
                                )
                            }),
                    )
                    .into_any_element()
            }
            Role::Note => {
                // A failure ends the transcript, and the only useful next step is
                // the same question again: offer it here, where the error is.
                let retry = retry_slot == Some(index);
                // If the failure cut an answer short, its text is still the row
                // above: continuing is as valid as asking the question again.
                let can_continue = idle && retry && continuable(messages);
                div()
                    .id(("message", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(
                        div()
                            // Capped so the buttons beside it stay inside the
                            // column; a wider box is drawn past the pane's edge.
                            .max_w(px(note_width(self.answer_width)))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(if message.failed {
                                palette.failed
                            } else {
                                palette.border
                            }))
                            .text_xs()
                            .text_color(rgb(if message.failed {
                                palette.failed
                            } else {
                                palette.muted
                            }))
                            .whitespace_normal()
                            .child(message.text.clone()),
                    )
                    .when(retry, |this| {
                        this.child(
                            div()
                                .id(("note-retry", index))
                                .flex_shrink_0()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .text_xs()
                                .text_color(rgb(palette.sky))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                .on_click(cx.listener(|view, _, _, cx| view.regenerate(cx)))
                                .child("重试"),
                        )
                    })
                    // Engine errors are long and worth pasting elsewhere.
                    .when(message.failed, |this| {
                        this.child(self.copy_button(index, message, palette, cx))
                    })
                    .when(can_continue, |this| {
                        this.child(
                            div()
                                .id(("note-continue", index))
                                .flex_shrink_0()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                .on_click(cx.listener(|view, _, _, cx| view.continue_answer(cx)))
                                .child("继续"),
                        )
                    })
                    .into_any_element()
            }
        }
    }
}

/// One labelled half of an expanded tool row.
/// Whether a tool row came from the provider's own web tools.
///
/// The row's title starts with the tool name (`web_search · …`), the same way
/// the actions card reads it.
fn is_web_row(text: &str) -> bool {
    let tool = text.split('·').next().unwrap_or_default().trim();
    tool == "web_search" || tool == "read_url"
}

/// One line of a web row's output, as `标题 — URL`.
fn web_source(line: &str) -> Option<(String, String)> {
    let (title, url) = line.split_once(" — ")?;
    let url = url.trim();
    if !crate::markdown::is_openable_link(url) {
        return None;
    }
    Some((title.trim().to_string(), url.to_string()))
}

/// The page a `read_url` row opened, read out of its title.
fn web_page_of(text: &str) -> Option<String> {
    let mut parts = text.splitn(2, '·');
    parts.next()?;
    let rest = parts.next()?.trim();
    let url = rest.split(" — ").next()?.trim();
    crate::markdown::is_openable_link(url).then(|| url.to_string())
}

/// One clickable source row: the title on the left, an "打开" hint on the right.
fn web_source_row(
    id: (&'static str, usize),
    label: String,
    url: String,
    palette: Palette,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .px_1()
        .py_0p5()
        .rounded_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(move |_, _, cx| {
            // Answers are untrusted text: `open_url` refuses anything that is
            // not an http(s) or mailto address, and never goes through a shell.
            let _ = crate::app::util::open_url(&url);
            cx.stop_propagation();
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .text_color(rgb(palette.text))
                .child(label),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.accent))
                .child("打开"),
        )
        .into_any_element()
}

/// The body of a web tool row: one clickable row per source, or the page it read.
///
/// The provider's citations arrive as plain text in the row's output, so the
/// transcript turns them back into links here instead of leaving a URL the
/// reader has to copy out by hand.
fn web_sources_section(
    text: &str,
    output: &str,
    index: usize,
    palette: Palette,
    width: f32,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    for (at, line) in output.lines().enumerate() {
        let Some((title, url)) = web_source(line) else {
            continue;
        };
        rows.push(web_source_row(
            ("web-source", index * 1024 + at),
            title,
            url,
            palette,
        ));
    }
    let mut header = format!("来源 {}", rows.len());
    if let Some(page) = web_page_of(text) {
        if rows.is_empty() {
            header = "页面".to_string();
        }
        rows.push(web_source_row(
            ("web-source", index * 1024 + rows.len()),
            page.clone(),
            page,
            palette,
        ));
    }
    if rows.is_empty() {
        // A failed call keeps its error text; a search the endpoint reported
        // without any result list says so instead of showing nothing at all.
        if !output.trim().is_empty() {
            return tool_detail_section("返回", output, palette, width);
        }
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(width))
            .child(div().text_xs().text_color(rgb(palette.faint)).child("来源"))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("端点没有返回来源列表，只报告了这次调用。"),
            )
            .into_any_element();
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .w(px(width))
        .child(div().text_xs().text_color(rgb(palette.faint)).child(header))
        .children(rows)
        .into_any_element()
}

fn tool_detail_section(title: &str, body: &str, palette: Palette, width: f32) -> AnyElement {
    // Shaped at a fixed width, like the markdown blocks: a plain text child
    // measures at its longest line and would overflow the detail instead of
    // wrapping inside it.
    let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> = Rc::new(|_, _, _| {});
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .w(px(width))
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(title.to_string()),
        )
        .child(
            div()
                .w(px(width))
                .font_family(MONO_FONT)
                .text_xs()
                .text_color(rgb(palette.text))
                .whitespace_normal()
                .child(SelectableText::new(
                    SharedString::from(body.to_string()),
                    &[],
                    &[],
                    rgb(palette.accent).into(),
                    px(width),
                    None,
                    rgb(palette.selection).into(),
                    handler,
                )),
        )
        .into_any_element()
}

/// Side padding inside a tool detail, either side.
pub(crate) const DETAIL_PAD: f32 = 8.;

/// Indent that lines a tool detail up under its row's text.
pub(crate) const DETAIL_INDENT: f32 = 34.;

/// Tallest a detail grows before its content scrolls.
pub(crate) const DETAIL_MAX_HEIGHT: f32 = 320.;

/// Tallest an approval diff grows before it scrolls.
pub(crate) const APPROVAL_MAX_HEIGHT: f32 = 260.;

/// Width of the approval card's diff box and the text inside it.
///
/// The card already spends `px_3` on each side of the row, so the answer column
/// width is exactly the room left for the box; the bar sits inside it, like a
/// tool detail's.
pub(crate) fn approval_detail_widths(answer_width: f32) -> (f32, f32) {
    let width = (answer_width - 2.).max(120.).floor();
    let text = (width - DETAIL_BAR - 2. * DETAIL_PAD - 12.)
        .max(80.)
        .floor();
    (width, text)
}

/// Width the detail's scrollbar takes on the right.
pub(crate) const DETAIL_BAR: f32 = 10.;

/// How wide the detail box and its text are, in a column `answer_width` wide.
///
/// Taken from the measured answer column rather than from a bounds read: the
/// first frame of an opened detail has no bounds yet, and a detail built from
/// that would keep its placeholder width for the rest of the conversation.
pub(crate) fn tool_detail_widths(answer_width: f32) -> (f32, f32) {
    // A margin on top of the measured column: the pane's own padding is not
    // subtracted from a `w_full` row in this layout, and a detail drawn a few
    // pixels too wide has its right edge cut off by the window.
    let column = answer_width + 2. * BUBBLE_PAD - DETAIL_MARGIN;
    let width = (column - DETAIL_INDENT).max(120.).floor();
    // A small gap before the bar, so the last glyph of a line never sits on the
    // border and the thumb has room to be grabbed.
    let text = (width - DETAIL_BAR - 2. * DETAIL_PAD - 12.)
        .max(80.)
        .floor();
    (width, text)
}

/// Room a tool row's one-line summary gets, after its badge and the chip at the
/// end of the row.
pub(crate) fn tool_row_text_width(answer_width: f32) -> f32 {
    (answer_width + 2. * BUBBLE_PAD - DETAIL_MARGIN - 104.)
        .max(80.)
        .floor()
}

/// Slack kept on the right of a tool row and its detail.
pub(crate) const DETAIL_MARGIN: f32 = 40.;

impl DshFlowApp {
    /// The scrollbar of one expanded tool detail.
    ///
    /// A real column beside the text rather than an overlay, and sized from the
    /// same handle the detail scrolls with, so the thumb and the offset can
    /// never disagree. It is only drawn while there is something to scroll.
    fn render_tool_bar(
        &self,
        index: usize,
        palette: Palette,
        handle: &ScrollHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((track, thumb, max, _)) = tool_bar_metrics(handle) else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-handle.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = matches!(self.tool_drag, Some((row, _, _)) if row == index);
        let bar = handle.clone();
        div()
            .id(("tool-bar", index))
            .w(px(DETAIL_BAR))
            .flex_shrink_0()
            // An explicit height: `100%` of an auto-height row is zero here, and
            // a zero-height column paints none of its children.
            .h(track + px(4.))
            .flex()
            .flex_col()
            .items_center()
            // The text underneath would otherwise take the press for a selection.
            .block_mouse_except_scroll()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    let Some((track, thumb, max, top)) = tool_bar_metrics(&bar) else {
                        return;
                    };
                    let travel = (track - thumb).max(px(1.));
                    let progress = if max > px(0.) {
                        (-bar.offset().y / max).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let thumb_top = top + travel * progress;
                    let on_thumb =
                        event.position.y >= thumb_top && event.position.y <= thumb_top + thumb;
                    if !on_thumb {
                        // A click on the track jumps there; the drag carries on
                        // from the new place, so click-and-pull works in one go.
                        let ratio = ((event.position.y - top) / travel).clamp(0., 1.);
                        let current = bar.offset();
                        bar.set_offset(point(current.x, -max * ratio));
                    }
                    view.tool_drag = Some((index, event.position.y, bar.offset().y));
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
}

/// Track height, thumb height, maximum offset and track top of a tool detail.
pub(crate) fn tool_bar_metrics(handle: &ScrollHandle) -> Option<(Pixels, Pixels, Pixels, Pixels)> {
    let bounds = handle.bounds();
    let max = handle.max_offset().y;
    let (track, thumb) = bar_geometry(bounds.size.height, max)?;
    Some((track, thumb, max, bounds.origin.y + px(2.)))
}

/// Track and thumb height for a bar over `viewport` with `max` left to scroll.
///
/// `None` means the content fits: a bar that cannot move is just noise.
pub(crate) fn bar_geometry(viewport: Pixels, max: Pixels) -> Option<(Pixels, Pixels)> {
    if viewport <= px(0.) || max <= px(8.) {
        return None;
    }
    let content = viewport + max;
    let track = (viewport - px(4.)).max(px(24.));
    let thumb = (track * (viewport / content).max(0.15))
        .max(px(24.))
        .min(track);
    Some((track, thumb))
}

/// The text a block draws.
///
/// [`markdown_blocks`] and the copy path both go through this, so a selection's
/// byte range always lines up with what is on screen. A list item's marker is
/// not part of it: the marker is drawn in its own column so wrapped lines line
/// up under the text, and copying a list gives its words, not its bullets.
pub(crate) fn block_body(block: &markdown::Block) -> String {
    match block {
        markdown::Block::Paragraph(text) => text.clone(),
        markdown::Block::Heading(_, text) => text.clone(),
        markdown::Block::Bullet(_, text) => text.clone(),
        markdown::Block::Task(_, _, text) | markdown::Block::Numbered(_, _, text) => text.clone(),
        markdown::Block::Table(table) => table.text(),
        markdown::Block::Code(_, body) => body.clone(),
        markdown::Block::Footnote(_, _) => String::new(),
        markdown::Block::Image(text, _) => text.clone(),
        markdown::Block::Math(formula) => formula.text.clone(),
        // A definition entry is one block of text: the term's line, then the
        // description's.
        markdown::Block::Definition(text) => text.clone(),
        // A rule draws no text of its own; a quote's, an indented block's and a
        // details block's text belongs to the blocks inside them.
        markdown::Block::Rule
        | markdown::Block::Quote(_)
        | markdown::Block::Indented(_, _, _)
        | markdown::Block::Details(_, _, _) => String::new(),
    }
}

/// The box drawn in front of a task item's text.
///
/// A tick glyph would have to come from a font that may not have one, so the box
/// says it on its own: an outline for an open item, a filled accent square for a
/// ticked one. It is smaller than the marker column and nudged down so it sits on
/// the middle of the first line.
fn task_marker(checked: bool, palette: Palette) -> AnyElement {
    // Square corners and a box smaller than the marker column: rounded corners
    // at this size read as a radio button, and a box that fills the column
    // touches the first word.
    let boxed = div().size(px(10.)).mt(px(5.));
    let boxed = if checked {
        boxed.bg(rgb(palette.accent))
    } else {
        boxed.border_1().border_color(rgb(palette.muted))
    };
    boxed.into_any_element()
}

/// The marker drawn in front of a list item, empty for other blocks.
pub(crate) fn list_marker(block: &markdown::Block) -> String {
    match block {
        markdown::Block::Bullet(..) => "•".to_string(),
        markdown::Block::Numbered(_, marker, _) => marker.clone(),
        _ => String::new(),
    }
}

/// Which list a block belongs to, so a run of items can be drawn as one block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListKind {
    /// `- item`.
    Bullet,
    /// `1. item`.
    Numbered,
}

impl ListKind {
    /// The list a block belongs to, `None` for everything that is not an item.
    pub(crate) fn of(block: &markdown::Block) -> Option<Self> {
        match block {
            // A task is a bullet with a box: it belongs to the same list.
            markdown::Block::Bullet(..) | markdown::Block::Task(..) => Some(Self::Bullet),
            markdown::Block::Numbered(..) => Some(Self::Numbered),
            _ => None,
        }
    }
}

/// Close the list being collected into `rows`.
///
/// The items of one list are spaced like the lines of a paragraph instead of
/// like separate blocks, so a long list reads as a list.
fn flush_list(list: &mut Vec<AnyElement>, rows: &mut Vec<AnyElement>) {
    if list.is_empty() {
        return;
    }
    rows.push(
        div()
            .flex()
            .flex_col()
            .w_full()
            .gap(px(LIST_GAP))
            .children(std::mem::take(list))
            .into_any_element(),
    );
}

/// The block texts of one answer, in drawing order.
pub(crate) fn answer_block_texts(text: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    push_block_texts(&markdown::parse_rich(text), &mut blocks);
    if blocks.is_empty() {
        vec![text.to_string()]
    } else {
        blocks
    }
}

/// The text of every block the renderer will draw, quotes unwrapped.
///
/// A quote puts one entry of its own (empty: the box draws no text) and then the
/// entries of the blocks inside it, which is the order [`markdown_blocks`] hands
/// out block numbers in.
fn push_block_texts(rich: &[markdown::Rich], out: &mut Vec<String>) {
    for item in rich {
        match &item.block {
            markdown::Block::Quote(inner)
            | markdown::Block::Details(_, _, inner)
            | markdown::Block::Footnote(_, inner) => {
                out.push(String::new());
                push_block_texts(inner, out);
            }
            markdown::Block::Indented(_, _, inner) => {
                out.push(String::new());
                push_block_texts(std::slice::from_ref(inner.as_ref()), out);
            }
            block => out.push(block_body(block)),
        }
    }
}

/// Font used for code blocks and inline code.
pub(crate) const MONO_FONT: &str = "Consolas";

/// Side padding of the transcript, either side (`px_5`).
pub(crate) const TRANSCRIPT_PAD: f32 = 20.;
/// Padding inside a bubble, either side (`px_3`).
pub(crate) const BUBBLE_PAD: f32 = 12.;
/// Padding inside a quote block, either side (`px_2`).
pub(crate) const QUOTE_PAD: f32 = 8.;
/// Padding inside a code block, either side (`px_3`).
pub(crate) const CODE_PAD: f32 = 12.;
/// Gap between the items of one list, so they read as one block (`gap_1`).
pub(crate) const LIST_GAP: f32 = 4.;
/// How far one nesting level insets a list item.
pub(crate) const LIST_INDENT: f32 = 16.;
/// Width of a bullet's marker column, so wrapped lines line up under the words.
pub(crate) const BULLET_MARKER: f32 = 14.;
/// Width of a numbered item's marker column: room for two digits and the dot.
pub(crate) const NUMBER_MARKER: f32 = 22.;
/// Body text size of an answer's blocks (`text_sm`).
pub(crate) const PARAGRAPH_SIZE: f32 = 14.;
/// A footnote's blocks are drawn this much smaller than the answer's text.
pub(crate) const FOOTNOTE_SCALE: f32 = 0.8;
/// Narrowest an answer's text gets, however small the window is.
pub(crate) const MIN_ANSWER_TEXT: f32 = 280.;
/// Widest the conversation column grows; wider windows split the rest evenly.
///
/// This is deliberately wider than the composer (`COMPOSER_MAX_WIDTH`): the
/// answer is the part that benefits from long lines, and the input card keeps
/// its own, narrower width.
pub(crate) const ANSWER_MAX_WIDTH: f32 = 900.;
/// Widest a question bubble grows; it is a short block, not a document.
pub(crate) const QUESTION_MAX_WIDTH: f32 = 620.;
/// Widest a failure note grows, so a long error stays readable but does not
/// stretch across a wide window.
pub(crate) const NOTE_MAX_WIDTH: f32 = 720.;
/// Room the icon buttons beside a bubble take, gaps included: two 24px buttons
/// and the `gap_2` before each.
///
/// A question has the pencil and copy there, a failed note the retry and copy,
/// and either pair has to fit beside the bubble inside the same column.
pub(crate) const BUBBLE_ACTIONS: f32 = 2. * 24. + 2. * 8.;
/// Padding inside an image placeholder, either side (`px_2`).
pub(crate) const IMAGE_CARD_PAD: f32 = 8.;
/// Width the 「图片」 badge takes in an image placeholder: two small glyphs, their
/// `px_1` padding, and the `gap_2` after them.
pub(crate) const IMAGE_BADGE: f32 = 30. + 8.;
/// Tallest a drawn picture may be, so one long screenshot cannot push the whole
/// answer off the screen; it is scaled down inside this and the bubble's width.
pub(crate) const IMAGE_MAX_HEIGHT: f32 = 480.;
/// Width of a footnote's `[1]` marker column.
pub(crate) const FOOTNOTE_MARKER: f32 = 28.;
/// Size of a display formula's text.
pub(crate) const MATH_SIZE: f32 = 16.;
/// How much smaller a script is than the formula it hangs off.
pub(crate) const MATH_SCRIPT: f32 = 0.72;

/// How much room an answer's text gets in a pane `pane` wide.
///
/// The transcript keeps [`transcript_pad`] either side, and the column that is
/// left stops at [`ANSWER_MAX_WIDTH`] so the space a very wide window leaves over
/// is split evenly between the two sides. Both the bubble's width and the wrap
/// width the text is shaped at come from this one number, so they cannot disagree
/// about where a line ends. The copy and regenerate buttons sit in the answer's
/// closing row, so they take no width from the text.
pub(crate) fn answer_text_width(pane: Pixels) -> f32 {
    let column = f32::from(pane) - 2. * transcript_pad(pane);
    (column - 2. * BUBBLE_PAD).max(MIN_ANSWER_TEXT).round()
}

/// How wide a question's text may wrap, in a column `answer_width` wide.
///
/// The question sits in the same column as the answers, but the row gives up
/// [`BUBBLE_ACTIONS`] to the icons beside the bubble: a bubble measured wider
/// than the room left over is drawn past the column's edge, where the text is
/// cut off. Wide windows still stop the bubble at [`QUESTION_MAX_WIDTH`].
pub(crate) fn question_text_width(answer_width: f32) -> f32 {
    (answer_width - BUBBLE_ACTIONS)
        .min(QUESTION_MAX_WIDTH - 2. * BUBBLE_PAD)
        .round()
}

/// How wide a failure note's box may grow, in a column `answer_width` wide.
///
/// Same reasoning as [`question_text_width`], measured on the bubble rather than
/// its text: the note lets its own text wrap, so the box is all it needs.
pub(crate) fn note_width(answer_width: f32) -> f32 {
    NOTE_MAX_WIDTH
        .min(answer_width + 2. * BUBBLE_PAD - BUBBLE_ACTIONS)
        .round()
}

/// Side padding the transcript keeps: the usual `px_5`, or more when the window
/// is wide enough for the conversation column to stop at its maximum width.
///
/// Every row of the conversation (questions included) sits inside this column,
/// so the spare room is shared by both sides instead of piling up on the right.
/// The chat header is a toolbar and keeps its own `px_5`.
pub(crate) fn transcript_pad(pane: Pixels) -> f32 {
    ((f32::from(pane) - ANSWER_MAX_WIDTH) / 2.)
        .max(TRANSCRIPT_PAD)
        .round()
}

/// A formula's pieces side by side, centred on the line.
///
/// A display formula may write its rows on separate lines with `\\`: each row is
/// laid out on its own, one under the other, instead of running together.
fn math_sequence(
    nodes: &[markdown::MathNode],
    palette: Palette,
    tone: u32,
    size: f32,
) -> AnyElement {
    let mut rows: Vec<Vec<&markdown::MathNode>> = vec![Vec::new()];
    for node in nodes {
        if matches!(node, markdown::MathNode::Break) {
            rows.push(Vec::new());
        } else if let Some(row) = rows.last_mut() {
            row.push(node);
        }
    }
    let row = |pieces: &[&markdown::MathNode]| {
        // An operator with its limits stands taller than the rest of the line;
        // aligning baselines would hang everything from its upper limit, so a
        // row holding one is centred instead.
        let limits = pieces.iter().any(|node| big_operator(node).is_some());
        let laid_out = div().flex().flex_row().children(
            pieces
                .iter()
                .map(|node| math_piece(node, palette, tone, size)),
        );
        if limits {
            laid_out.items_center().into_any_element()
        } else {
            laid_out.items_baseline().into_any_element()
        }
    };
    if rows.len() == 1 {
        return row(&rows[0]);
    }
    div()
        .flex()
        .flex_col()
        .items_center()
        .children(rows.iter().map(|pieces| row(pieces)))
        .into_any_element()
}

/// One piece of a formula: text, a stack of two, a raised or lowered script, or a
/// root with its bar.
fn math_piece(node: &markdown::MathNode, palette: Palette, tone: u32, size: f32) -> AnyElement {
    let script = size * MATH_SCRIPT;
    match node {
        markdown::MathNode::Text(text) => div()
            .font_family(MONO_FONT)
            .text_size(px(size))
            .text_color(rgb(tone))
            .child(text.clone())
            .into_any_element(),
        markdown::MathNode::Row(pieces) => {
            let limits = pieces.iter().any(|node| big_operator(node).is_some());
            let laid_out = div().flex().flex_row().children(
                pieces
                    .iter()
                    .map(|piece| math_piece(piece, palette, tone, size)),
            );
            if limits {
                laid_out.items_center().into_any_element()
            } else {
                laid_out.items_baseline().into_any_element()
            }
        }
        markdown::MathNode::Frac(first, second) => {
            // The bar needs a definite width: a percentage of an auto-width column
            // resolves to nothing, which leaves fractions with no line at all.
            let width = math_width(first, size).max(math_width(second, size)) + size * 0.4;
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(width))
                .child(math_piece(first, palette, tone, size))
                .child(div().w_full().h(px(1.)).my(px(1.)).bg(rgb(palette.muted)))
                .child(math_piece(second, palette, tone, size))
                .into_any_element()
        }
        // A script sits beside its base and is nudged up or down: `relative()`
        // moves it without asking the line to grow. A sum's limits go above and
        // below it instead, the way a display formula writes them.
        markdown::MathNode::Sup(base, power) | markdown::MathNode::Sub(base, power) => {
            if let Some((operator, lower, upper)) = big_operator(node) {
                return math_limits(operator, lower, upper, palette, tone, size);
            }
            let offset = if matches!(node, markdown::MathNode::Sup(..)) {
                -size * 0.35
            } else {
                size * 0.25
            };
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .child(math_piece(base, palette, tone, size))
                .child(
                    div()
                        .relative()
                        .top(px(offset))
                        .child(math_piece(power, palette, tone, script)),
                )
                .into_any_element()
        }
        markdown::MathNode::Sqrt(index, body) => {
            let body_width = math_width(body, size) + size * 0.2;
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .children(
                    index
                        .as_ref()
                        .map(|index| math_piece(index, palette, tone, script)),
                )
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(size * 1.15))
                        .text_color(rgb(tone))
                        .child("√"),
                )
                // The bar over the radicand is a hairline above it, as wide as the
                // radicand in the mono font's advance.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(px(body_width))
                        .child(div().w_full().h(px(1.)).bg(rgb(palette.muted)))
                        .child(math_piece(body, palette, tone, size)),
                )
                .into_any_element()
        }
        // `\binom{n}{k}`: the two parts stacked inside brackets, with no bar.
        markdown::MathNode::Binom(top, bottom) => {
            let glyph = |text: &'static str| {
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(size * 1.7))
                    .text_color(rgb(tone))
                    .child(text)
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(glyph("("))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .px(px(size * 0.15))
                        .child(math_piece(top, palette, tone, size))
                        .child(math_piece(bottom, palette, tone, size)),
                )
                .child(glyph(")"))
                .into_any_element()
        }
        // `\overline{x}` / `\underline{x}`: a hairline above or below the piece.
        markdown::MathNode::Overline(body, above) => {
            let rule = || div().w_full().h(px(1.)).bg(rgb(palette.muted));
            let width = math_width(body, size) + size * 0.2;
            let piece = math_piece(body, palette, tone, size);
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(width))
                .when(*above, |this| this.child(rule()))
                .child(piece)
                .when(!*above, |this| this.child(rule()))
                .into_any_element()
        }
        // A matrix: rows of cells in a grid, inside the bracket its environment
        // names. Every column is as wide as its widest cell, so `aligned` lines
        // up its `&` marks instead of each row drifting on its own.
        markdown::MathNode::Matrix(rows, bracket) => {
            let (left, right) = match bracket {
                Some(markdown::Bracket::Paren) => ("(", ")"),
                Some(markdown::Bracket::Square) => ("[", "]"),
                Some(markdown::Bracket::Bar) => ("|", "|"),
                Some(markdown::Bracket::Brace) => ("{", "}"),
                None => ("", ""),
            };
            let glyph = |text: &'static str| {
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(size * 1.7))
                    .text_color(rgb(tone))
                    .child(text)
            };
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            let widths: Vec<f32> = (0..columns)
                .map(|column| {
                    rows.iter()
                        .filter_map(|row| row.get(column))
                        .map(|cell| math_nodes_width(cell, size))
                        .fold(0., f32::max)
                })
                .collect();
            div()
                .flex()
                .flex_row()
                .items_center()
                .when(!left.is_empty(), |this| this.child(glyph(left)))
                .child(div().flex().flex_col().gap_1().px(px(size * 0.3)).children(
                    rows.iter().map(|row| {
                        div().flex().flex_row().items_center().gap_2().children(
                            row.iter().enumerate().map(|(column, cell)| {
                                div()
                                    .w(px(widths.get(column).copied().unwrap_or(0.)))
                                    .child(math_sequence(cell, palette, tone, size))
                            }),
                        )
                    }),
                ))
                .when(!right.is_empty(), |this| this.child(glyph(right)))
                .into_any_element()
        }
        // `\boxed{x}`: a frame drawn round the piece.
        markdown::MathNode::Framed(body) => div()
            .border_1()
            .border_color(rgb(tone))
            .px(px(size * 0.25))
            .py(px(1.))
            .child(math_piece(body, palette, tone, size))
            .into_any_element(),
        // Rows are split before a piece is drawn; a stray break draws nothing.
        markdown::MathNode::Break => div().into_any_element(),
    }
}

/// The drawn width of a piece, in pixels.
///
/// Pieces are drawn in the mono font, whose advance is about 0.62 em, and a stack
/// is as wide as its wider side. Rules are given this definite width because a
/// percentage inside a column that is sized by its content comes out as nothing.
fn math_width(node: &markdown::MathNode, size: f32) -> f32 {
    match node {
        markdown::MathNode::Text(text) => {
            text.chars()
                .map(|character| {
                    // A CJK glyph is about twice as wide as a mono ASCII one.
                    if character.is_ascii() { 0.62 } else { 1.0 }
                })
                .sum::<f32>()
                * size
        }
        markdown::MathNode::Break => 0.,
        markdown::MathNode::Row(pieces) => math_nodes_width(pieces, size),
        markdown::MathNode::Frac(first, second) => {
            math_width(first, size).max(math_width(second, size)) + size * 0.4
        }
        markdown::MathNode::Sup(base, power) | markdown::MathNode::Sub(base, power) => {
            math_width(base, size) + math_width(power, size * MATH_SCRIPT)
        }
        markdown::MathNode::Sqrt(index, body) => {
            index
                .as_ref()
                .map(|index| math_width(index, size * MATH_SCRIPT))
                .unwrap_or(0.)
                + size * 1.0
                + math_width(body, size)
        }
        markdown::MathNode::Binom(top, bottom) => {
            math_width(top, size).max(math_width(bottom, size)) + size * 1.4
        }
        markdown::MathNode::Overline(body, _) => math_width(body, size) + size * 0.2,
        markdown::MathNode::Framed(body) => math_width(body, size) + size * 0.8,
        markdown::MathNode::Matrix(rows, bracket) => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            let mut total = 0.;
            for column in 0..columns {
                total += rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| math_nodes_width(cell, size))
                    .fold(0., f32::max);
            }
            total
                + columns.saturating_sub(1) as f32 * 8.
                + if bracket.is_some() { size * 2.4 } else { 0. }
        }
    }
}

/// The drawn width of a run of pieces.
fn math_nodes_width(nodes: &[markdown::MathNode], size: f32) -> f32 {
    nodes.iter().map(|node| math_width(node, size)).sum()
}

/// A big operator with its limits, when `node` reads as one.
///
/// `\sum_{i=1}^{n}` parses as `Sup(Sub(Text("∑"), i=1), n)`, and a sum with only a
/// subscript as `Sub(Text("∑"), i=1)`.
fn big_operator(
    node: &markdown::MathNode,
) -> Option<(
    &'static str,
    Option<&markdown::MathNode>,
    Option<&markdown::MathNode>,
)> {
    match node {
        markdown::MathNode::Sup(base, power) => {
            if let Some(operator) = operator_of(base) {
                return Some((operator, None, Some(power)));
            }
            if let markdown::MathNode::Sub(inner, lower) = base.as_ref()
                && let Some(operator) = operator_of(inner)
            {
                return Some((operator, Some(lower), Some(power)));
            }
            None
        }
        markdown::MathNode::Sub(base, power) => {
            operator_of(base).map(|operator| (operator, Some(power.as_ref()), None))
        }
        _ => None,
    }
}

/// The big operator a node stands for, if it is one.
fn operator_of(node: &markdown::MathNode) -> Option<&'static str> {
    let markdown::MathNode::Text(text) = node else {
        return None;
    };
    Some(match text.as_str() {
        "∑" => "∑",
        "∏" => "∏",
        "∫" => "∫",
        "∮" => "∮",
        "⋃" => "⋃",
        "⋂" => "⋂",
        // Names whose limit sits under the word itself.
        "lim" => "lim",
        "limsup" => "lim sup",
        "liminf" => "lim inf",
        "argmax" => "argmax",
        "argmin" => "argmin",
        "max" => "max",
        "min" => "min",
        "sup" => "sup",
        "inf" => "inf",
        "det" => "det",
        "gcd" => "gcd",
        "Pr" => "Pr",
        _ => return None,
    })
}

/// An operator with its limits above and below, one size up.
fn math_limits(
    operator: &str,
    lower: Option<&markdown::MathNode>,
    upper: Option<&markdown::MathNode>,
    palette: Palette,
    tone: u32,
    size: f32,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .children(upper.map(|node| math_piece(node, palette, tone, size * MATH_SCRIPT)))
        .child(
            div()
                .font_family(MONO_FONT)
                .text_size(px(size * 1.2))
                .text_color(rgb(tone))
                .child(operator.to_string()),
        )
        .children(lower.map(|node| math_piece(node, palette, tone, size * MATH_SCRIPT)))
        .into_any_element()
}

/// The name a code block shows for its fence's info string.
///
/// Answers write `rs`, `ts` or `py`; the label says what those stand for, and an
/// unknown name is shown as written.
fn language_label(fence: &str) -> String {
    let name = fence.trim();
    let canonical = match name.to_ascii_lowercase().as_str() {
        "" => return String::new(),
        "rs" | "rust" => "Rust",
        "ts" | "typescript" => "TypeScript",
        "tsx" => "TSX",
        "js" | "javascript" => "JavaScript",
        "jsx" => "JSX",
        "py" | "python" => "Python",
        "sh" | "bash" | "zsh" | "shell" | "console" => "Shell",
        "go" | "golang" => "Go",
        "sql" => "SQL",
        "json" | "jsonc" => "JSON",
        "c" | "h" => "C",
        "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hxx" => "C++",
        "cs" | "csharp" | "c#" => "C#",
        "kt" | "kts" | "kotlin" => "Kotlin",
        "swift" => "Swift",
        "rb" | "ruby" => "Ruby",
        "php" => "PHP",
        "lua" => "Lua",
        "yml" | "yaml" => "YAML",
        "toml" => "TOML",
        "html" | "htm" => "HTML",
        "xml" | "svg" => "XML",
        "css" | "scss" | "less" => "CSS",
        "dart" => "Dart",
        "scala" | "sc" => "Scala",
        "r" => "R",
        "hs" | "haskell" => "Haskell",
        "ex" | "exs" | "elixir" => "Elixir",
        "ps1" | "pwsh" | "powershell" => "PowerShell",
        "dockerfile" | "docker" => "Dockerfile",
        "nix" => "Nix",
        "jl" | "julia" => "Julia",
        "makefile" | "make" | "mk" => "Makefile",
        "graphql" | "gql" => "GraphQL",
        "proto" | "protobuf" => "Protobuf",
        "zig" => "Zig",
        "sol" | "solidity" => "Solidity",
        "hcl" | "tf" | "terraform" => "HCL",
        "ml" | "ocaml" => "OCaml",
        "fs" | "fsharp" | "f#" => "F#",
        "clj" | "cljs" | "clojure" | "edn" => "Clojure",
        "vim" | "viml" | "vimscript" => "Vim",
        "diff" | "patch" | "udiff" => "Diff",
        _ => return name.to_string(),
    };
    canonical.to_string()
}

/// A `<details>` summary with its bold, italic, strike and underline kept.
///
/// A highlight has no font family of its own, so inline code inside a summary
/// is drawn in the normal font; everything else survives.
fn summary_text(cell: &markdown::Cell, tone: u32) -> AnyElement {
    let mut highlights = Vec::new();
    let mut start = 0;
    for span in &cell.spans {
        let end = start + span.len;
        if span.bold || span.italic || span.strike || span.underline {
            highlights.push((
                start..end,
                gpui::HighlightStyle {
                    font_weight: span.bold.then_some(gpui::FontWeight::BOLD),
                    font_style: span.italic.then_some(gpui::FontStyle::Italic),
                    underline: span.underline.then(|| gpui::UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    strikethrough: span.strike.then(|| gpui::StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..Default::default()
                },
            ));
        }
        start = end;
    }
    div()
        .flex_1()
        .min_w(px(0.))
        .text_sm()
        .text_color(rgb(tone))
        .child(gpui::StyledText::new(cell.text.clone()).with_highlights(highlights))
        .into_any_element()
}

/// The local file an image address points at, if one exists.
///
/// Nothing here uses the network: `file://` and absolute paths are read as they
/// are, a relative path is looked up in the conversation's own folder, and
/// anything else (`http:`, `https:`, bare hosts) is left to the placeholder.
fn local_image(url: &str, dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let raw = url.trim();
    if raw.is_empty() {
        return None;
    }
    let path = if let Some(rest) = raw.strip_prefix("file://") {
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        // `file:///C:/x` leaves a slash before the drive letter on Windows.
        let rest = match rest.as_bytes() {
            [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &rest[1..],
            _ => rest,
        };
        std::path::PathBuf::from(percent_decoded(rest))
    } else if raw.contains("://") || raw.starts_with("//") || raw.starts_with("www.") {
        return None;
    } else {
        let path = std::path::PathBuf::from(percent_decoded(raw));
        if path.is_absolute() {
            path
        } else {
            dir.join(path)
        }
    };
    path.is_file().then_some(path)
}

/// A path with its `%20`-style escapes turned back into characters.
///
/// Escapes are collected as bytes first, so a UTF-8 name such as `%E5%9B%BE` — the
/// character 图 — comes back whole instead of three mojibake letters.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3])
            && let Ok(value) = u8::from_str_radix(hex, 16)
        {
            out.push(value);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An assistant answer, laid out from blocks that were already parsed.
///
/// The caller keeps the parse (see `DshFlowApp::parsed_blocks`): this runs on
/// every frame, while a parse only has to happen when the text changes.
/// The gutter of a code block: how many lines it draws, and the numbers.
///
/// A trailing newline does not add a drawn line, which is also how the code
/// lays itself out; an empty block still shows `1` so the gutter keeps the same
/// height as the (blank) code beside it.
pub(crate) fn line_numbers(body: &str) -> (usize, String) {
    let lines = body.lines().count().max(1);
    let text = (1..=lines)
        .map(|line| line.to_string())
        .collect::<Vec<String>>()
        .join("\n");
    (lines, text)
}

pub(crate) fn markdown_blocks(
    blocks: &[markdown::Rich],
    text: &str,
    palette: Palette,
    message: usize,
    width: f32,
    selection: Option<&TextSelection>,
    on_drag: &Rc<dyn Fn(usize, DragEvent, &mut Window, &mut App)>,
    copied_block: Option<usize>,
    on_copy: &Rc<dyn Fn(usize, SharedString, &mut Window, &mut App)>,
    open: &std::collections::HashSet<(usize, usize)>,
    on_toggle: &Rc<dyn Fn(usize, &mut Window, &mut App)>,
    ticked: &std::collections::HashSet<(usize, usize)>,
    on_check: &Rc<dyn Fn(usize, &mut Window, &mut App)>,
    dir: &std::path::Path,
    block_scroll: &dyn Fn((usize, usize)) -> ScrollHandle,
    find: Option<&str>,
) -> Vec<AnyElement> {
    let mut number = 0;
    block_rows(
        blocks,
        &mut number,
        text,
        palette.text,
        palette,
        message,
        width,
        selection,
        on_drag,
        copied_block,
        on_copy,
        open,
        on_toggle,
        ticked,
        on_check,
        dir,
        block_scroll,
        0,
        1.0,
        find,
    )
}

/// Draw a run of blocks, handing each one the next number.
///
/// A quote is drawn by calling this again for the blocks inside it — with the
/// quote's own tone and a width that leaves out its padding — so a list, a
/// heading or a fence inside a quote keeps its shape instead of turning into one
/// line of markers and words.
fn block_rows(
    rich: &[markdown::Rich],
    counter: &mut usize,
    empty_text: &str,
    tone: u32,
    palette: Palette,
    message: usize,
    width: f32,
    selection: Option<&TextSelection>,
    on_drag: &Rc<dyn Fn(usize, DragEvent, &mut Window, &mut App)>,
    copied_block: Option<usize>,
    on_copy: &Rc<dyn Fn(usize, SharedString, &mut Window, &mut App)>,
    toggled_details: &std::collections::HashSet<(usize, usize)>,
    on_toggle: &Rc<dyn Fn(usize, &mut Window, &mut App)>,
    ticked: &std::collections::HashSet<(usize, usize)>,
    on_check: &Rc<dyn Fn(usize, &mut Window, &mut App)>,
    dir: &std::path::Path,
    block_scroll: &dyn Fn((usize, usize)) -> ScrollHandle,
    quote_depth: usize,
    // Text size multiplier: `1.0` for the answer, smaller inside a footnote.
    scale: f32,
    // Lower-cased find query, when this message is the hit being shown.
    find: Option<&str>,
) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = Vec::new();
    let highlight: Hsla = rgb(palette.selection).into();
    let find_color: Hsla = rgb(palette.find).into();

    // One block of text the pointer can drag over. Blocks are numbered so the
    // view can tell which one a drag belongs to.
    let selectable = |block: usize,
                      body: SharedString,
                      wrap: f32,
                      spans: &[markdown::Span],
                      links: &[markdown::Link]|
     -> SelectableText {
        let range = selection.and_then(|chosen| chosen.part_of(message, block, body.len()));
        let marks = find.map_or_else(Vec::new, |needle| {
            find_ranges(&body, needle, MAX_FIND_MARKS)
        });
        let on_drag = on_drag.clone();
        let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> =
            Rc::new(move |event, window, cx| on_drag(block, event, window, cx));
        SelectableText::new(
            body,
            spans,
            links,
            rgb(palette.accent).into(),
            px(wrap),
            range,
            highlight,
            handler,
        )
        .with_mark(rgb(palette.selection).into())
        .with_marks(marks, find_color)
    };

    // A code block keeps one line per source line and scrolls sideways, so a
    // long line is never broken in the middle of a token.
    let code_selectable = |block: usize,
                           body: SharedString,
                           spans: &[markdown::Span],
                           links: &[markdown::Link]|
     -> SelectableText {
        let range = selection.and_then(|chosen| chosen.part_of(message, block, body.len()));
        let marks = find.map_or_else(Vec::new, |needle| {
            find_ranges(&body, needle, MAX_FIND_MARKS)
        });
        let on_drag = on_drag.clone();
        let handler: Rc<dyn Fn(DragEvent, &mut Window, &mut App)> =
            Rc::new(move |event, window, cx| on_drag(block, event, window, cx));
        SelectableText::new(
            body,
            spans,
            links,
            rgb(palette.accent).into(),
            px(0.),
            range,
            highlight,
            handler,
        )
        .with_mark(rgb(palette.selection).into())
        .with_marks(marks, find_color)
        .without_wrap()
    };

    // Items of the list being collected. A list is one block of the answer, so
    // its items sit at `LIST_GAP` while the blocks around it keep the caller's
    // spacing.
    let mut list: Vec<AnyElement> = Vec::new();
    let mut list_kind: Option<ListKind> = None;

    for rich in rich {
        let number = *counter;
        *counter += 1;
        // The text the block draws, and the emphasis that covers it: both come
        // from the same parse, so a selection's bytes line up with the runs.
        // Kept as a `SharedString`: the code and table arms clone it into their
        // copy handlers, and a clone must not copy the text again.
        let body: SharedString = block_body(&rich.block).into();
        let emphasis = &rich.spans;
        let links = &rich.links;
        // A `-` list and a `1.` list are two lists even when they touch.
        let kind = ListKind::of(&rich.block);
        if kind != list_kind {
            flush_list(&mut list, &mut rows);
            list_kind = kind;
        }
        let element =
            match &rich.block {
                markdown::Block::Paragraph(_) => div()
                    .w_full()
                    .text_size(px(PARAGRAPH_SIZE * scale))
                    .text_color(rgb(tone))
                    .whitespace_normal()
                    .child(selectable(number, body, width, emphasis, links))
                    .into_any_element(),
                // A definition list has no marker column, so the term is drawn bold
                // on its own line with the description under it — which is also how
                // the block's text reads.
                markdown::Block::Definition(_) => div()
                    .w_full()
                    .text_size(px(PARAGRAPH_SIZE * scale))
                    .text_color(rgb(tone))
                    .whitespace_normal()
                    .pl(px(4.))
                    .child(selectable(number, body, width - 4., emphasis, links))
                    .into_any_element(),
                markdown::Block::Heading(level, _) => {
                    // A level one heading has to look like the top of the answer,
                    // not the same as the level two under it; the deep levels step
                    // down so an outline still shows its shape.
                    let size = match *level {
                        1 => 20.,
                        2 => 16.,
                        3 => 14.,
                        4 => 13.,
                        _ => 12.,
                    } * scale;
                    div()
                        .w_full()
                        .pt_1()
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_size(px(size))
                        .text_color(rgb(tone))
                        .whitespace_normal()
                        .child(selectable(number, body, width, emphasis, links))
                        .into_any_element()
                }
                // The marker is drawn in its own column, so the words start at the
                // same x on every line and a wrapped item hangs under them instead
                // of running back to the marker. A nested item is inset by its level.
                markdown::Block::Bullet(level, _)
                | markdown::Block::Numbered(level, _, _)
                | markdown::Block::Task(level, _, _) => {
                    let indent = *level as f32 * LIST_INDENT;
                    let marker_width = if matches!(rich.block, markdown::Block::Numbered(..)) {
                        NUMBER_MARKER
                    } else {
                        BULLET_MARKER
                    };
                    // A task draws a box where a bullet draws its dot. The box is a
                    // control: a click flips it, and the flip belongs to the
                    // conversation, so it survives a restart.
                    let marker: AnyElement = match &rich.block {
                        markdown::Block::Task(_, checked, _) => {
                            let checked = *checked != ticked.contains(&(message, number));
                            let on_check = on_check.clone();
                            let id = ("task", code_copy_id(message, number));
                            div()
                                .id(id)
                                .cursor_pointer()
                                .on_click(move |_, window, cx| on_check(number, window, cx))
                                .child(task_marker(checked, palette))
                                .into_any_element()
                        }
                        _ => div().child(list_marker(&rich.block)).into_any_element(),
                    };
                    div()
                        .w_full()
                        .pl(px(indent))
                        .flex()
                        .flex_row()
                        .items_start()
                        .text_size(px(PARAGRAPH_SIZE * scale))
                        .text_color(rgb(tone))
                        .child(div().flex_shrink_0().w(px(marker_width)).child(marker))
                        .child(selectable(
                            number,
                            body,
                            width - indent - marker_width,
                            emphasis,
                            links,
                        ))
                        .into_any_element()
                }
                // A thin line between sections, the same weight as the table's own
                // dividers.
                markdown::Block::Rule => div()
                    .w_full()
                    .py_1()
                    .child(div().w_full().h(px(1.)).bg(rgb(palette.border)))
                    .into_any_element(),
                // The quote is the other block that pads itself, so its text wraps
                // inside that padding instead of reaching the block's edge. What is
                // inside it is drawn as blocks of its own, one box around them all.
                // A quote inside a quote flips to the panel colour, so the nesting
                // is visible instead of two boxes of the same grey.
                markdown::Block::Quote(inner) => div()
                    .w_full()
                    .px(px(QUOTE_PAD))
                    .py_1()
                    .rounded_md()
                    .bg(rgb(if quote_depth.is_multiple_of(2) {
                        palette.panel_soft
                    } else {
                        palette.canvas
                    }))
                    .flex()
                    .flex_col()
                    .gap(px(LIST_GAP))
                    .children(block_rows(
                        inner.as_slice(),
                        counter,
                        "",
                        palette.muted,
                        palette,
                        message,
                        width - 2. * QUOTE_PAD,
                        selection,
                        on_drag,
                        copied_block,
                        on_copy,
                        toggled_details,
                        on_toggle,
                        ticked,
                        on_check,
                        dir,
                        block_scroll,
                        quote_depth + 1,
                        scale,
                        find,
                    ))
                    .into_any_element(),
                // A block written under a list item: drawn one level in, under the
                // item's text column, so a fence or a quote reads as part of it.
                markdown::Block::Indented(level, marker, inner) => {
                    // The child starts where the item's text starts: past its marker
                    // column, which is wider for a number than for a bullet.
                    let marker_width = match marker {
                        markdown::Marker::Bullet => BULLET_MARKER,
                        markdown::Marker::Number => NUMBER_MARKER,
                    };
                    let inset = *level as f32 * LIST_INDENT + marker_width;
                    div()
                        .w_full()
                        .pl(px(inset))
                        .flex()
                        .flex_col()
                        .gap(px(LIST_GAP))
                        .children(block_rows(
                            std::slice::from_ref(inner.as_ref()),
                            counter,
                            empty_text,
                            tone,
                            palette,
                            message,
                            width - inset,
                            selection,
                            on_drag,
                            copied_block,
                            on_copy,
                            toggled_details,
                            on_toggle,
                            ticked,
                            on_check,
                            dir,
                            block_scroll,
                            quote_depth,
                            scale,
                            find,
                        ))
                        .into_any_element()
                }
                // A picture on a line of its own. A local file — an absolute path, a
                // `file://` address, or a path relative to the conversation folder —
                // is drawn. A remote URL stays a card that names the image, because
                // nothing here fetches the network.
                markdown::Block::Image(alt, url) => match local_image(url, dir) {
                    Some(path) => {
                        let caption = alt.clone();
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                gpui::img(path)
                                    .max_w(px(width))
                                    .max_h(px(IMAGE_MAX_HEIGHT))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(palette.border)),
                            )
                            .when(!caption.trim().is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(caption),
                                )
                            })
                            .into_any_element()
                    }
                    None => div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.border))
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(palette.panel_soft))
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child("图片"),
                        )
                        .child(selectable(
                            number,
                            body,
                            width - 2. * IMAGE_CARD_PAD - IMAGE_BADGE,
                            emphasis,
                            links,
                        ))
                        .into_any_element(),
                },
                // One note under the answer: its number in the marker column, so a
                // wrapped note lines up under the text like a list item does. The
                // note's own blocks are drawn under the same column, in the faint
                // tone notes use.
                markdown::Block::Footnote(note, inner) => {
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_start()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(
                            div()
                                .flex_shrink_0()
                                .w(px(FOOTNOTE_MARKER))
                                .child(format!("[{note}]")),
                        )
                        .child(div().w_full().flex().flex_col().gap(px(LIST_GAP)).children(
                            block_rows(
                                inner,
                                counter,
                                "",
                                palette.faint,
                                palette,
                                message,
                                width - FOOTNOTE_MARKER,
                                selection,
                                on_drag,
                                copied_block,
                                on_copy,
                                toggled_details,
                                on_toggle,
                                ticked,
                                on_check,
                                dir,
                                block_scroll,
                                quote_depth,
                                scale * FOOTNOTE_SCALE,
                                find,
                            ),
                        ))
                        .into_any_element()
                }
                // A formula on its own line. What can be stacked is drawn as
                // numerator over denominator, the rest stays one line of text that a
                // selection can still copy.
                markdown::Block::Math(formula) => {
                    let stacked = formula.stacked();
                    let copied = copied_block == Some(number);
                    let on_copy = on_copy.clone();
                    let source: SharedString = formula.source.clone().into();
                    let inner: AnyElement = if stacked {
                        math_sequence(&formula.nodes, palette, tone, MATH_SIZE)
                    } else {
                        selectable(number, body, width - 2. * CODE_PAD, emphasis, links)
                            .into_any_element()
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .w_full()
                        .px(px(CODE_PAD))
                        .py_3()
                        .rounded_md()
                        .bg(rgb(palette.panel_soft))
                        // A drawn formula is not selectable, so it gets the copy
                        // button a code block has: the LaTeX, ready to paste on.
                        .when(stacked, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_end()
                                    .w_full()
                                    .child(
                                        div()
                                            .id(("math-copy", code_copy_id(message, number)))
                                            .px_1()
                                            .rounded_sm()
                                            .text_xs()
                                            .text_color(rgb(if copied {
                                                palette.ok
                                            } else {
                                                palette.faint
                                            }))
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.border)))
                                            .on_click(move |_, window, cx| {
                                                on_copy(number, source.clone(), window, cx);
                                            })
                                            .child(if copied { "已复制" } else { "复制" }),
                                    ),
                            )
                        })
                        .child(inner)
                        .into_any_element()
                }
                // `<details>`: the summary line opens and closes the blocks it holds.
                markdown::Block::Details(summary, open_default, inner) => {
                    // The reader's toggles are kept as flips of what the element
                    // asked for, so `<details open>` starts expanded and a click
                    // closes it.
                    let expanded = *open_default != toggled_details.contains(&(message, number));
                    let toggle = on_toggle.clone();
                    // The contents are built either way, so the block numbers handed
                    // out do not depend on whether the reader has opened the section.
                    let contents = block_rows(
                        inner,
                        counter,
                        "",
                        tone,
                        palette,
                        message,
                        width - 2. * QUOTE_PAD,
                        selection,
                        on_drag,
                        copied_block,
                        on_copy,
                        toggled_details,
                        on_toggle,
                        ticked,
                        on_check,
                        dir,
                        block_scroll,
                        quote_depth,
                        scale,
                        find,
                    );
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .px(px(QUOTE_PAD))
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette.border))
                        .child(
                            div()
                                .id(("details", code_copy_id(message, number)))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .w_full()
                                .cursor_pointer()
                                .on_click(move |_, window, cx| toggle(number, window, cx))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(if expanded { "▾" } else { "▸" }),
                                )
                                .child(summary_text(summary, tone)),
                        )
                        .when(expanded, move |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .w_full()
                                    .min_w(px(0.))
                                    .children(contents),
                            )
                        })
                        .into_any_element()
                }
                markdown::Block::Table(table) => {
                    let markdown: SharedString = table.markdown().into();
                    let range =
                        selection.and_then(|chosen| chosen.part_of(message, number, body.len()));
                    let mut cells: Vec<Vec<markdown::Cell>> =
                        Vec::with_capacity(table.rows.len() + 1);
                    cells.push(table.header.clone());
                    cells.extend(table.rows.clone());
                    let on_drag = on_drag.clone();
                    let handler: OnDrag =
                        Rc::new(move |event, window, cx| on_drag(number, event, window, cx));
                    let copied = copied_block == Some(number);
                    let on_copy = on_copy.clone();
                    // The table paints its own frame: the wrapper fills the answer so
                    // the copy button sits at its right edge, while the table itself
                    // stays only as wide as its columns.
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .w_full()
                        // Copies the table alone, as Markdown the model's own answer
                        // can be pasted into a document.
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_end()
                                .w_full()
                                .child(
                                    div()
                                        .id(("table-copy", code_copy_id(message, number)))
                                        .px_1()
                                        .rounded_sm()
                                        .text_xs()
                                        .text_color(rgb(if copied {
                                            palette.ok
                                        } else {
                                            palette.faint
                                        }))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(palette.border)))
                                        .on_click(move |_, window, cx| {
                                            on_copy(number, markdown.clone(), window, cx);
                                        })
                                        .child(if copied { "已复制" } else { "复制" }),
                                ),
                        )
                        .child(
                            div()
                                .id(("table-scroll", code_copy_id(message, number)))
                                .flex()
                                .flex_col()
                                .items_start()
                                .w_full()
                                .min_w(px(0.))
                                .overflow_x_scroll()
                                .track_scroll(&block_scroll((message, number)))
                                .child(SelectableTable::new(
                                    cells,
                                    body,
                                    px(width),
                                    range,
                                    table.aligns.clone(),
                                    TableLook {
                                        header: rgb(palette.panel_soft).into(),
                                        divider: rgb(palette.border).into(),
                                        selection: highlight,
                                        link: rgb(palette.accent).into(),
                                        mark: rgb(palette.selection).into(),
                                    },
                                    handler,
                                )),
                        )
                        .into_any_element()
                }
                markdown::Block::Code(language, body) => {
                    let language = language_label(language);
                    let copied = copied_block == Some(number);
                    let body: SharedString = body.clone().into();
                    let clickable = body.clone();
                    let on_copy = on_copy.clone();
                    // Keywords, strings, numbers and comments each get a colour; the
                    // palette has no code-specific fields, so the ones it has stand in.
                    let tokens = TokenColors {
                        comment: rgb(palette.faint).into(),
                        string: rgb(palette.ok).into(),
                        number: rgb(palette.warn).into(),
                        keyword: rgb(palette.accent).into(),
                        function: rgb(palette.code_function).into(),
                        type_name: rgb(palette.code_type).into(),
                        // A diff reads as "this line goes" and "this line comes",
                        // so it borrows the two colours those states already use.
                        added: rgb(palette.ok).into(),
                        removed: rgb(palette.failed).into(),
                    };
                    let block = div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .w_full()
                        .px(px(CODE_PAD))
                        .py_2()
                        .rounded_md()
                        .bg(rgb(palette.panel_soft))
                        // Language on the left, a copy button on the right; a block
                        // without a language still gets the button.
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .w_full()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(language),
                                )
                                .child(
                                    div()
                                        .id(("code-copy", code_copy_id(message, number)))
                                        .flex_shrink_0()
                                        .px_1()
                                        .rounded_sm()
                                        .text_xs()
                                        .text_color(rgb(if copied {
                                            palette.ok
                                        } else {
                                            palette.faint
                                        }))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(palette.border)))
                                        .on_click(move |_, window, cx| {
                                            on_copy(number, clickable.clone(), window, cx);
                                        })
                                        .child(if copied { "已复制" } else { "复制" }),
                                ),
                        );
                    // One number per source line: the code lays out unwrapped, so the
                    // gutter's line height matches the code's exactly. The numbers sit
                    // outside the sideways scroller, so a long line never drags them
                    // off; the copy button still copies the code alone.
                    let (_lines, gutter) = line_numbers(&body);
                    block
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_start()
                                .w_full()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .pr_2()
                                        .font_family(MONO_FONT)
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .child(gutter),
                                )
                                .child(
                                    div()
                                        .id(("code-scroll", code_copy_id(message, number)))
                                        .flex()
                                        .flex_col()
                                        .items_start()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .overflow_x_scroll()
                                        .track_scroll(&block_scroll((message, number)))
                                        .font_family(MONO_FONT)
                                        .text_xs()
                                        .text_color(rgb(tone))
                                        .whitespace_normal()
                                        .child(
                                            code_selectable(number, body, emphasis, &[])
                                                .with_tokens(tokens),
                                        ),
                                ),
                        )
                        .into_any_element()
                }
            };
        match kind {
            Some(_) => list.push(element),
            None => rows.push(element),
        }
    }
    flush_list(&mut list, &mut rows);
    if rows.is_empty() && !empty_text.is_empty() {
        rows.push(
            div()
                .w_full()
                .text_sm()
                .text_color(rgb(tone))
                .whitespace_normal()
                .child(selectable(
                    *counter,
                    empty_text.to_string().into(),
                    width,
                    &[],
                    &[],
                ))
                .into_any_element(),
        );
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_blocks_label_names_its_language() {
        assert_eq!(language_label("rs"), "Rust");
        assert_eq!(language_label("TS"), "TypeScript");
        assert_eq!(language_label("py"), "Python");
        assert_eq!(language_label("c++"), "C++");
        assert_eq!(language_label("yml"), "YAML");
        assert_eq!(language_label("ps1"), "PowerShell");
        assert_eq!(language_label("dockerfile"), "Dockerfile");
        assert_eq!(language_label("jl"), "Julia");
        assert_eq!(language_label("mk"), "Makefile");
        assert_eq!(language_label("gql"), "GraphQL");
        assert_eq!(language_label("proto"), "Protobuf");
        // An unknown name is shown as written; an empty one shows nothing.
        assert_eq!(language_label("brainfuck"), "brainfuck");
        assert_eq!(language_label("  "), "");
    }

    #[test]
    fn a_local_image_is_found_and_a_remote_one_is_not() {
        let dir = std::env::temp_dir().join(format!("dsh-flow-ui-img-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("图 1.png");
        std::fs::write(&file, b"x").unwrap();

        // A relative path is looked up in the conversation folder.
        assert_eq!(local_image("图 1.png", &dir), Some(file.clone()));
        // A `%20` in the address is the space in the name, and a
        // percent-escaped UTF-8 name decodes whole.
        assert_eq!(local_image("图%201.png", &dir), Some(file.clone()));
        assert_eq!(local_image("%E5%9B%BE%201.png", &dir), Some(file.clone()));
        // An absolute path and a `file://` address are used as they are.
        let absolute = file.to_string_lossy().to_string();
        assert_eq!(
            local_image(&absolute, std::path::Path::new(".")),
            Some(file.clone())
        );
        assert_eq!(
            local_image(&format!("file:///{}", absolute), &dir),
            Some(file.clone())
        );
        // Nothing remote is fetched, and an empty address is no image.
        assert_eq!(local_image("https://example.com/a.png", &dir), None);
        assert_eq!(local_image("www.example.com/a.png", &dir), None);
        assert_eq!(local_image("", &dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}

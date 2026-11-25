//! The title bar, the sidebar with its category accordion, and the workflow cards.

use super::*;

impl DshFlowApp {
    /// The custom title bar.
    ///
    /// The drag surface and the window buttons must not overlap: the platform
    /// looks up which control area is under the cursor and stops at the first
    /// match, so nesting the buttons inside the drag area would turn every click
    /// into "start dragging the window".
    pub(crate) fn render_titlebar(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(TITLEBAR_HEIGHT))
            .flex_shrink_0()
            .bg(rgb(palette.chrome))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .pl_3()
                    .window_control_area(WindowControlArea::Drag)
                    .child(
                        svg()
                            .path("icons/app.svg")
                            .w(px(16.))
                            .h(px(16.))
                            .text_color(rgb(palette.accent)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child("PAgent"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .h_full()
                    .flex_shrink_0()
                    // The help sheet lives beside the window buttons: the three
                    // to its right are the platform's control areas, this one is
                    // the app's own, so it must not claim one of them.
                    .child(
                        div()
                            .id("title-help")
                            .flex()
                            .items_center()
                            .justify_center()
                            .flex_shrink_0()
                            .w(px(34.))
                            .h_full()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.shortcuts_open = !view.shortcuts_open;
                                cx.notify();
                            }))
                            .child(
                                svg()
                                    .path("icons/help.svg")
                                    .w(px(15.))
                                    .h(px(15.))
                                    .text_color(rgb(if self.shortcuts_open {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    })),
                            ),
                    )
                    .child(window_button(
                        WindowControlArea::Min,
                        "−",
                        palette,
                        self.title_hover == Some(WindowControlArea::Min),
                    ))
                    .child(window_button(
                        WindowControlArea::Max,
                        "□",
                        palette,
                        self.title_hover == Some(WindowControlArea::Max),
                    ))
                    .child(window_button(
                        WindowControlArea::Close,
                        "×",
                        palette,
                        self.title_hover == Some(WindowControlArea::Close),
                    )),
            )
    }

    /// The sidebar: navigation rows, conversations, and workflow cards.
    pub(crate) fn render_sidebar(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        // Conversations are the "unfixed" cards: green leading bar, newest pinned
        // on top of the same list the discovered workflows live in.
        let needle = self.chat_filter.text.trim().to_lowercase();
        let filtering = !needle.is_empty();
        // Cards are bucketed by category id while they are built; the category
        // headers below drain their own bucket, so a card can only show once.
        let mut chat_cards: std::collections::HashMap<String, Vec<AnyElement>> =
            std::collections::HashMap::new();
        for index in chat_order(&self.chats) {
            let chat = &self.chats[index];
            if !needle.is_empty() && !chat_matches(chat, &needle) {
                continue;
            }
            let selected = self.view == View::Chat && index == self.active_chat;
            let busy = self.pending.iter().any(|pending| pending.dir == chat.dir);
            let hot = self.hovered_delete == Some(index);
            let confirming = self.confirm_delete == Some(index);
            let menu_open = self
                .card_menu
                .as_ref()
                .is_some_and(|(menu, _)| *menu == CardMenu::Chat(index));
            let title: AnyElement = div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_sm()
                .text_color(rgb(palette.text))
                .child(chat.title.clone())
                .into_any_element();
            // An unsent message is worth surfacing: it is the one thing a switch
            // away would otherwise hide.
            let meta = if confirming {
                format!("再点「删除对话」确认（{} 条消息）", chat.messages.len())
            } else {
                let meta = chat_meta(
                    chat.messages.len(),
                    busy,
                    chat.updated_ms,
                    self.chat_drafts.contains_key(&chat.dir)
                        || self.attachment_drafts.contains_key(&chat.dir),
                );
                if chat.pinned {
                    format!("置顶 · {meta}")
                } else {
                    meta
                }
            };
            // With a filter typed, the card also says which message matched and
            // opens right there instead of at the newest one.
            let hit = if filtering {
                chat_match(chat, &needle)
            } else {
                None
            };
            let hit_for_click = hit.clone();
            let hit_snippet = hit.as_ref().map(|(_, text)| text.clone());
            chat_cards.entry(self.category_of(chat)).or_default().push(
                div()
                    .id(("chat", index))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .mx_2()
                    // The list scrolls, so every row keeps its own height: a flex
                    // item shrinks by default, and a fixed one (a category header)
                    // would be squeezed to its text as soon as the cards overflow.
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(rgb(palette.canvas))
                    .border_1()
                    .border_color(rgb(if selected {
                        palette.sky
                    } else {
                        palette.canvas
                    }))
                    .shadow_md()
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        match &hit_for_click {
                            Some((message, _)) => view.open_chat_at_message(index, *message),
                            None => view.select_chat(index),
                        }
                        view.leave_fields(window, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap_2()
                            .child(div().pt(px(5.)).child(status_dot(palette.ok)))
                            .child(title)
                            // The card's own menu: rename it, move it to another
                            // category, or delete it.
                            .child(
                                div()
                                    .id(("chat-menu", index))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .flex_shrink_0()
                                    .mt(px(-4.))
                                    .mr(px(-9.))
                                    .size(px(18.))
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(
                                        move |view, event: &gpui::ClickEvent, _, cx| {
                                            // The card selects itself on click; without
                                            // this the conversation would change under
                                            // the menu.
                                            cx.stop_propagation();
                                            let same =
                                                view.card_menu.as_ref().is_some_and(|(menu, _)| {
                                                    *menu == CardMenu::Chat(index)
                                                });
                                            view.category_menu = None;
                                            view.confirm_delete = None;
                                            view.card_menu = if same {
                                                None
                                            } else {
                                                Some((CardMenu::Chat(index), event.position()))
                                            };
                                            cx.notify();
                                        },
                                    ))
                                    .child(
                                        svg()
                                            .path("icons/ellipsis.svg")
                                            .w(px(14.))
                                            .h(px(14.))
                                            .text_color(rgb(if hot || menu_open {
                                                palette.accent
                                            } else {
                                                palette.faint
                                            })),
                                    )
                                    .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                                        let next = if *hovered {
                                            Some(index)
                                        } else if view.hovered_delete == Some(index) {
                                            None
                                        } else {
                                            view.hovered_delete
                                        };
                                        if view.hovered_delete != next {
                                            view.hovered_delete = next;
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(15.))
                            .text_color(rgb(if confirming {
                                palette.failed
                            } else if busy {
                                palette.running
                            } else {
                                palette.faint
                            }))
                            .child(meta),
                    )
                    .when_some(hit_snippet, |this, snippet| {
                        this.child(
                            div()
                                .text_xs()
                                .line_height(px(14.))
                                .text_color(rgb(palette.muted))
                                .truncate()
                                .child(snippet),
                        )
                    })
                    .into_any_element(),
            );
        }

        if chat_cards.values().all(|cards| cards.is_empty()) && !needle.is_empty() {
            chat_cards.entry(String::new()).or_default().push(
                div()
                    .mx_2()
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(palette.canvas))
                    .shadow_sm()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(format!(
                        "没有匹配「{}」的对话",
                        self.chat_filter.text.trim()
                    ))
                    .into_any_element(),
            );
        }

        // Workflow cards are filed like conversations: each one belongs to exactly
        // one category (its saved filing, else the default), and it leaves the
        // list when it was removed from the 工作区.
        let mut workflow_cards: std::collections::HashMap<String, Vec<AnyElement>> =
            std::collections::HashMap::new();
        for (index, entry) in self.workflows.iter().enumerate() {
            if self
                .hidden_workflows
                .iter()
                .any(|hidden| hidden == &entry.name)
            {
                continue;
            }
            let selected = self.view == View::Flow && self.selected_flow == Some(index);
            let badge = self.last_run(&entry.name);
            let run_colour = badge.map(|run| run.color(palette));
            let last_run = badge.map(|run| ago(run.finished_ms));
            let menu_open = self
                .card_menu
                .as_ref()
                .is_some_and(|(menu, _)| *menu == CardMenu::Workflow(index));
            let card = div()
                .id(("workflow", index))
                .flex()
                .flex_col()
                .gap_1()
                .mx_2()
                .flex_shrink_0()
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(rgb(palette.canvas))
                .border_1()
                .border_color(rgb(if selected {
                    palette.sky
                } else {
                    palette.canvas
                }))
                .shadow_md()
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.select_flow(index, cx);
                    view.leave_fields(window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(status_dot(palette.accent))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_sm()
                                .text_color(rgb(palette.text))
                                .child(entry.name.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(palette.background))
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(format!("{} 步", entry.spec.steps.len())),
                        )
                        .child(
                            div()
                                .id(("workflow-menu", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .flex_shrink_0()
                                .mr(px(-6.))
                                .size(px(18.))
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgb(palette.background)))
                                .on_click(cx.listener(
                                    move |view, event: &gpui::ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        let same =
                                            view.card_menu.as_ref().is_some_and(|(menu, _)| {
                                                *menu == CardMenu::Workflow(index)
                                            });
                                        view.category_menu = None;
                                        view.confirm_delete = None;
                                        view.card_menu = if same {
                                            None
                                        } else {
                                            Some((CardMenu::Workflow(index), event.position()))
                                        };
                                        cx.notify();
                                    },
                                ))
                                .child(
                                    svg()
                                        .path("icons/ellipsis.svg")
                                        .w(px(14.))
                                        .h(px(14.))
                                        .text_color(rgb(if menu_open {
                                            palette.accent
                                        } else {
                                            palette.faint
                                        })),
                                ),
                        ),
                )
                .when(!entry.description.is_empty(), |this| {
                    this.child(
                        div()
                            .line_clamp(2)
                            .text_xs()
                            .line_height(px(15.))
                            .text_color(rgb(palette.muted))
                            .child(entry.description.clone()),
                    )
                })
                // The manifest path is not shown: the row is about the workflow,
                // and the file on disk is an implementation detail. Only the last
                // run's status keeps a line of its own.
                .when_some(last_run, |this, label| {
                    this.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            // The status dot leads the time, as it always has.
                            .when_some(run_colour, |this, colour| this.child(status_dot(colour)))
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(15.))
                                    .text_color(rgb(palette.faint))
                                    .child(label),
                            ),
                    )
                })
                .into_any_element();
            workflow_cards
                .entry(self.category_of_workflow(&entry.name))
                .or_default()
                .push(card);
        }
        let mut scan_note: Vec<AnyElement> = Vec::new();
        if workflow_cards.values().all(|cards| cards.is_empty()) && self.hidden_workflows.is_empty()
        {
            scan_note.push(
                div()
                    .mx_2()
                    .p_3()
                    .rounded_lg()
                    .bg(rgb(palette.canvas))
                    .shadow_sm()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(format!("{} 下没有 flow.yml", self.scan_root.display()))
                    .into_any_element(),
            );
        }

        // The accordion: one header per category with its own cards underneath.
        // The list is mutually exclusive — `open_category` names the one that is
        // unfolded. Workflow cards sit in the category they are filed under, so
        // they fold away with it.
        let mut rows: Vec<AnyElement> = Vec::new();
        for (order, category) in self.categories.iter().enumerate() {
            let cards = chat_cards.remove(&category.id).unwrap_or_default();
            let flows = workflow_cards.remove(&category.id).unwrap_or_default();
            let open = filtering || self.open_category == category.id;
            if filtering && cards.is_empty() {
                continue;
            }
            let count = cards.len();
            rows.push(self.render_category_header(order, category, open, count, palette, cx));
            if !open {
                continue;
            }
            if cards.is_empty() {
                rows.push(
                    div()
                        .mx_2()
                        .px_3()
                        .pb_1()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child("这个分类还没有对话")
                        .into_any_element(),
                );
            } else {
                rows.extend(cards);
            }
            // Workflow cards follow the conversations of the same category with
            // no extra spacer: the list's own gap is the only spacing, so a
            // chat-to-workflow step reads the same as a chat-to-chat one.
            rows.extend(flows);
        }
        // Whatever was left over — the search note, or a chat filed under a
        // category that is gone — still has to be reachable.
        if let Some(rest) = chat_cards.remove("") {
            rows.extend(rest);
        }
        if self.new_category.is_some() {
            rows.push(self.render_new_category_row(palette, cx));
        }

        // Workflows with no folder of their own to sit in: a search is running (it
        // unfolds every category with a hit, so they are collected here in category
        // order instead of being repeated per category).
        if filtering {
            let mut leftover: Vec<AnyElement> = Vec::new();
            for category in self.categories.iter() {
                if let Some(flows) = workflow_cards.remove(&category.id) {
                    leftover.extend(flows);
                }
            }
            if !leftover.is_empty() {
                rows.extend(leftover);
            }
        }
        rows.extend(scan_note);

        div()
            .flex()
            .flex_col()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(palette.chrome))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .pt_3()
                    .pb_1()
                    .child(
                        div()
                            .px_3()
                            .pb_1()
                            .text_lg()
                            .text_color(rgb(palette.text))
                            .child("对话"),
                    )
                    // The row is a plain list row on the sidebar's own
                    // background: the icon leads, the label follows. The page
                    // links below it use the same shape.
                    .child(
                        div()
                            .id("new-chat")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .mx_2()
                            .h(px(34.))
                            .px_3()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.new_chat(window, cx);
                            }))
                            .child(
                                svg()
                                    .path("icons/new-chat.svg")
                                    .w(px(18.))
                                    .h(px(18.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child("新建对话"),
                            ),
                    )
                    // The page links sit in the same list as the row above: icon
                    // then label, one row style for the whole block.
                    .child(nav_row(
                        "plugins",
                        "icons/plugins.svg",
                        "插件",
                        self.view == View::Plugins,
                        palette,
                        cx,
                        |view, cx| {
                            view.view = View::Plugins;
                            // Plugins come and go outside the window, so the
                            // list is read again each time the page is opened.
                            view.refresh_plugins(cx);
                        },
                    ))
                    .child(nav_row(
                        "mcp",
                        "icons/mcp.svg",
                        "MCP",
                        self.view == View::Mcp,
                        palette,
                        cx,
                        |view, cx| {
                            view.view = View::Mcp;
                            // Servers come and go outside the window, so the
                            // list is read again each time the page is opened.
                            view.refresh_mcp(cx);
                        },
                    ))
                    .child(nav_row(
                        "skills",
                        "icons/skill.svg",
                        "Skill",
                        self.view == View::Skills,
                        palette,
                        cx,
                        |view, cx| {
                            view.view = View::Skills;
                            // Skill folders come and go outside the window, so
                            // the list is read again each time the page opens.
                            view.refresh_skills(cx);
                        },
                    ))
                    // 设置 opens its own window, so its row is never selected; the
                    // 统计 page link follows it, and that closes the list (as asked).
                    .child(nav_row(
                        "settings",
                        "icons/settings.svg",
                        "设置",
                        false,
                        palette,
                        cx,
                        |view, cx| view.open_settings(cx),
                    ))
                    .child(nav_row(
                        "stats",
                        "icons/stats.svg",
                        "统计",
                        self.view == View::Stats,
                        palette,
                        cx,
                        |view, _| {
                            view.view = View::Stats;
                        },
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(palette.text))
                            .child("工作区"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            // The plus starts a new category, which appears at the
                            // end of the list.
                            .child(
                                div()
                                    .id("new-category")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .size(px(20.))
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.start_new_category(window, cx);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/plus.svg")
                                            .w(px(12.))
                                            .h(px(12.))
                                            .text_color(rgb(palette.muted)),
                                    ),
                            ),
                    ),
            )
            // Conversation search sits with the list it filters: under the 工作区
            // title, above the tree, always visible instead of a page link.
            .child(
                div()
                    .pb_1()
                    // The filter is a row until it is clicked; only then does it
                    // draw an outline and take the caret.
                    .child(
                        div()
                            .id("chat-filter-row")
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .mx_2()
                            .h(px(34.))
                            .px_3()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(palette.border))
                            .cursor_text()
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.focused = Some(Field::ChatFilter);
                                if let Some(field) = view.field(Field::ChatFilter) {
                                    window.focus(&field.focus_handle().clone(), cx);
                                }
                                view.start_caret_blink(cx);
                                cx.notify();
                            }))
                            .child(
                                svg()
                                    .path("icons/search.svg")
                                    .w(px(14.))
                                    .h(px(14.))
                                    .text_color(rgb(palette.muted)),
                            )
                            .child(div().flex_1().min_w(px(0.)).child(self.render_field(
                                Field::ChatFilter,
                                "搜索对话",
                                palette,
                                cx,
                            )))
                            .when(!self.chat_filter.text.is_empty(), |this| {
                                this.child(
                                    div()
                                        .id("clear-filter")
                                        .flex_shrink_0()
                                        .px_1()
                                        .rounded_sm()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                        .on_click(cx.listener(|view, _, _, cx| {
                                            view.chat_filter.clear();
                                            cx.notify();
                                        }))
                                        .child("×"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .id("workflows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .gap_2()
                    .pb_2()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .when_some(self.undo_delete.as_ref(), |this, undo| {
                this.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .flex_shrink_0()
                        .px_3()
                        .pb_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child(format!("已删除「{}」", undo.chat.title)),
                        )
                        .child(
                            div()
                                .id("undo-delete")
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .cursor_pointer()
                                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.undo_delete(cx);
                                }))
                                .child("撤销"),
                        ),
                )
            })
    }

    /// The category a conversation is filed under; an empty or stale id falls
    /// back to the default category, so nothing can become unreachable.
    pub(crate) fn category_of(&self, chat: &Chat) -> String {
        if self
            .categories
            .iter()
            .any(|category| category.id == chat.category)
        {
            chat.category.clone()
        } else {
            self.categories
                .first()
                .map(|category| category.id.clone())
                .unwrap_or_default()
        }
    }

    /// One accordion header: chevron, name (or its rename field), card count.
    pub(crate) fn render_category_header(
        &self,
        order: usize,
        category: &Category,
        open: bool,
        count: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let renaming = self
            .rename_category
            .as_ref()
            .is_some_and(|(editing, _)| *editing == category.id);
        let menu_open = self
            .category_menu
            .as_ref()
            .is_some_and(|(open, _)| *open == category.id);
        let name: AnyElement = if renaming {
            div()
                .flex_1()
                .min_w(px(0.))
                .child(self.render_field(Field::CategoryName, "分类名称", palette, cx))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_base()
                .text_color(rgb(palette.strong))
                .child(category.name.clone())
                .into_any_element()
        };
        let toggle = category.id.clone();
        let menu_id = category.id.clone();
        let mut row = div()
            .id(("category", order))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .mx_2()
            .h(px(30.))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.panel_soft)))
            // On click, not on press: folding the list moves everything below it,
            // and pressing would hand the release to whatever slid under the
            // pointer (a workflow card, for one).
            .on_click(cx.listener(move |view, _, _, cx| {
                view.toggle_category(&toggle, cx);
                cx.notify();
            }))
            // A folder, not a chevron: the list is a tree of folders whose leaves
            // are cards, like a file explorer. Open and closed use different
            // drawings so the state reads without an arrow.
            .child(
                svg()
                    .path(if open {
                        "icons/folder-open.svg"
                    } else {
                        "icons/folder.svg"
                    })
                    .w(px(16.))
                    .h(px(16.))
                    .flex_shrink_0()
                    .text_color(rgb(if open { palette.accent } else { palette.muted })),
            )
            .child(name);
        if count > 0 {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(count.to_string()),
            );
        }
        // The row's own menu: rename and delete live here, so a single click on
        // the folder only ever folds it (`on_click` + `stop_propagation`, like the
        // buttons on a card).
        row = row.child(
            div()
                .id(("category-menu", order))
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .size(px(18.))
                .rounded_sm()
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                .on_click(cx.listener(move |view, event: &gpui::ClickEvent, _, cx| {
                    cx.stop_propagation();
                    let same = view
                        .category_menu
                        .as_ref()
                        .is_some_and(|(open, _)| *open == menu_id);
                    view.field_menu = None;
                    view.category_menu = if same {
                        None
                    } else {
                        Some((menu_id.clone(), event.position()))
                    };
                    cx.notify();
                }))
                .child(
                    svg()
                        .path("icons/ellipsis.svg")
                        .w(px(14.))
                        .h(px(14.))
                        .text_color(rgb(if menu_open {
                            palette.accent
                        } else {
                            palette.muted
                        })),
                ),
        );
        row.into_any_element()
    }

    /// The category row's menu: rename it, or delete it.
    ///
    /// gpui has no context menu of its own, so it is the same small floating card
    /// the field menu uses; the caller places it where the "…" was clicked.
    pub(crate) fn render_category_menu(
        &self,
        id: &str,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The tree always keeps one folder: with a single category left, deleting
        // it would leave nowhere for the conversations to live.
        let can_delete = self.categories.len() > 1;
        let item = |element: &'static str,
                    label: &'static str,
                    colour: u32,
                    enabled: bool,
                    action: &'static str,
                    cx: &mut Context<Self>|
         -> AnyElement {
            let mut item = div()
                .id(element)
                .px_3()
                .py(px(1.))
                .rounded_sm()
                .text_xs()
                .text_color(rgb(if enabled { colour } else { palette.faint }));
            if enabled {
                let target = id.to_string();
                item = item
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.category_menu = None;
                        if action == "rename" {
                            view.start_category_rename(&target, window, cx);
                        } else {
                            view.delete_category(&target, cx);
                        }
                        cx.notify();
                    }));
            }
            item.child(label).into_any_element()
        };
        div()
            .id("category-menu-card")
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_1()
            .w(px(112.))
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            // Presses inside the card must not reach the root, which closes the
            // menu on a click anywhere else.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(item(
                "category-rename",
                "重命名",
                palette.text,
                true,
                "rename",
                cx,
            ))
            .child(item(
                "category-delete",
                "删除分类",
                palette.failed,
                can_delete,
                "delete",
                cx,
            ))
            .into_any_element()
    }

    /// The inline row that names a new category; Enter files it, Escape drops it.
    pub(crate) fn render_new_category_row(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("new-category-row")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .mx_2()
            .h(px(28.))
            .flex_shrink_0()
            .px_2()
            .rounded_md()
            .child(
                svg()
                    .path("icons/plus.svg")
                    .w(px(12.))
                    .h(px(12.))
                    .text_color(rgb(palette.faint)),
            )
            .child(div().flex_1().min_w(px(0.)).child(self.render_field(
                Field::NewCategory,
                "分类名称",
                palette,
                cx,
            )))
            .into_any_element()
    }

    /// The category a workflow card is filed under; an id that no longer exists
    /// falls back to the default, so no card can become unreachable.
    pub(crate) fn category_of_workflow(&self, name: &str) -> String {
        workflow_category(&self.workflow_categories, &self.categories, name)
    }

    /// The card's "…" menu: rename it, file it under another category, or
    /// remove it.
    ///
    /// Conversations can also be deleted from here; the first press arms it and
    /// the second one does it, exactly like the old `×` button did.
    pub(crate) fn render_card_menu(
        &self,
        menu: CardMenu,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = match menu {
            CardMenu::Chat(index) => self
                .chats
                .get(index)
                .map(|chat| self.category_of(chat))
                .unwrap_or_default(),
            CardMenu::Workflow(index) => self
                .workflows
                .get(index)
                .map(|entry| self.category_of_workflow(&entry.name))
                .unwrap_or_default(),
        };
        let mut card = div()
            .id("card-menu")
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_1()
            .w(px(144.))
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            // Presses inside the card must not reach the root, which closes it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .px_3()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("移动到分类"),
            );
        // One row per category; the one the card sits in is marked, not repeated.
        for (order, category) in self.categories.iter().enumerate() {
            let here = category.id == current;
            let target = category.id.clone();
            card = card.child(
                div()
                    .id(("card-move", order))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(1.))
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(if here { palette.accent } else { palette.text }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        match menu {
                            CardMenu::Chat(index) => view.move_chat(index, &target, cx),
                            CardMenu::Workflow(index) => {
                                if let Some(entry) = view.workflows.get(index) {
                                    let name = entry.name.clone();
                                    view.move_workflow(&name, &target, cx);
                                }
                            }
                        }
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(category.name.clone()),
                    )
                    .when(here, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child("✓"),
                        )
                    }),
            );
        }
        card = card.child(div().h(px(2.)).flex_shrink_0());
        // A pinned card stays at the top of its category; the item says which
        // way it will go, so the list never moves by surprise.
        if let CardMenu::Chat(index) = menu {
            let pinned = self.chats.get(index).is_some_and(|chat| chat.pinned);
            card = card.child(
                div()
                    .id("card-pin")
                    .px_3()
                    .py(px(1.))
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.toggle_pin(index, cx);
                    }))
                    .child(if pinned { "取消置顶" } else { "置顶" }),
            );
        }
        // Renaming sits with the other actions, below the category list: the
        // pencil that used to do it was one more target on every card.
        if let CardMenu::Chat(index) = menu {
            card = card.child(
                div()
                    .id("card-instructions")
                    .px_3()
                    .py(px(1.))
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.card_menu = None;
                        view.select_chat(index);
                        view.settings_tab = SettingsTab::Instructions;
                        view.load_instructions(cx);
                        view.open_settings(cx);
                    }))
                    .child("编辑指令"),
            );
        }
        if let CardMenu::Chat(index) = menu {
            card = card.child(
                div()
                    .id("card-rename")
                    .px_3()
                    .py(px(1.))
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.card_menu = None;
                        // A rename in progress on another card keeps what was
                        // typed: opening this menu does not select that card.
                        view.commit_rename();
                        view.start_rename(index, window, cx);
                    }))
                    .child("重命名"),
            );
        }
        card = card.child(match menu {
            CardMenu::Chat(index) => {
                let armed = self.confirm_delete == Some(index);
                div()
                    .id("card-delete")
                    .px_3()
                    .py(px(1.))
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.failed))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        // One conversation is one press away from gone, so the
                        // first press only arms the item.
                        if view.confirm_delete == Some(index) {
                            view.confirm_delete = None;
                            view.card_menu = None;
                            view.delete_chat(index, cx);
                        } else {
                            view.confirm_delete = Some(index);
                            cx.notify();
                        }
                    }))
                    .child(if armed {
                        "再点一次确认删除"
                    } else {
                        "删除对话"
                    })
            }
            CardMenu::Workflow(index) => div()
                .id("card-remove")
                .px_3()
                .py(px(1.))
                .rounded_sm()
                .text_xs()
                .text_color(rgb(palette.failed))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(entry) = view.workflows.get(index) {
                        let name = entry.name.clone();
                        view.remove_workflow(&name, cx);
                    }
                }))
                // The manifest stays on disk: the card leaves the list, the folder
                // is not ours to delete.
                .child("从工作区移除"),
        });
        card.into_any_element()
    }

    /// The rename dialog: a centred card over the transcript.
    ///
    /// Renaming used to happen in the card's title slot; a dialog keeps the title
    /// readable while editing and gives save / cancel an obvious place to live.
    pub(crate) fn render_rename_dialog(
        &self,
        viewport_height: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id("rename-sheet")
            .absolute()
            .top(px(TITLEBAR_HEIGHT))
            .left(px(0.))
            .w_full()
            .h(viewport_height - px(TITLEBAR_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            // A press outside the card is a cancel, and nothing behind the sheet
            // may see it: the cards would select or open a menu otherwise.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    cx.stop_propagation();
                    view.finish_rename(false, window, cx);
                }),
            )
            .child(
                div()
                    .id("rename-dialog")
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(px(320.))
                    .p_3()
                    .rounded_md()
                    .bg(rgb(palette.panel))
                    .border_1()
                    .border_color(rgb(palette.border))
                    .shadow_md()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.strong))
                            .child("重命名对话"),
                    )
                    .child(self.render_field(Field::ChatTitle, "对话名称", palette, cx))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id("rename-cancel")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(palette.panel_soft))
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.border)))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.finish_rename(false, window, cx);
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("rename-save")
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(palette.accent))
                                    .text_sm()
                                    .text_color(rgb(palette.accent_text))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.sky)))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.finish_rename(true, window, cx);
                                    }))
                                    .child("保存"),
                            ),
                    ),
            )
            .into_any_element()
    }

    /// The newest run of a workflow: this session's records win over the saved
    /// ones, and between two runs the later finish wins.
    pub(crate) fn last_run(&self, name: &str) -> Option<RunBadge> {
        let this_session = self
            .history
            .iter()
            .find(|record| record.workflow_name == name)
            .map(|record| RunBadge {
                ok: record.outcome.ok,
                cancelled: record.cancelled,
                finished_ms: epoch_ms(record.finished),
            });
        let saved = self
            .past_runs
            .iter()
            .find(|run| run.workflow == name)
            .map(|run| RunBadge {
                ok: run.ok,
                cancelled: run.cancelled,
                finished_ms: run.finished_ms,
            });
        match (this_session, saved) {
            (Some(fresh), Some(stored)) => Some(if fresh.finished_ms >= stored.finished_ms {
                fresh
            } else {
                stored
            }),
            (fresh, stored) => fresh.or(stored),
        }
    }
}

/// What a workflow card needs to know about a finished run.
#[derive(Clone, Copy)]
pub(crate) struct RunBadge {
    /// Finished without error.
    pub(crate) ok: bool,
    /// Stopped by the user.
    pub(crate) cancelled: bool,
    /// When it finished, in milliseconds since the Unix epoch.
    pub(crate) finished_ms: u64,
}

impl RunBadge {
    /// Colour of the card's status dot.
    pub(crate) fn color(self, palette: Palette) -> u32 {
        if self.ok && !self.cancelled {
            palette.ok
        } else if self.cancelled {
            palette.warn
        } else {
            palette.failed
        }
    }
}

/// One sidebar navigation row: icon then label.
///
/// 新建对话 and the 插件 / MCP / Skill / 设置 / 统计 links share this shape, so
/// the block above 工作区 reads as one list: 34px rows, 18px icon, flat until
/// hovered. The conversation search lives under the 工作区 title instead.
fn nav_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    action: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .mx_2()
        .h(px(34.))
        .px_3()
        .rounded_md()
        .when(selected, |this| this.bg(rgb(palette.panel_soft)))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| {
            action(view, cx);
            cx.notify();
        }))
        .child(
            svg()
                .path(icon)
                .w(px(18.))
                .h(px(18.))
                .text_color(rgb(if selected {
                    palette.accent
                } else {
                    palette.muted
                })),
        )
        .child(div().text_sm().text_color(rgb(palette.text)).child(label))
}

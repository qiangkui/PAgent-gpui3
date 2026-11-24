//! The composer at the bottom of a conversation and its model menu.

use super::*;

impl DshFlowApp {
    /// The floating message box.
    ///
    /// Like dsh's composer: the card is the surface, the field inside is
    /// chromeless, and the send control is a circle that sits at the end.
    /// The model picker, drawn above the composer's chip.
    /// The composer's model picker: provider tabs on top, their versions below.
    ///
    /// Everything is on one surface — picking a provider only switches the list
    /// underneath, so choosing a model never takes a second step.
    pub(crate) fn render_model_menu(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let providers: Vec<(usize, &'static str)> = self
            .ai
            .verified()
            .iter()
            .map(|(index, connection)| (*index, connection.provider_label()))
            .collect();
        // The tab follows the model in use, falling back to the first provider.
        let tab = if providers.iter().any(|(index, _)| *index == self.model_tab) {
            self.model_tab
        } else {
            providers.first().map_or(0, |(index, _)| *index)
        };
        let active = self.ai.active;

        let mut tabs: Vec<AnyElement> = Vec::new();
        for (index, label) in providers {
            let selected = index == tab;
            tabs.push(
                div()
                    .id(("menu-tab", index))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(if selected {
                        palette.panel_soft
                    } else {
                        palette.background
                    }))
                    .text_xs()
                    .text_color(rgb(if selected {
                        palette.accent
                    } else {
                        palette.muted
                    }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.model_tab = index;
                        cx.notify();
                    }))
                    .child(label)
                    .into_any_element(),
            );
        }

        let connection = self.ai.connections.get(tab);
        let current = connection
            .filter(|_| tab == active)
            .map(|connection| connection.model.clone());
        let mut names: Vec<String> = connection
            .map(|connection| {
                if connection.models.is_empty() {
                    model_versions(&connection.provider)
                        .iter()
                        .map(|model| (*model).to_string())
                        .collect()
                } else {
                    connection.models.clone()
                }
            })
            .unwrap_or_default();
        if let Some(current) = current.clone()
            && !names.contains(&current)
        {
            names.push(current);
        }

        let mut versions: Vec<AnyElement> = Vec::new();
        for (position, name) in names.iter().enumerate() {
            let selected = current.as_deref() == Some(name.as_str());
            let choice = name.clone();
            versions.push(
                div()
                    .id(("menu-version", tab * 64 + position))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |this| this.bg(rgb(palette.panel_soft)))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.choose_composer_model(tab, &choice, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(if selected {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(name.clone()),
                    )
                    .when(selected, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child("使用中"),
                        )
                    })
                    .into_any_element(),
            );
        }
        if versions.is_empty() {
            versions.push(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("这个接入点还没有可用版本")
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .mb_1()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(div().flex().flex_row().flex_wrap().gap_1().children(tabs))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("版本"),
                    )
                    .children(versions),
            )
    }

    /// The attach menu: files to add, then the context command.
    ///
    /// The shell matches the model picker (a floating card, one row per
    /// choice). Nothing here keeps the menu open: choosing a file opens the
    /// native dialog, and 压缩上下文 runs at once.
    pub(crate) fn render_attach_menu(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let heading = |text: &'static str, first: bool| {
            div()
                .px_2()
                .pt(if first { px(0.) } else { px(6.) })
                .pb(px(2.))
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_2()
            .mb_1()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(heading("文件", true))
            .child(attach_file_row(
                "attach-text",
                "icons/file-text.svg",
                "文本",
                AttachmentKind::Text,
                palette,
                cx,
            ))
            .child(attach_file_row(
                "attach-image",
                "icons/image.svg",
                "图片",
                AttachmentKind::Image,
                palette,
                cx,
            ))
            .child(attach_file_row(
                "attach-pdf",
                "icons/pdf.svg",
                "PDF",
                AttachmentKind::Pdf,
                palette,
                cx,
            ))
            .child(heading("指令", false))
            .child(
                div()
                    .id("attach-compact")
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(|view, _, _, cx| view.compress_context(cx)))
                    .child(
                        svg()
                            .path("icons/compress.svg")
                            .w(px(14.))
                            .h(px(14.))
                            .flex_shrink_0()
                            .text_color(rgb(palette.muted)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child("压缩上下文"),
                    ),
            )
    }

    /// The tool-permission menu, opened by the chip beside the model.
    ///
    /// The three rows are dsh's presets, glyph for glyph: 仅可查看 /
    /// 工作区内修改 / 完全权限. Each one bundles how far the tools reach with
    /// whether they stop to ask, so "stop asking" is chosen once.
    pub(crate) fn render_tools_menu(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let current = ToolMode::of(self.chat());
        let mut items: Vec<AnyElement> = Vec::new();
        for mode in ToolMode::ALL {
            let selected = current == mode;
            let label_color = if selected {
                palette.accent
            } else {
                palette.text
            };
            items.push(
                div()
                    .id(("tool-mode", mode as usize))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |this| this.bg(rgb(palette.panel_soft)))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_tool_mode(mode, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                svg()
                                    .path(mode.icon())
                                    .w(px(16.))
                                    .h(px(16.))
                                    .flex_shrink_0()
                                    .text_color(rgb(if selected {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(rgb(label_color))
                                    .child(mode.label()),
                            )
                            .when(selected, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.accent))
                                        .child("使用中"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .pl(px(24.))
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(mode.hint()),
                    )
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .mb_1()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .children(items)
    }

    /// The tool-budget menu: round-trips and calls for this conversation.
    pub(crate) fn render_budget_menu(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let (rounds, calls) = (self.chat().max_rounds, self.chat().max_calls);
        let mut round_items: Vec<AnyElement> = Vec::new();
        for value in [6u32, 12, 24] {
            let selected = rounds == value;
            round_items.push(
                div()
                    .id(("max-rounds", value as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |this| this.bg(rgb(palette.panel_soft)))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_max_rounds(value, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(if selected {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(format!("{value} 轮")),
                    )
                    .when(selected, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child("使用中"),
                        )
                    })
                    .into_any_element(),
            );
        }
        let mut call_items: Vec<AnyElement> = Vec::new();
        for value in [20u32, 40, 80] {
            let selected = calls == value;
            call_items.push(
                div()
                    .id(("max-calls", value as usize))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(selected, |this| this.bg(rgb(palette.panel_soft)))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.set_max_calls(value, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(rgb(if selected {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(format!("{value} 次")),
                    )
                    .when(selected, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child("使用中"),
                        )
                    })
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .mb_1()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("工具轮次"),
            )
            .children(round_items)
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("工具调用"),
            )
            .children(call_items)
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("达到上限时先让模型总结；shell、移动、删除仍逐次确认。"),
            )
    }

    /// The files waiting to be sent, drawn above the model and tool chips.
    ///
    /// They live in the card's column on purpose: unlike the menus, a chip row
    /// that only exists while files are attached is allowed to make the card
    /// taller. The name carries the meaning, so the chip has no fill until the
    /// pointer is on it.
    pub(crate) fn render_attachments(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut chips: Vec<AnyElement> = Vec::new();
        for (index, attachment) in self.attachments.iter().enumerate() {
            let name = attachment
                .name
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(&attachment.name)
                .to_string();
            chips.push(
                div()
                    .id(("attachment-chip", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .max_w(px(260.))
                    .px_2()
                    .py_1()
                    .rounded_full()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .when(attachment.kind != AttachmentKind::Text, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .px_1()
                                .rounded_sm()
                                .bg(rgb(palette.panel_soft))
                                .text_xs()
                                .text_color(rgb(palette.accent))
                                .child(attachment.kind.label()),
                        )
                    })
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.sky))
                            .child(name),
                    )
                    .when(attachment.truncated, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .child("已截断"),
                        )
                    })
                    .child(
                        div()
                            .id(("attachment-remove", index))
                            .flex_shrink_0()
                            .rounded_sm()
                            .px_1()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .cursor_pointer()
                            .hover(move |style| {
                                style.bg(rgb(palette.panel)).text_color(rgb(palette.text))
                            })
                            .on_click(
                                cx.listener(move |view, _, _, cx| {
                                    view.remove_attachment(index, cx)
                                }),
                            )
                            .child("×"),
                    )
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_2()
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("附件"),
            )
            .children(chips)
    }

    pub(crate) fn render_composer(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let waiting = self.chat_busy(self.active_chat);
        // Nothing typed and nothing running: the send control reads as dormant
        // instead of inviting a click that would do nothing.
        let ready = waiting || !self.composer.text.trim().is_empty();
        // The sheet has to fill the pane: an absolutely positioned element with
        // only insets and an automatic size collapses to nothing in this
        // layout engine, so the stack is pinned with `justify_end` instead.
        // The stack is the card and the capsule under it, both centred on the
        // same axis and sharing one width.
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .px(px(24.))
            .pb(px(24.))
            .child(
                // Menus first: they are painted before the card, so their
                // downward shadow falls under the card surface instead of
                // across the input panel, while the menu itself (above the
                // card) stays fully visible.
                div()
                    .relative()
                    .w_full()
                    .max_w(px(COMPOSER_MAX_WIDTH))
                    .min_w(px(0.))
                    .when(self.tools_menu, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(relative(1.))
                                .child(self.render_tools_menu(palette, cx)),
                        )
                    })
                    .when(self.budget_menu, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(relative(1.))
                                .child(self.render_budget_menu(palette, cx)),
                        )
                    })
                    .when(self.model_menu, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(relative(1.))
                                .child(self.render_model_menu(palette, cx)),
                        )
                    })
                    .when(self.attach_menu, |this| {
                        this.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .bottom(relative(1.))
                                .child(self.render_attach_menu(palette, cx)),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w(px(0.))
                            .gap_1()
                            .p_2()
                            .pt_1()
                            .rounded_xl()
                            .bg(rgb(palette.panel))
                            .border_1()
                            .border_color(rgb(palette.border))
                            .shadow_lg()
                            .on_drop(cx.listener(|view, paths: &ExternalPaths, _window, cx| {
                                view.attach_files(paths.paths(), cx);
                            }))
                            .when(!self.attachments.is_empty(), |this| {
                                this.child(self.render_attachments(palette, cx))
                            })
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .px_2()
                                    .pt_1()
                                    // Add files or run a context command: the round
                                    // "add" control sits before the model chip, on
                                    // the row's leading edge.
                                    .child(
                                        div()
                                            .id("attach-chip")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .flex_shrink_0()
                                            .size(px(26.))
                                            .rounded_full()
                                            .bg(rgb(if self.attach_menu {
                                                palette.panel_soft
                                            } else {
                                                mix(palette.panel, palette.panel_soft, 0.5)
                                            }))
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                            )
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.model_menu = false;
                                                view.tools_menu = false;
                                                view.budget_menu = false;
                                                view.attach_menu = !view.attach_menu;
                                                cx.notify();
                                            }))
                                            .child(
                                                svg()
                                                    .path("icons/plus-thin.svg")
                                                    .w(px(18.))
                                                    .h(px(18.))
                                                    .flex_shrink_0()
                                                    .text_color(rgb(palette.muted)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("model-chip")
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .min_w(px(0.))
                                            .max_w(px(200.))
                                            .px_2()
                                            .py_1()
                                            .rounded_full()
                                            .bg(rgb(palette.panel))
                                            .when(!self.ai.usable(), |this| {
                                                this.border_1().border_color(rgb(palette.failed))
                                            })
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .cursor_pointer()
                                            // Keep this press away from the root, which
                                            // closes menus: otherwise mouse-down closes
                                            // it and mouse-up toggles it straight back.
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                            )
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.attach_menu = false;
                                                if view.ai.verified().is_empty() {
                                                    // Nothing has answered a test yet, so the
                                                    // picker would be empty: open the settings
                                                    // window, where a model is connected.
                                                    view.open_settings(cx);
                                                    view.model_menu = false;
                                                    view.tools_menu = false;
                                                    view.budget_menu = false;
                                                } else {
                                                    if !view.model_menu {
                                                        // Open on the provider in use.
                                                        view.model_tab = view.ai.active;
                                                    }
                                                    view.tools_menu = false;
                                                    view.budget_menu = false;
                                                    view.model_menu = !view.model_menu;
                                                }
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .min_w(px(0.))
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(rgb(if self.ai.usable() {
                                                        palette.text
                                                    } else {
                                                        palette.failed
                                                    }))
                                                    .child(match self.ai.active() {
                                                        Some(connection) if connection.verified => {
                                                            connection.model.clone()
                                                        }
                                                        _ => "未接入模型".to_string(),
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(palette.muted))
                                                    .child("▾"),
                                            ),
                                    )
                                    .when(!self.ai.usable(), |this| {
                                        this.child(
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .truncate()
                                                .text_xs()
                                                .text_color(rgb(palette.failed))
                                                .child("在「设置 → AI」里测试通过后即可使用"),
                                        )
                                    })
                                    // Tools change what the assistant may do to the
                                    // conversation folder, so the state is always visible.
                                    .child(
                                        div()
                                            .id("tools-chip")
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded_full()
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                            )
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.model_menu = false;
                                                view.budget_menu = false;
                                                view.attach_menu = false;
                                                view.tools_menu = !view.tools_menu;
                                                cx.notify();
                                            }))
                                            // dsh draws the current preset's glyph in the
                                            // trigger as well, tinted like the label.
                                            .child(
                                                svg()
                                                    .path(ToolMode::of(self.chat()).icon())
                                                    .w(px(14.))
                                                    .h(px(14.))
                                                    .flex_shrink_0()
                                                    .text_color(rgb(palette.muted)),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(palette.text))
                                                    .child(ToolMode::of(self.chat()).label()),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(palette.muted))
                                                    .child("▾"),
                                            ),
                                    )
                                    // Rounds and calls share one chip: they are one
                                    // budget and the row is already busy.
                                    .child(
                                        div()
                                            .id("budget-chip")
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .gap_1()
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded_full()
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_mouse_down(
                                                MouseButton::Left,
                                                cx.listener(|_, _, _, cx| cx.stop_propagation()),
                                            )
                                            .on_click(cx.listener(|view, _, _, cx| {
                                                view.model_menu = false;
                                                view.tools_menu = false;
                                                view.attach_menu = false;
                                                view.budget_menu = !view.budget_menu;
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(if self.chat().tools {
                                                        palette.muted
                                                    } else {
                                                        palette.faint
                                                    }))
                                                    .child(format!(
                                                        "预算 {} 轮 / {} 次",
                                                        self.chat().max_rounds,
                                                        self.chat().max_calls
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(palette.muted))
                                                    .child("▾"),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_end()
                                    .gap_2()
                                    .child(div().flex_1().min_w(px(0.)).child(self.render_field(
                                        Field::Composer,
                                        "给助手发消息 · Enter 发送 · Shift+Enter 换行",
                                        palette,
                                        cx,
                                    )))
                                    .child(
                                        div()
                                            .id("send")
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .flex_shrink_0()
                                            .size(px(32.))
                                            .rounded_full()
                                            // While an answer is being written the button
                                            // turns into a stop control: same blue, a square
                                            // in the middle instead of a word. With nothing
                                            // to send it fades into the card.
                                            .bg(rgb(if ready {
                                                palette.accent
                                            } else {
                                                palette.panel_soft
                                            }))
                                            .cursor_pointer()
                                            .on_click(cx.listener(|view, _, window, cx| {
                                                if view.chat_busy(view.active_chat) {
                                                    view.cancel_answer(cx);
                                                } else {
                                                    view.send_message(cx);
                                                }
                                                // Keep the caret in the composer so the
                                                // next question can be typed straight away.
                                                view.focused = Some(Field::Composer);
                                                window.focus(&view.composer.focus_handle().clone(), cx);
                                                cx.notify();
                                            }))
                                            .child(if waiting {
                                                div()
                                                    .size(px(10.))
                                                    .rounded_sm()
                                                    .bg(rgb(palette.accent_text))
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        svg()
                                                            .path("icons/submit.svg")
                                                            .w(px(20.))
                                                            .h(px(20.))
                                                            .text_color(rgb(if ready {
                                                                palette.accent_text
                                                            } else {
                                                                palette.muted
                                                            })),
                                                    )
                                                    .into_any_element()
                                            }),
                                    ),
                            ),
                    ),
            )
            // The status capsule rides under the card, on the same axis and the
            // same width, so the two read as one floating stack. It takes the
            // card's own shadow so the pair sits on one plane.
            .child(self.render_status_capsule(palette))
    }
}

/// Mix two packed RGB colours; `amount` is how much of `to` to use (0–1).
///
/// The add button wants a tone lighter than `panel_soft` but still visible on
/// the card, and the palette has no token between the two.
fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let amount = amount.clamp(0., 1.);
    let channel = |shift: u32| {
        let from = ((from >> shift) & 0xff) as f32;
        let to = ((to >> shift) & 0xff) as f32;
        ((from + (to - from) * amount).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// One file kind in the attach menu: its icon, name, and the picker it opens.
fn attach_file_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    kind: AttachmentKind,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| view.open_attach_picker(kind, cx)))
        .child(
            svg()
                .path(icon)
                .w(px(14.))
                .h(px(14.))
                .flex_shrink_0()
                .text_color(rgb(palette.muted)),
        )
        .child(div().text_xs().text_color(rgb(palette.text)).child(label))
        .into_any_element()
}

/// How many lines of a draft are drawn before the rest is clipped.
pub(crate) const MAX_COMPOSER_LINES: usize = 8;

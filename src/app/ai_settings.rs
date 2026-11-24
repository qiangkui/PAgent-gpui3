//! The AI page: connection cards, model lists, and key testing.

use super::*;

impl DshFlowApp {
    /// The AI block of the settings page: who answers, and with which key.
    pub(crate) fn render_ai_settings(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let usable = self.ai.usable();
        let status_color = if usable { palette.ok } else { palette.failed };
        let status_text = if usable {
            format!(
                "使用中：{}",
                self.ai
                    .active()
                    .map(|connection| connection.model.clone())
                    .unwrap_or_default()
            )
        } else {
            "还没有可用的模型：填好 Key 后点「测试」".to_string()
        };

        let mut cards: Vec<AnyElement> = Vec::new();
        for (index, connection) in self.ai.connections.iter().enumerate() {
            cards.push(
                self.render_connection_card(index, connection, palette, cx)
                    .into_any_element(),
            );
        }
        if cards.is_empty() {
            cards.push(
                div()
                    .py_3()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("还没有接入任何模型，从下面的按钮添加一个。")
                    .into_any_element(),
            );
        }

        let mut presets: Vec<AnyElement> = Vec::new();
        for preset in PRESETS {
            presets.push(
                div()
                    .id(preset.id)
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.add_connection(preset.id, cx);
                    }))
                    .child(format!("＋ {}", preset.label))
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings_page_header(
                "AI",
                "选择回答问题的模型，并设置用量与网络。",
                palette,
            ))
            .child(
                settings_section("接入点", palette)
                    .child(settings_note(
                        "每个接入点都是 OpenAI 兼容接口，通过 dsh-flow 的 ai 桥调用；「测试」会真发一句 \
                         ping，通过后才会出现在对话上方的模型列表里。Key 存在 Windows 凭据管理器（本机），\
                         配置写在程序目录的 config.json，都不进入仓库。",
                        palette,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(status_dot(status_color))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(status_color))
                                    .child(status_text),
                            ),
                    )
                    .child(div().flex().flex_col().gap_2().children(cards))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .pt_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap_2()
                                    .children(presets),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .flex_shrink_0()
                                    .when_some(self.ai_status.clone(), |this, (text, error)| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(if error {
                                                    palette.failed
                                                } else {
                                                    palette.ok
                                                }))
                                                .child(text),
                                        )
                                    }),
                            ),
                    ),
            )
            .child(
                settings_section("用量与代理", palette)
                    .child(self.render_budget_row(palette, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.text))
                                    .child("HTTP 代理"),
                            )
                            .child(
                                div().w(px(340.)).child(self.render_field(
                                    Field::Proxy,
                                    "http://127.0.0.1:7890（留空为直连）",
                                    palette,
                                    cx,
                                )),
                            )
                            .child(settings_note(
                                "模型请求会走这个代理，本地地址始终直连；下一个引擎进程开始时生效。",
                                palette,
                            )),
                    ),
            )
    }

    /// 用量上限: the budgets people actually pick, so no number box is needed.
    ///
    /// The figure is only ever compared with what a conversation has spent, so a
    /// free-typed value would add validation without adding a choice.
    fn render_budget_row(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let current = self.ai.budget_tokens;
        let mut chips: Vec<AnyElement> = Vec::new();
        for budget in [0u64, 50_000, 200_000, 500_000] {
            let selected = current == budget;
            let label = if budget == 0 {
                "不限制".to_string()
            } else {
                format!("{} tok", token_amount(budget))
            };
            chips.push(
                div()
                    .id(("budget", budget as usize))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if selected {
                        palette.accent
                    } else {
                        palette.border
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
                        view.set_budget(budget, cx);
                    }))
                    .child(label)
                    .into_any_element(),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .child("每次对话用量上限"),
            )
            .child(div().flex().flex_row().flex_wrap().gap_2().children(chips))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child("超过上限后状态条变黄提醒；只是提示，不会中断提问。"),
            )
    }

    /// One connection: who it talks to, its three fields, and a test button.
    pub(crate) fn render_connection_card(
        &self,
        index: usize,
        connection: &Connection,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let testing = self
            .testing
            .as_ref()
            .is_some_and(|test| test.index == index);
        let (badge, badge_color) = if testing {
            ("测试中", palette.running)
        } else if connection.verified {
            ("已接入", palette.ok)
        } else {
            ("未测试", palette.faint)
        };
        let selected = index == self.ai.active && connection.verified;

        let status = self
            .conn_status
            .get(index)
            .and_then(|slot| slot.clone())
            .map(|(text, error)| {
                div()
                    .text_xs()
                    .text_color(rgb(if error { palette.failed } else { palette.ok }))
                    .whitespace_normal()
                    .child(text)
            });

        // Versions on offer: whatever the endpoint last reported (authoritative),
        // otherwise the preset list. The model typed in is always shown too, so
        // the current choice is visible even when the endpoint is not listed.
        let typed = self.conn_fields.get(index).map_or_else(
            || connection.model.clone(),
            |fields| fields.model.text.clone(),
        );
        let mut names: Vec<String> = if connection.models.is_empty() {
            model_versions(&connection.provider)
                .iter()
                .map(|model| (*model).to_string())
                .collect()
        } else {
            connection.models.clone()
        };
        let typed = typed.trim().to_string();
        if !typed.is_empty() && !names.contains(&typed) {
            names.push(typed.clone());
        }
        let listing = self
            .listing
            .as_ref()
            .is_some_and(|state| state.index == index);
        let mut versions: Vec<AnyElement> = Vec::new();
        for (position, name) in names.iter().enumerate() {
            let current = *name == typed;
            let choice = name.clone();
            versions.push(
                div()
                    .id(("conn-model", index * 64 + position))
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(if current {
                        palette.panel_soft
                    } else {
                        palette.background
                    }))
                    .text_xs()
                    .text_color(rgb(if current {
                        palette.accent
                    } else {
                        palette.muted
                    }))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.choose_model(index, &choice, cx);
                    }))
                    .child(name.clone())
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(if selected {
                palette.accent
            } else {
                palette.border
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child(connection.provider_label()),
                            )
                            .child(status_chip(badge, badge_color, palette))
                            .when(selected, |this| {
                                this.child(status_chip("使用中", palette.accent, palette))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .id(("test", index))
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(if testing {
                                        palette.panel_soft
                                    } else {
                                        palette.accent
                                    }))
                                    .text_xs()
                                    .text_color(rgb(if testing {
                                        palette.muted
                                    } else {
                                        palette.accent_text
                                    }))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if view.testing.is_none() {
                                            view.test_connection(index, cx);
                                        }
                                    }))
                                    .child(if testing { "测试中…" } else { "测试" }),
                            )
                            .child(
                                div()
                                    .id(("remove", index))
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.remove_connection(index, cx);
                                    }))
                                    .child("删除"),
                            ),
                    ),
            )
            .child(
                self.settings_field_row(
                    "模型",
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(self.render_field(
                            Field::ConnModel(index),
                            "模型 id，例如 deepseek-chat",
                            palette,
                            cx,
                        ))
                        .when(!versions.is_empty() || listing, |this| {
                            this.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_start()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_1()
                                            .flex_1()
                                            .min_w(px(0.))
                                            .children(versions),
                                    )
                                    .child(
                                        div()
                                            .id(("list-models", index))
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .text_xs()
                                            .text_color(rgb(if listing {
                                                palette.faint
                                            } else {
                                                palette.accent
                                            }))
                                            .cursor_pointer()
                                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                if view.listing.is_none() {
                                                    view.list_models(index, cx);
                                                }
                                            }))
                                            .child(if listing {
                                                "读取中…"
                                            } else {
                                                "读取可用模型"
                                            }),
                                    ),
                            )
                        }),
                    palette,
                ),
            )
            .child(self.settings_field_row(
                "API Key",
                self.render_field(Field::ConnKey(index), "sk-…", palette, cx),
                palette,
            ))
            .child(self.settings_field_row(
                "Base URL",
                self.render_field(Field::ConnUrl(index), "https://…/v1", palette, cx),
                palette,
            ))
            .child(
                self.settings_field_row(
                    "温度",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(settings_choice(
                            5,
                            "默认",
                            connection.sampling == Sampling::Default,
                            move |view, cx| view.set_sampling(index, Sampling::Default, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            6,
                            "精确",
                            connection.sampling == Sampling::Precise,
                            move |view, cx| view.set_sampling(index, Sampling::Precise, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            7,
                            "平衡",
                            connection.sampling == Sampling::Balanced,
                            move |view, cx| view.set_sampling(index, Sampling::Balanced, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            8,
                            "发散",
                            connection.sampling == Sampling::Loose,
                            move |view, cx| view.set_sampling(index, Sampling::Loose, cx),
                            palette,
                            cx,
                        ))
                        .child(div().text_xs().text_color(rgb(palette.faint)).child(
                            match connection.sampling.temperature() {
                                None => "不发送温度，用端点自己的默认值".to_string(),
                                Some(value) => format!("temperature {value}"),
                            },
                        )),
                    palette,
                ),
            )
            .child(
                self.settings_field_row(
                    "最大输出",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(settings_choice(
                            9,
                            "默认",
                            connection.max_tokens == 0,
                            move |view, cx| view.set_max_tokens(index, 0, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            10,
                            "2k",
                            connection.max_tokens == 2048,
                            move |view, cx| view.set_max_tokens(index, 2048, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            11,
                            "4k",
                            connection.max_tokens == 4096,
                            move |view, cx| view.set_max_tokens(index, 4096, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            12,
                            "8k",
                            connection.max_tokens == 8192,
                            move |view, cx| view.set_max_tokens(index, 8192, cx),
                            palette,
                            cx,
                        ))
                        .child(settings_choice(
                            13,
                            "16k",
                            connection.max_tokens == 16384,
                            move |view, cx| view.set_max_tokens(index, 16384, cx),
                            palette,
                            cx,
                        ))
                        .child(div().text_xs().text_color(rgb(palette.faint)).child(
                            if connection.max_tokens == 0 {
                                "不发送 max_tokens，用端点默认".to_string()
                            } else {
                                format!("max_tokens {}", connection.max_tokens)
                            },
                        )),
                    palette,
                ),
            )
            .when_some(status, |this, status| this.child(status))
    }

    /// One label/value row of a settings block.
    pub(crate) fn settings_field_row(
        &self,
        label: &'static str,
        value: impl IntoElement,
        palette: Palette,
    ) -> Div {
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap_4()
            .py_2()
            .border_b_1()
            .border_color(rgb(palette.border))
            .child(
                div()
                    .w(px(96.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(label),
            )
            .child(div().flex_1().min_w(px(0.)).child(value))
    }
}

/// A selectable chip in settings, used for the tool modes and the temperature.
fn settings_choice(
    mode: usize,
    label: &'static str,
    selected: bool,
    on_click: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> impl IntoElement {
    div()
        .id(("tool-mode-settings", mode))
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
        .on_click(cx.listener(move |view, _, _, cx| on_click(view, cx)))
        .child(label)
}

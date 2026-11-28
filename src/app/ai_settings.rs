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
                settings_section_help(
                    "接入点",
                    "每个接入点都是 OpenAI 兼容接口，通过 dsh-flow 的 ai 桥调用；「测试」会真发一句 ping，通过后才会出现在对话上方的模型列表里。Key 存在 Windows 凭据管理器（本机），配置写在程序目录的 config.json，都不进入仓库。",
                    palette,
                )
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
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.text))
                                            .child("HTTP 代理"),
                                    )
                                    .child(help_icon(
                                        "help-proxy",
                                        "模型请求会走这个代理，本地地址始终直连；下一个引擎进程开始时生效。留空表示直连。",
                                        palette,
                                    )),
                            )
                            .child(
                                div().w(px(340.)).child(self.render_field(
                                    Field::Proxy,
                                    "http://127.0.0.1:7890（留空为直连）",
                                    palette,
                                    cx,
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.text))
                                            .child("自动审查允许的命令"),
                                    )
                                    .child(help_icon(
                                        "help-review-allow",
                                        "自动审查下，这些命令不必逐次确认。按前缀匹配整行（「npm test」也允许「npm test --watch」），含 && / | / > 等串联的仍然会问。只填你信任其全部参数的命令，例如「npm test, pytest」；留空只用内置规则。",
                                        palette,
                                    )),
                            )
                            .child(
                                div().w(px(340.)).child(self.render_field(
                                    Field::ReviewAllow,
                                    "npm test, npm run build（留空只用内置规则）",
                                    palette,
                                    cx,
                                )),
                            ),
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
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.text))
                            .child("每次对话用量上限"),
                    )
                    .child(help_icon(
                        "help-budget",
                        "达到上限后状态条变黄提醒；只是提示，不会中断提问，也不会限制请求。统计按整段对话累计，改档位立即生效。",
                        palette,
                    )),
            )
            .child(div().flex().flex_row().flex_wrap().gap_2().children(chips))
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
        let mut names: Vec<String> = offered_models(
            &connection.provider,
            &connection.base_url,
            &connection.models,
        );
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
                            .child(
                                div()
                                    .id(("conn-vision", index))
                                    .px_2()
                                    .py(px(1.))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(if connection.vision {
                                        palette.border
                                    } else {
                                        palette.warn
                                    }))
                                    .text_xs()
                                    .text_color(rgb(if connection.vision {
                                        palette.muted
                                    } else {
                                        palette.warn
                                    }))
                                    .cursor_pointer()
                                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        let on = !view
                                            .ai
                                            .connections
                                            .get(index)
                                            .is_some_and(|entry| entry.vision);
                                        view.set_connection_vision(index, on, cx);
                                    }))
                                    .child(if connection.vision { "图片：开" } else { "图片：关" }),
                            )
                            .child(help_icon(
                                "help-vision",
                                "这个接入点是否接受图片。关掉后请求里不再声明 read_image / screenshot（纯文本模型收到图片会被拒绝）；默认开启。",
                                palette,
                            ))
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
                            "模型 id，例如 deepseek-flash",
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
                self.settings_field_row_help(
                    "温度",
                    ("help-temperature", index),
                    "控制回答的随机性：0 几乎只用概率最高的词，稳定、可复现，适合改代码与按格式输出；0.7 左右是日常问答；1 以上更发散，适合头脑风暴，也更容易跑题或编造。默认＝不发送 temperature，用端点自己的值（DeepSeek 为 1）。思考不是「关闭」时端点会忽略温度，这一行会变灰、点不动——DeepSeek 的思考模式默认就开着。",
                    temperature_row(index, connection.temperature, connection.reasoning, palette, cx),
                    palette,
                ),
            )
            .child(
                self.settings_field_row_help(
                    "思考",
                    ("help-thinking", index),
                    "思考是模型回答前的推理过程，deepseek-flash / deepseek-v4-pro 默认开启、力度 high；思考会按轮记进对话，也计入费用。默认＝不发送任何思考字段，沿用端点设置；关闭＝发送 {\"thinking\":{\"type\":\"disabled\"}}，更快更省，也不会产生「思考」记录；低 / 高 / 最高＝发送 reasoning_effort 的 low / high / max。思考开着时端点会忽略温度。",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(settings_slider(
                            "reasoning",
                            REASONING_CHOICES.len(),
                            reasoning_notch(connection.reasoning),
                            true,
                            palette,
                            cx,
                            move |view, notch, cx| {
                                view.set_reasoning(index, REASONING_CHOICES[notch], cx)
                            },
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if connection.reasoning == Reasoning::Default {
                                    palette.muted
                                } else {
                                    palette.text
                                }))
                                .child(reasoning_label(connection.reasoning)),
                        )
                        .child(settings_default_button(
                            ("reasoning-default", index),
                            connection.reasoning == Reasoning::Default,
                            palette,
                            cx,
                            move |view, cx| view.set_reasoning(index, Reasoning::Default, cx),
                        )),
                    palette,
                ),
            )
            .child(
                self.settings_field_row_help(
                    "最大输出",
                    ("help-max-tokens", index),
                    "限制一次回答最多生成多少 token。默认＝不发送 max_tokens，此时端点给的是：非思考 8k、思考 64k、思考且力度最高 128k——行内显示的就是这个数，切思考时会跟着移。手动选定的值直接发送，上限 384k（deepseek-flash / deepseek-v4-pro 的封顶）。设得太小会把回答截断；上限本身不影响费用，费用只按实际生成算。",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(settings_slider(
                            "max-tokens",
                            MAX_TOKENS_CHOICES.len(),
                            max_tokens_notch(connection.max_tokens, connection.reasoning),
                            true,
                            palette,
                            cx,
                            move |view, notch, cx| {
                                view.set_max_tokens(index, MAX_TOKENS_CHOICES[notch], cx)
                            },
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if connection.max_tokens == 0 {
                                    palette.muted
                                } else {
                                    palette.text
                                }))
                                .child(token_label(connection.max_tokens, connection.reasoning)),
                        )
                        .child(settings_default_button(
                            ("max-tokens-default", index),
                            connection.max_tokens == 0,
                            palette,
                            cx,
                            move |view, cx| view.set_max_tokens(index, 0, cx),
                        )),
                    palette,
                ),
            )
            .child(
                self.settings_field_row_help(
                    "上下文窗口",
                    ("help-window", index),
                    "这个端点一次能带多少 token。默认按模型名估算（deepseek-flash / deepseek-v4 为 1M，未知模型 128k），行内显示估算值；本地模型或更新于内置表的模型请在这里指定。引擎按它的 80% 触发压缩，标题下的占用条也用它计算。",
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .flex_wrap()
                        .gap_2()
                        .child(settings_slider(
                            "window",
                            WINDOW_CHOICES.len(),
                            window_notch(connection.window, &connection.model),
                            true,
                            palette,
                            cx,
                            move |view, notch, cx| view.set_window(index, WINDOW_CHOICES[notch], cx),
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(if connection.window == 0 {
                                    palette.muted
                                } else {
                                    palette.text
                                }))
                                .child(window_label(connection.window, &connection.model)),
                        )
                        .child(settings_default_button(
                            ("window-default", index),
                            connection.window == 0,
                            palette,
                            cx,
                            move |view, cx| view.set_window(index, 0, cx),
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

    /// One label/value row whose explanation is behind a "?" beside the label.
    ///
    /// Labels here are two or three characters, so the icon rides in the same
    /// narrow column as the text and the control keeps the whole width.
    pub(crate) fn settings_field_row_help(
        &self,
        label: &'static str,
        id: impl Into<ElementId>,
        explanation: impl Into<String>,
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
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(div().text_xs().text_color(rgb(palette.faint)).child(label))
                    .child(help_icon(id, explanation, palette)),
            )
            .child(div().flex_1().min_w(px(0.)).child(value))
    }
}

/// The temperature rail: 0.0–2.0 in quarters, nine notches.
///
/// A slider rather than presets because the useful range is continuous — 0.3
/// and 0.5 behave differently, and neither used to be reachable. Quarters keep
/// the rail readable and still land on every figure DeepSeek recommends
/// (0, 1.0, 1.3, 1.5).
pub(crate) const TEMPERATURE_NOTCHES: usize = 9;

/// One notch of that rail: 2.0 over eight steps.
pub(crate) const TEMPERATURE_STEP: f64 = 0.25;

/// What an unset temperature stands for: DeepSeek's own default.
pub(crate) const DEFAULT_TEMPERATURE: f64 = 1.;

/// What the thinking slider offers, in order. "Default" is not a notch: it
/// means "send nothing", and then the knob parks where the endpoint sits.
const REASONING_CHOICES: [Reasoning; 4] = [
    Reasoning::Off,
    Reasoning::Low,
    Reasoning::High,
    Reasoning::Max,
];

/// Where the knob parks when thinking is left to the endpoint: deepseek-flash
/// and deepseek-v4-pro think at `high`, so that is the notch the row shows.
const REASONING_DEFAULT_NOTCH: usize = 2;

/// The answer-length rail: 2k up to the 384K ceiling DeepSeek's V4 models take.
const MAX_TOKENS_CHOICES: [u32; 8] = [
    2_048, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072, 393_216,
];

/// The window rail: the sizes worth picking between 32k and 1M.
const WINDOW_CHOICES: [u64; 6] = [32_768, 65_536, 131_072, 262_144, 524_288, 1_000_000];

/// The cap the endpoint applies when `max_tokens` is not sent.
///
/// DeepSeek's own: 8K without thinking, 64K with it, 128K at the top effort.
/// [`Reasoning::Default`] leaves thinking on at `high`, so it lands with `High`.
fn default_max_tokens(reasoning: Reasoning) -> u32 {
    match reasoning {
        Reasoning::Off => 8_192,
        Reasoning::Max => 131_072,
        Reasoning::Default | Reasoning::Low | Reasoning::High => 65_536,
    }
}

/// Where a stored temperature sits on the rail: the nearest quarter.
///
/// An unset value parks the knob where DeepSeek's own default is (1.0), so the
/// rail still shows the reader what the endpoint is doing.
pub(crate) fn temperature_notch(temperature: Option<f64>) -> usize {
    let value = temperature.unwrap_or(DEFAULT_TEMPERATURE);
    ((value.clamp(0., 2.) / TEMPERATURE_STEP).round() as usize).min(TEMPERATURE_NOTCHES - 1)
}

/// The temperature row: the rail, the figure in use, and 恢复默认.
///
/// Inert while the model thinks: DeepSeek's thinking mode ignores `temperature`
/// (and thinking is on by default), so the row greys out and says why rather
/// than pretending the figure is doing anything.
fn temperature_row(
    index: usize,
    temperature: Option<f64>,
    reasoning: Reasoning,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    let notch = temperature_notch(temperature);
    let enabled = reasoning == Reasoning::Off;
    let at_default = temperature.is_none();
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_wrap()
        .gap_2()
        .child(settings_slider(
            "temperature",
            TEMPERATURE_NOTCHES,
            notch,
            enabled,
            palette,
            cx,
            move |view, notch, cx| {
                view.set_temperature(index, Some(notch as f64 * TEMPERATURE_STEP), cx);
            },
        ))
        .child(
            div()
                .text_xs()
                .text_color(rgb(if at_default || !enabled {
                    palette.muted
                } else {
                    palette.text
                }))
                .child(temperature_label(
                    temperature.unwrap_or(DEFAULT_TEMPERATURE),
                )),
        )
        .child(settings_default_button(
            ("temperature-default", index),
            at_default || !enabled,
            palette,
            cx,
            move |view, cx| view.set_temperature(index, None, cx),
        ))
        .when(!enabled, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child("思考开启时无效"),
            )
        })
        .into_any_element()
}

/// "1", "1.25", "0.75" — the figure without trailing zeros.
fn temperature_label(temperature: f64) -> String {
    let text = format!("{temperature:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Where a stored reasoning level sits on its rail.
pub(crate) fn reasoning_notch(reasoning: Reasoning) -> usize {
    if reasoning == Reasoning::Default {
        return REASONING_DEFAULT_NOTCH;
    }
    REASONING_CHOICES
        .iter()
        .position(|choice| *choice == reasoning)
        .unwrap_or(REASONING_DEFAULT_NOTCH)
}

/// The name the thinking row shows beside its rail.
///
/// An unset level reads as the notch it parks on, which is the effort the
/// endpoint picks by itself; the row greys it and the 恢复默认 link explains it.
fn reasoning_label(reasoning: Reasoning) -> &'static str {
    match reasoning {
        Reasoning::Default => "高",
        Reasoning::Off => "关闭",
        Reasoning::Low => "低",
        Reasoning::High => "高",
        Reasoning::Max => "最高",
    }
}

/// Where a stored answer cap sits on its rail; an unlisted figure lands on the
/// nearest notch, which is also what picking it would send. Unset parks on the
/// cap the endpoint applies for the thinking level in force.
pub(crate) fn max_tokens_notch(max_tokens: u32, reasoning: Reasoning) -> usize {
    let value = if max_tokens == 0 {
        default_max_tokens(reasoning)
    } else {
        max_tokens
    };
    nearest_notch(
        &MAX_TOKENS_CHOICES,
        |choice| *choice == value,
        |choice| u64::from(*choice),
        u64::from(value),
    )
}

/// Where a stored window sits on its rail. Unset parks on the window the name
/// table gives this model, which is what the gauge and the engine will use.
pub(crate) fn window_notch(window: u64, model: &str) -> usize {
    let value = if window == 0 {
        crate::ai::context_window(model)
    } else {
        window
    };
    nearest_notch(
        &WINDOW_CHOICES,
        |choice| *choice == value,
        |choice| *choice,
        value,
    )
}

/// The index of `value` in `choices`, or of the closest one when it is not
/// listed (an older config can hold 6 000 tokens, which has no notch).
fn nearest_notch<T>(
    choices: &[T],
    exact: impl Fn(&T) -> bool,
    as_value: impl Fn(&T) -> u64,
    value: u64,
) -> usize {
    if let Some(found) = choices.iter().position(exact) {
        return found;
    }
    choices
        .iter()
        .enumerate()
        .min_by_key(|(_, choice)| as_value(choice).abs_diff(value))
        .map(|(index, _)| index)
        .unwrap_or(0)
}

/// The figure, for the answer cap — the value the endpoint will use, which for
/// an unset cap depends on the thinking level.
fn token_label(max_tokens: u32, reasoning: Reasoning) -> String {
    let value = if max_tokens == 0 {
        default_max_tokens(reasoning)
    } else {
        max_tokens
    };
    match value {
        value if value % 1024 == 0 => format!("{}k", value / 1024),
        value => value.to_string(),
    }
}

/// The figure, for the context window — the size the gauge will use, which for
/// an unset window comes from the model name.
fn window_label(window: u64, model: &str) -> String {
    let value = if window == 0 {
        crate::ai::context_window(model)
    } else {
        window
    };
    match value {
        1_000_000 => "1M".to_string(),
        value if value % 1024 == 0 && value >= 1024 => format!("{}k", value / 1024),
        value => value.to_string(),
    }
}

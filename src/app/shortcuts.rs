//! The keyboard-shortcut sheet: every key the window answers, in one place.

use super::*;

/// One shortcut row: the keys on a soft chip, then what they do.
fn shortcut_row(keys: &str, what: &str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .w_full()
        .min_w(px(0.))
        .child(
            div()
                .flex_shrink_0()
                .w(px(200.))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .bg(rgb(palette.panel_soft))
                .border_1()
                .border_color(rgb(palette.border))
                .font_family(MONO_FONT)
                .text_xs()
                .text_color(rgb(palette.text))
                .child(keys.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(what.to_string()),
        )
        .into_any_element()
}

/// Every window-level key, grouped: what the sheet and the settings page list.
pub(crate) const SHORTCUT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "对话",
        &[
            ("Ctrl+N", "新建一个对话"),
            ("Ctrl+F", "在当前对话里查找（Enter 下一个）"),
        ],
    ),
    (
        "阅读",
        &[
            ("PageUp / PageDown", "向上 / 向下翻一屏对话"),
            ("Ctrl+Home / Ctrl+End", "跳到最早一条 / 最新一条"),
            ("Ctrl+A", "选中整个对话（输入框里则是全选文字）"),
            ("Ctrl+C", "复制选中的内容"),
        ],
    ),
    (
        "应用",
        &[
            ("Ctrl+T", "在深色 / 浅色主题之间切换"),
            ("Ctrl+R", "运行工作流（工作流页）"),
            ("Ctrl+/ 或 F1", "打开 / 关闭这张快捷键表"),
            ("Esc", "关闭弹层、查找、选择；再按一次回到对话"),
        ],
    ),
];

/// The shortcut list as rows: a heading per group, then its keys.
///
/// One copy, used by the sheet the title bar's help button opens.
pub(crate) fn shortcut_rows(palette: Palette) -> Vec<AnyElement> {
    let mut rows: Vec<AnyElement> = Vec::new();
    for (group, items) in SHORTCUT_GROUPS {
        rows.push(
            div()
                .w_full()
                .pt_1()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.muted))
                .child(group.to_string())
                .into_any_element(),
        );
        for (keys, what) in *items {
            rows.push(shortcut_row(keys, what, palette));
        }
    }
    rows
}

impl DshFlowApp {
    /// The shortcut sheet, centred over a dimmed window.
    ///
    /// The sheet itself is the dismiss layer: a click on the dim area closes it,
    /// a click on the card does not. `Ctrl+/` and `F1` toggle it from anywhere,
    /// and `Esc` closes it before anything else Escape would cancel.
    pub(crate) fn render_shortcuts(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let card = div()
            .w(px(600.))
            .flex()
            .flex_col()
            .gap_2()
            .p_5()
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_lg()
            // Clicks inside the card are the card's, not the sheet's.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
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
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child("键盘快捷键"),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child("Ctrl+/ 或 F1 再按一次关闭"),
                    )
                    .child(
                        div()
                            .id("shortcuts-close")
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.shortcuts_open = false;
                                cx.notify();
                            }))
                            .child(div().text_xs().text_color(rgb(palette.muted)).child("×")),
                    ),
            )
            .child(div().flex_1())
            .children(shortcut_rows(palette));
        appear(card, "shortcuts-card", 8.).into_any_element()
    }
}

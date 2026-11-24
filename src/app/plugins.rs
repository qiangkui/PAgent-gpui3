//! The plugins page: built-in tools, externally installed plugins, and what is
//! still planned.
//!
//! Every entry is a card of the same size, with two lines of grey text; the
//! arrow in the corner opens the rest of a long description. The built-in cards
//! describe the current conversation (whether a tool asks first), the external
//! ones the whole program (whether a plugin loads at all).

use super::*;

/// Width of one card in the wrapping grid.
const CARD_WIDTH: f32 = 320.;

/// Height of a card while its description is collapsed.
///
/// Fixed so the grid lines up; two lines of `text_xs` plus the header row and
/// the card's own padding.
const CARD_HEIGHT: f32 = 84.;

/// Width units one line of a card's description holds.
///
/// Roughly: a CJK glyph is two units wide, an ASCII character one. Only used to
/// decide whether the corner arrow is worth showing, so a small error is fine.
const LINE_UNITS: f32 = 52.;

/// Inner padding on either side of a card (`px_3`).
const CARD_PADDING: f32 = 12.;

/// Gap between the pieces of a card's header row (`gap_2`).
const CARD_GAP: f32 = 8.;

/// How many width units a string takes, counting wide characters twice.
fn text_units(text: &str) -> f32 {
    text.chars()
        .map(|character| if character.is_ascii() { 1. } else { 2. })
        .sum()
}

/// Whether a description needs more than the two lines a card shows.
fn overflows(text: &str) -> bool {
    text_units(text) > LINE_UNITS * 2.
}

/// Width left for a card's name once the dot, the trailing control, the arrow
/// and the optional "…" menu have taken theirs.
///
/// The name needs a definite width to be truncated instead of running under the
/// switch, so it is computed up front rather than left to `flex_1`.
fn name_width(dot: bool, trailing: Option<f32>, arrow: bool, menu: bool) -> f32 {
    let mut width = CARD_WIDTH - CARD_PADDING * 2.;
    if dot {
        width -= 7. + CARD_GAP;
    }
    if let Some(trailing) = trailing {
        width -= trailing + CARD_GAP;
    }
    if arrow {
        width -= 16. + CARD_GAP;
    }
    if menu {
        width -= 18. + CARD_GAP;
    }
    width.max(32.)
}

impl DshFlowApp {
    /// Plugins: what the assistant can call, and which external plugins are on.
    pub(crate) fn render_plugins(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let builtin = [
            (
                "read",
                "读取工作区里的文本文件，带行号，最多 400 行 / 20 万字符",
            ),
            ("write", "新建或整体覆盖文件，自动建目录，单次 ≤ 40 万字符"),
            ("edit", "替换一处或多处只出现一次的文本，支持 edits 批量"),
            (
                "replace",
                "按 glob 选多个文件，批量替换同一段文本；默认每文件唯一",
            ),
            ("move", "移动或重命名文件；目标已存在时拒绝，默认执行前确认"),
            ("delete", "删除单个文件（不删文件夹），默认需确认，不能撤销"),
            ("list", "列出文件夹的直接子项，目录以 / 结尾，不递归"),
            (
                "glob",
                "按通配符列文件，例如 src/**/*.ts，跳过 .git / node_modules",
            ),
            (
                "tree",
                "按树形看结构（可设 depth），跳过 .git / node_modules / target，遵守 .gitignore",
            ),
            (
                "grep",
                "按正则搜内容，可忽略大小写、带上下文；返回「路径:行号: 内容」，最多 200 行",
            ),
            (
                "symbols",
                "按名字找声明（Rust / TS / JS / Python / Go），首次建索引并缓存，10 分钟内复用；find: references 查整词引用",
            ),
            (
                "git",
                "只读查看仓库：status / diff / log / show / ls-files 等，不会改动工作区",
            ),
            (
                "shell",
                "在当前目录运行命令（测试 / 构建 / 格式化）；路径受沙箱约束，默认执行前确认，可给 stdin 或用 background 转为后台任务",
            ),
            (
                "task",
                "管理后台任务：list 列出、output 看输出尾部、stop 结束一个、stopAll 结束全部",
            ),
            (
                "python",
                "用 python -c 运行一段 Python 代码；支持多行，默认执行前确认",
            ),
            (
                "node",
                "用 node -e 运行一段 Node.js 代码；支持多行，默认执行前确认",
            ),
            (
                "http",
                "发一个 GET / 有限 POST 请求（只允许 http / https，不跟随重定向，不发 authorization / cookie），默认执行前确认",
            ),
            (
                "mcp",
                "连接并调用 MCP 服务器：list 看 mcpserver 目录，link 按 id 或 command/url 连接，tools 看本次已连接，call 调用工具，disconnect 断开；连接在本次提问内保持",
            ),
        ];
        let planned = [
            ("approve / pause", "人工确认与 2FA 接管，拒绝则中止并留痕"),
            ("fetch 录制回放", "离线运行完整流程"),
            ("use 子流程", "把 login / fetch / parse 拆开复用"),
            ("state 游标", "第二次运行只处理新增数据"),
        ];
        let chat = self.chat();
        let mut slot = 0usize;
        let mut builtin_cards: Vec<AnyElement> = Vec::new();
        for (name, description) in builtin {
            let writable = matches!(name, "write" | "edit" | "replace");
            // Commands and destructive file actions ask in every level except
            // 完全权限.
            let always = matches!(
                name,
                "shell" | "task" | "python" | "node" | "http" | "move" | "delete" | "mcp"
            );
            let asks = chat.tools && !chat.auto_actions && (always || (chat.read_only && writable));
            let trusted = chat.tools && chat.auto_actions && always;
            let (dot, note, note_colour) = if !chat.tools {
                (palette.faint, "工具已关", palette.faint)
            } else if trusted {
                (palette.failed, "不再询问", palette.failed)
            } else if asks {
                (palette.warn, "需确认", palette.warn)
            } else {
                (palette.ok, "直接执行", palette.muted)
            };
            let status: AnyElement = div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(note_colour))
                .child(note)
                .into_any_element();
            builtin_cards.push(self.plugin_card(
                name,
                palette,
                cx,
                slot,
                asks || trusted,
                Some(dot),
                name,
                Some((status, 52.)),
                description,
                palette.muted,
                dot,
                None,
            ));
            slot += 1;
        }

        let mut planned_cards: Vec<AnyElement> = Vec::new();
        for (name, description) in planned {
            planned_cards.push(self.plugin_card(
                name,
                palette,
                cx,
                slot,
                false,
                None,
                name,
                None,
                description,
                palette.muted,
                palette.border,
                None,
            ));
            slot += 1;
        }

        let enabled = self
            .installed_plugins
            .iter()
            .filter(|plugin| self.ai.plugin_enabled(&plugin.id))
            .count();
        // Installed folders, plus any plugin still switched on whose folder is
        // gone: leaving that one invisible would hide why its tools vanished.
        let mut entries: Vec<(String, String, String, Option<String>)> = self
            .installed_plugins
            .iter()
            .map(|plugin| {
                let problem = if plugin.problem.is_empty() {
                    self.plugin_errors
                        .iter()
                        .find(|(id, _)| *id == plugin.id)
                        .map(|(_, message)| message.clone())
                } else {
                    Some(plugin.problem.clone())
                };
                (
                    plugin.id.clone(),
                    plugin.name.clone(),
                    plugin.version.clone(),
                    problem,
                )
            })
            .collect();
        for id in &self.ai.plugins {
            if !entries.iter().any(|(entry, ..)| entry == id) {
                entries.push((
                    id.clone(),
                    id.clone(),
                    String::new(),
                    Some("插件文件夹不存在（仍标记为启用）".to_string()),
                ));
            }
        }
        let mut external_cards: Vec<AnyElement> = Vec::new();
        for (id, name, version, problem) in entries {
            let on = self.ai.plugin_enabled(&id);
            let detail = if version.is_empty() {
                id.clone()
            } else {
                format!("{id} · {version}")
            };
            let toggle_id = id.clone();
            let border = if problem.is_some() {
                palette.failed
            } else {
                palette.border
            };
            let trailing = switch(slot, on, palette, cx, move |view, cx| {
                let want = !view.ai.plugin_enabled(&toggle_id);
                view.ai.set_plugin(&toggle_id, want);
                if let Err(error) = view.ai.save() {
                    view.banner = Some(Banner::error(error));
                }
                view.plugin_errors.retain(|(entry, _)| *entry != toggle_id);
                cx.notify();
            });
            // The id (and version) is always shown; a load failure follows it in
            // the same two lines, in red.
            let (text, colour) = match problem {
                Some(message) => (format!("{detail}　{message}"), palette.failed),
                None => (detail, palette.muted),
            };
            let menu_id = id.clone();
            let menu = plugin_menu_button(slot, palette, cx, move |view, position, cx| {
                let same = view
                    .plugin_menu
                    .as_ref()
                    .is_some_and(|(entry, _)| *entry == menu_id);
                view.category_menu = None;
                view.card_menu = None;
                view.confirm_delete = None;
                view.mcp_menu = None;
                view.mcp_delete_confirm = None;
                view.mcp_skills = None;
                view.plugin_delete_confirm = None;
                view.skill_menu = None;
                view.skill_delete_confirm = None;
                view.plugin_menu = if same {
                    None
                } else {
                    Some((menu_id.clone(), position))
                };
                cx.notify();
            });
            external_cards.push(self.plugin_card(
                &id,
                palette,
                cx,
                slot,
                false,
                None,
                &name,
                Some((trailing, 34.)),
                &text,
                colour,
                border,
                Some(menu),
            ));
            slot += 1;
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .child(page_title("插件", Vec::new(), palette))
            .child(
                div()
                    .id("plugins-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .px_6()
                                .py_4()
                                .child(plugin_heading("内置插件", palette))
                                .child(grid(builtin_cards))
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .flex_wrap()
                                        .gap_2()
                                        .px_1()
                                        .pt_3()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(palette.text))
                                                .child("外部插件"),
                                        )
                                        .child(icon_button(
                                            "plugins-folder",
                                            "icons/folder-open.svg",
                                            palette,
                                            cx,
                                            |view, cx| {
                                                if let Some(dir) =
                                                    crate::config::AiSettings::plugin_dir()
                                                {
                                                    let _ = std::fs::create_dir_all(&dir);
                                                    let _ = open_path(&dir);
                                                }
                                                view.refresh_plugins(cx);
                                            },
                                        ))
                                        .child(icon_button(
                                            "plugins-rescan",
                                            "icons/regenerate2.svg",
                                            palette,
                                            cx,
                                            |view, cx| view.refresh_plugins(cx),
                                        ))
                                        .child(icon_button(
                                            "plugins-example",
                                            "icons/plus.svg",
                                            palette,
                                            cx,
                                            |view, cx| view.create_example_plugin(cx),
                                        ))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(rgb(palette.faint))
                                                .child(format!(
                                                    "{} 个已安装 · {enabled} 个已启用",
                                                    self.installed_plugins.len()
                                                )),
                                        ),
                                )
                                .when(!self.plugin_note.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .whitespace_normal()
                                            .child(self.plugin_note.clone()),
                                    )
                                })
                                .child(if external_cards.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .whitespace_normal()
                                        .child(
                                            "还没有外部插件。点「外部插件」旁边的 + 生成示例，或自己建一个文件夹再点刷新图标。",
                                        )
                                        .into_any_element()
                                } else {
                                    grid(external_cards).into_any_element()
                                })
                                .child(plugin_heading("计划中", palette))
                                .child(grid(planned_cards)),
                    ),
            )
    }

    /// One card: a header row built here, then the grey description.
    ///
    /// A collapsed card is a fixed height and shows two lines; the corner arrow
    /// appears when the text is longer than that, and opens the rest.
    pub(crate) fn plugin_card(
        &self,
        key: &str,
        palette: Palette,
        cx: &mut Context<Self>,
        slot: usize,
        tinted: bool,
        dot: Option<u32>,
        name: &str,
        trailing: Option<(AnyElement, f32)>,
        description: &str,
        description_colour: u32,
        border: u32,
        menu: Option<AnyElement>,
    ) -> AnyElement {
        let open = self.open_plugin_cards.iter().any(|entry| entry == key);
        let arrow = overflows(description);
        let name_width = name_width(
            dot.is_some(),
            trailing.as_ref().map(|(_, width)| *width),
            arrow,
            menu.is_some(),
        );
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.));
        if let Some(dot) = dot {
            header = header.child(status_dot(dot));
        }
        header = header.child(
            div()
                .w(px(name_width.max(32.)))
                .min_w(px(0.))
                .whitespace_normal()
                .line_clamp(1)
                .text_ellipsis()
                .text_sm()
                .text_color(rgb(palette.text))
                .child(name.to_string()),
        );
        if let Some((trailing, _)) = trailing {
            header = header.child(trailing);
        }
        if arrow {
            let key = key.to_string();
            header = header.child(arrow_glyph(slot, open, palette, cx, move |view, cx| {
                view.toggle_plugin_card(&key, cx);
            }));
        }
        if let Some(menu) = menu {
            header = header.child(menu);
        }
        div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(CARD_WIDTH))
            .min_w(px(0.))
            .px_3()
            .py_2()
            .rounded_lg()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(if tinted { border } else { palette.border }))
            .when(!open, |this| this.h(px(CARD_HEIGHT)).overflow_hidden())
            .child(header)
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_xs()
                    .text_color(rgb(description_colour))
                    .whitespace_normal()
                    .when(!open, |this| this.line_clamp(2).text_ellipsis())
                    .child(description.to_string()),
            )
            .into_any_element()
    }

    /// Open or close one card's description.
    pub(crate) fn toggle_plugin_card(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.open_plugin_cards.iter().position(|entry| entry == key) {
            self.open_plugin_cards.remove(index);
        } else {
            self.open_plugin_cards.push(key.to_string());
        }
        cx.notify();
    }
}

/// Wrap the cards into as many columns as the pane has room for.
pub(crate) fn grid(cards: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_start()
        .gap_2()
        .w_full()
        .min_w(px(0.))
        .children(cards)
}

/// The "…" button on an external plugin card; opens the one-item menu.
pub(crate) fn plugin_menu_button(
    slot: usize,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, Point<Pixels>, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(("plugin-card-menu", slot))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(18.))
        .rounded_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |view, event: &gpui::ClickEvent, _, cx| {
            cx.stop_propagation();
            run(view, event.position(), cx);
        }))
        .child(
            svg()
                .path("icons/ellipsis.svg")
                .w(px(12.))
                .h(px(12.))
                .text_color(rgb(palette.muted)),
        )
        .into_any_element()
}

/// The corner arrow: down when closed, up when open.
fn arrow_glyph(
    slot: usize,
    open: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(("plugin-card-more", slot))
        .flex_shrink_0()
        .px_1()
        .rounded_sm()
        .text_xs()
        .text_color(rgb(palette.faint))
        .cursor_pointer()
        .hover(move |style| {
            style
                .text_color(rgb(palette.text))
                .bg(rgb(palette.panel_soft))
        })
        .on_click(cx.listener(move |view, _, _, cx| run(view, cx)))
        .child(if open { "▴" } else { "▾" })
        .into_any_element()
}

/// The on/off switch on an external plugin card.
pub(crate) fn switch(
    slot: usize,
    on: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    // A track with a knob rather than a checkbox: the state is the whole point
    // of the control, so it has to read at a glance.
    div()
        .id(("plugin-switch", slot))
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .w(px(34.))
        .h(px(18.))
        .p(px(2.))
        .rounded_full()
        .bg(rgb(if on { palette.accent } else { palette.chrome }))
        .border_1()
        .border_color(rgb(if on { palette.accent } else { palette.border }))
        .cursor_pointer()
        .when(on, |this| this.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(rgb(if on {
            palette.accent_text
        } else {
            palette.faint
        })))
        .on_click(cx.listener(move |view, _, _, cx| run(view, cx)))
        .into_any_element()
}

/// A group heading inside the card grid.
pub(crate) fn plugin_heading(title: &'static str, palette: Palette) -> AnyElement {
    div()
        .px_1()
        .pt_3()
        .text_sm()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(palette.text))
        .child(title)
        .into_any_element()
}

impl DshFlowApp {
    /// Rescan the plugin folder, after a toggle or when the page is opened.
    pub(crate) fn refresh_plugins(&mut self, cx: &mut Context<Self>) {
        self.installed_plugins = crate::config::installed_plugins();
        self.plugin_errors.clear();
        cx.notify();
    }

    /// Write a minimal, working plugin into the plugin folder.
    ///
    /// The fastest way to explain the format: a real folder with a real
    /// `activate(api)`, which the reader can open, edit and switch on. An
    /// existing folder is never overwritten.
    pub(crate) fn create_example_plugin(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = crate::config::AiSettings::plugin_dir() else {
            self.plugin_note = "找不到插件目录".to_string();
            cx.notify();
            return;
        };
        let folder = dir.join(EXAMPLE_PLUGIN_ID);
        if folder.exists() {
            let _ = open_path(&folder);
            self.plugin_note = format!(
                "{} 已存在，已打开它的文件夹（不会覆盖你改过的内容）。",
                EXAMPLE_PLUGIN_ID
            );
            self.refresh_plugins(cx);
            return;
        }
        let result = std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(folder.join("plugin.json"), EXAMPLE_PLUGIN_MANIFEST))
            .and_then(|()| std::fs::write(folder.join("index.js"), EXAMPLE_PLUGIN_SOURCE));
        self.plugin_note = match result {
            Ok(()) => format!(
                "已生成 {}：打开右边的开关，下一轮提问就能用 plugin_echo（每次调用会先弹确认卡）。",
                EXAMPLE_PLUGIN_ID
            ),
            Err(error) => format!("生成示例插件失败：{error}"),
        };
        self.refresh_plugins(cx);
    }

    /// The one-item card menu: delete this external plugin from disk.
    pub(crate) fn render_plugin_menu(
        &self,
        id: &str,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let armed = self.plugin_delete_confirm.as_deref() == Some(id);
        let target = id.to_string();
        div()
            .id("plugin-menu")
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_1()
            .w(px(168.))
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
                    .id("plugin-delete")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.failed))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        if view.plugin_delete_confirm.as_deref() == Some(target.as_str()) {
                            view.plugin_delete_confirm = None;
                            view.plugin_menu = None;
                            view.delete_plugin(&target, cx);
                        } else {
                            view.plugin_delete_confirm = Some(target.clone());
                            cx.notify();
                        }
                    }))
                    .child(if armed {
                        "再点一次确认删除"
                    } else {
                        "删除外部插件"
                    }),
            )
            .into_any_element()
    }

    /// Delete one external plugin folder and forget its switch.
    pub(crate) fn delete_plugin(&mut self, id: &str, cx: &mut Context<Self>) {
        let result = crate::config::delete_plugin(id);
        if let Err(error) = result {
            self.plugin_note = error;
            cx.notify();
            return;
        }
        if self.ai.plugin_enabled(id) {
            self.ai.set_plugin(id, false);
            if let Err(error) = self.ai.save() {
                self.banner = Some(Banner::error(error));
            }
        }
        self.plugin_errors.retain(|(entry, _)| entry != id);
        self.open_plugin_cards.retain(|entry| entry != id);
        self.refresh_plugins(cx);
        // A deleted plugin needs no success text; the card disappearing says it.
        self.plugin_note.clear();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{CARD_PADDING, CARD_WIDTH, LINE_UNITS, name_width, overflows, text_units};

    #[test]
    fn only_long_descriptions_get_an_arrow() {
        // Two lines at the card's width; a short one-liner must not show a
        // control that would change nothing.
        assert!(!overflows("删除单个文件"));
        assert!(!overflows("列出文件夹的直接子项，目录以 / 结尾，不递归"));
        assert!(overflows(
            "按树形看结构（可设 depth），跳过 .git / node_modules / target，遵守 .gitignore，并且会跳过所有被忽略的路径"
        ));
        // Wide characters count double, so a run of CJK reaches the limit twice
        // as fast as ASCII.
        assert!(text_units("中") > text_units("a"));
    }

    #[test]
    fn the_name_always_leaves_room_for_what_follows_it() {
        let full = CARD_WIDTH - CARD_PADDING * 2.;
        // Nothing else in the header: the whole inner width is the name's.
        assert_eq!(name_width(false, None, false, false), full);
        // Every extra piece only ever takes width away, so a long name cannot
        // end up under the switch or the arrow.
        assert!(name_width(true, None, false, false) < full);
        assert!(name_width(false, Some(34.), false, false) < full);
        assert!(name_width(false, None, true, false) < full);
        assert!(name_width(false, None, false, true) < full);
        assert!(name_width(true, Some(34.), true, true) < name_width(true, Some(34.), false, true));
        assert!(
            name_width(true, Some(34.), true, false) < name_width(true, Some(34.), false, false)
        );
        // A short terminal still gets a usable slice rather than a negative one.
        assert!(name_width(true, Some(400.), true, true) >= 32.);
        // The two lines a card shows are exactly the description budget.
        assert!(!overflows(&"a".repeat((LINE_UNITS * 2.) as usize)));
    }
}

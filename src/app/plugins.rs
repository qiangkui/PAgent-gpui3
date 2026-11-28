//! The plugins page: built-in tools and externally installed plugins.
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

/// Whether one card survives the tools filter.
///
/// The needle is already trimmed and lowercased by the caller; a name and a
/// description are compared the same way, so `Sandbox` and `沙箱` both find the
/// cards that mention them.
pub(crate) fn tool_matches(needle: &str, name: &str, description: &str) -> bool {
    needle.is_empty()
        || name.to_lowercase().contains(needle)
        || description.to_lowercase().contains(needle)
}

impl DshFlowApp {
    /// Plugins: what the assistant can call, and which external plugins are on.
    pub(crate) fn render_plugins(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        // The filter matches a card's name or description, case-insensitively:
        // "sandbox", "图片" or "commit" are the words a person actually types.
        let needle = self.tool_filter.text.trim().to_lowercase();
        let filtering = !needle.is_empty();
        let keep =
            |name: &str, description: &str| -> bool { tool_matches(&needle, name, description) };
        let mut slot = 0usize;
        let mut builtin_sections: Vec<AnyElement> = Vec::new();
        for group in ToolGroup::ALL {
            let mut cards: Vec<AnyElement> = Vec::new();
            for tool in BUILTIN_TOOLS.iter().filter(|tool| tool.group == group) {
                // The card shows what the engine says the tool does, once a run has
                // reported its schemas; the catalogue line covers a page opened
                // before that. The filter matches whichever text is on screen.
                let description = shown_description(tool, &self.tool_descriptions);
                if !keep(tool.name, &description) {
                    continue;
                }
                cards.push(self.builtin_tool_card(tool, &description, slot, palette, cx));
                slot += 1;
            }
            if cards.is_empty() {
                continue;
            }
            builtin_sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .child(tool_group_heading(group.label(), palette))
                    .child(grid(cards))
                    .into_any_element(),
            );
        }
        // While a filter is on, "no cards" has to say so instead of looking like
        // the page failed to load.
        let no_builtin_match = filtering && builtin_sections.is_empty();

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
            // The same filter covers external plugins by their folder name and
            // display name, so one box searches everything on the page.
            if !keep(&id, &name) {
                continue;
            }
            let on = self.ai.plugin_enabled(&id);
            // What this plugin added to the last request: two folders can look
            // alike and do very different things, so the card says which.
            let contribution = plugin_contribution(&id, &self.plugin_tools, &self.plugin_prompts);
            let detail = if version.is_empty() {
                format!("{id}{contribution}")
            } else {
                format!("{id} · {version}{contribution}")
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
            .child(div().px_6().pt_2().child(self.tool_filter_row(palette, cx)))
            .child(
                div()
                    .id("plugins-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scroll()
                        .contained_wheel()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .px_6()
                                .py_4()
                                .child(plugin_heading("内置插件", palette))
                                .when(no_builtin_match, |this| {
                                    this.child(
                                        div()
                                            .px_1()
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(format!("没有匹配「{needle}」的工具")),
                                    )
                                })
                                .children(builtin_sections)
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_end()
                                        .flex_wrap()
                                        .gap(px(20.))
                                        .px_1()
                                        .pt_3()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui::FontWeight::BOLD)
                                                .text_color(rgb(palette.text))
                                                .child("外部插件"),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_end()
                                                .gap_1()
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
                                                    "plugins-add",
                                                    "icons/plus.svg",
                                                    palette,
                                                    cx,
                                                    |view, cx| view.add_plugin(cx),
                                                )),
                                        )
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
                                    let note = if filtering {
                                        format!("没有匹配「{needle}」的外部插件。")
                                    } else {
                                        "还没有外部插件。点「外部插件」旁边的 + 选择一个插件文件夹（里面有 plugin.json），\
                                         会复制到插件目录并启用；也可以自己建一个文件夹再点刷新图标。"
                                            .to_string()
                                    };
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.faint))
                                        .whitespace_normal()
                                        .child(note)
                                        .into_any_element()
                                } else {
                                    grid(external_cards).into_any_element()
                                }),
                    ),
            )
    }

    /// The filter above the tool cards: a tool name, or a word from a description.
    ///
    /// It reuses the same text field machinery as the sidebar's conversation
    /// search, so Escape empties it and the caret behaves the same way.
    fn tool_filter_row(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("tool-filter-row")
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .h(px(34.))
            .px_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(palette.border))
            .cursor_text()
            .on_click(cx.listener(|view, _, window, cx| {
                view.focused = Some(Field::ToolFilter);
                if let Some(field) = view.field(Field::ToolFilter) {
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
                Field::ToolFilter,
                "过滤工具（名字或描述）",
                palette,
                cx,
            )))
            .when(!self.tool_filter.text.is_empty(), |this| {
                this.child(
                    div()
                        .id("tool-filter-clear")
                        .flex_shrink_0()
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(palette.panel_soft)))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.tool_filter.clear();
                            cx.notify();
                        }))
                        .child("✕"),
                )
            })
            .into_any_element()
    }

    /// One built-in tool card, with the note its permission level produces.
    ///
    /// The note is about the conversation, not the tool: the same card says
    /// 「需确认」 in one level and 「直接执行」 in another, and the automatic
    /// review rule can turn a must-ask tool into one granted by rule.
    fn builtin_tool_card(
        &self,
        tool: &ToolCard,
        description: &str,
        slot: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chat = self.chat();
        let name = tool.name;
        let writable = tool.writes;
        // Commands and destructive file actions ask in every level except
        // 完全权限.
        let always = tool.always;
        // `replace` rewrites several files at once, so the automatic review never
        // grants it by rule; `wait_for` only polls, so it does.
        let rule_grants = chat.auto_review
            && !chat.read_only
            && matches!(
                name,
                "write" | "edit" | "copy" | "mkdir" | "shell" | "wait_for"
            );
        let rule_asks = chat.auto_review && !chat.read_only && name == "replace";
        let asks = chat.tools
            && !chat.auto_actions
            && !rule_grants
            && (always || (chat.read_only && writable) || rule_asks);
        let trusted = chat.tools && chat.auto_actions && always;
        // With automatic review on, a rule-granted tool is approved by rule rather
        // than by the person; saying "需确认" for it would be wrong the other way.
        let by_rule = chat.tools && !chat.auto_actions && !trusted && rule_grants;
        let (dot, note, note_colour) = if !chat.tools {
            (palette.faint, "工具已关", palette.faint)
        } else if trusted {
            (palette.failed, "不再询问", palette.failed)
        } else if asks {
            (palette.warn, "需确认", palette.warn)
        } else if by_rule {
            (palette.warn, "按规则", palette.muted)
        } else {
            (palette.ok, "直接执行", palette.muted)
        };
        let status: AnyElement = div()
            .flex_shrink_0()
            .text_xs()
            .text_color(rgb(note_colour))
            .child(note)
            .into_any_element();
        self.plugin_card(
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

/// A section heading inside the built-in tools grid.
fn tool_group_heading(title: &'static str, palette: Palette) -> AnyElement {
    div()
        .px_1()
        .pt_2()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(palette.muted))
        .child(title)
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

    /// Ask for a plugin folder to copy into the plugin folder.
    pub(crate) fn add_plugin(&mut self, cx: &mut Context<Self>) {
        let opened = self.open_picker(
            PickPurpose::Plugin,
            || match crate::folder::pick_folder("Add plugin folder") {
                Some(folder) => PickResult::Folder(folder),
                None => PickResult::Cancelled,
            },
            cx,
        );
        if !opened {
            self.plugin_note = "已经有一个选择对话框打开了，先处理它。".to_string();
            cx.notify();
        }
    }

    /// Copy the chosen plugin folder in, switch it on and rescan.
    ///
    /// The folder keeps its own name as the id, and the manifest is checked the
    /// way the scan checks it, so a folder that would not load is refused
    /// before it reaches the plugin directory.
    pub(crate) fn install_plugin_folder(&mut self, source: &Path, cx: &mut Context<Self>) {
        match crate::config::import_plugin(source) {
            Ok(id) => {
                self.ai.set_plugin(&id, true);
                if let Err(error) = self.ai.save() {
                    self.banner = Some(Banner::error(error));
                }
                self.plugin_note = format!("已添加插件「{id}」并启用：下一轮提问会带上它。");
            }
            Err(error) => self.plugin_note = error,
        }
        self.refresh_plugins(cx);
        cx.notify();
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
    use super::{
        CARD_PADDING, CARD_WIDTH, LINE_UNITS, name_width, overflows, text_units, tool_matches,
    };

    #[test]
    fn the_tool_filter_matches_names_and_descriptions() {
        // Empty needle keeps everything; the page shows all cards by default.
        assert!(tool_matches("", "read", "读取文件"));
        assert!(tool_matches("read", "read", "读取文件"));
        // Descriptions are searched too, and the needle is already lowercase.
        assert!(tool_matches("沙箱", "shell", "路径受沙箱约束"));
        assert!(tool_matches("shell", "SHELL", "任意"));
        assert!(!tool_matches("沙箱", "read", "读取文件"));
        // A name that merely contains the needle still matches.
        assert!(tool_matches("read", "read_image", "x"));
    }

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

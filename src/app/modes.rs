//! 模式: which external plugins, skills and MCP servers a request may carry.

use super::*;

impl DshFlowApp {
    /// Settings → 模式.
    pub(crate) fn render_modes_settings(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let selected = if self.ai.mode_exists(&self.mode_selected) {
            self.mode_selected.clone()
        } else {
            crate::config::MODE_FULL.to_string()
        };
        let mut mode_rows: Vec<AnyElement> = Vec::new();
        mode_rows.push(mode_row(
            crate::config::MODE_BASIC,
            "基础模式",
            "只带内置工具；不加载外部插件、Skill 或 MCP",
            selected == crate::config::MODE_BASIC,
            false,
            palette,
            cx,
        ));
        mode_rows.push(mode_row(
            crate::config::MODE_FULL,
            "全量模式",
            "带上所有手动启用的外部插件、Skill 与 MCP",
            selected == crate::config::MODE_FULL,
            false,
            palette,
            cx,
        ));
        for mode in &self.ai.modes {
            mode_rows.push(mode_row(
                &mode.id,
                &mode.name,
                &format!(
                    "插件 {} · Skill {} · MCP {}",
                    mode.plugins.len(),
                    mode.skills.len(),
                    mode.mcp_servers.len()
                ),
                selected == mode.id,
                true,
                palette,
                cx,
            ));
        }
        let list = settings_card("模式列表", palette)
            .child(settings_note(
                "基础/全量是内置模式；自定义模式单独选择资源。任何在插件、Skill、MCP 页面手动关闭的资源都不会被请求携带。",
                palette,
            ))
            .child(div().flex().flex_col().gap_1().children(mode_rows))
            .child(
                div().flex().flex_row().gap_2().pt_2().child(
                    settings_button(
                        "mode-create",
                        "新建自定义模式",
                        SettingsButton::Primary,
                        palette,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.create_custom_mode(cx))),
                ),
            );

        let editor = if selected == crate::config::MODE_BASIC {
            settings_card("基础模式", palette)
                .child(settings_note(
                    "内置工具照常可用；不发送任何外部插件、Skill 或 MCP 定义。",
                    palette,
                ))
                .into_any_element()
        } else if selected == crate::config::MODE_FULL {
            let enabled = self.ai.mode_selection(crate::config::MODE_FULL);
            settings_card("全量模式", palette)
                .child(settings_note(
                    "发送所有当前手动启用的资源。取消插件、Skill 或 MCP 页面上的开关，也会立即把它们从全量模式中移除。",
                    palette,
                ))
                .child(settings_text_row(
                    "当前携带",
                    format!(
                        "{} 个插件 · {} 个 Skill · {} 个 MCP",
                        enabled.plugins.len(),
                        enabled.skills.len(),
                        enabled.mcp_servers.len()
                    ),
                    SettingsTone::Normal,
                    palette,
                ))
                .into_any_element()
        } else if let Some(mode) = self.ai.modes.iter().find(|mode| mode.id == selected) {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.render_mode_plugins(mode, palette, cx))
                .child(self.render_mode_skills(mode, palette, cx))
                .child(self.render_mode_mcp(mode, palette, cx))
                .into_any_element()
        } else {
            settings_card("模式", palette)
                .child(settings_note("这个模式已不存在，请选择另一个模式。", palette))
                .into_any_element()
        };

        let mut page = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings_page_header(
                "模式",
                "选择每轮提问实际发送给模型的外部插件、Skill 和 MCP；内置工具不受影响。",
                palette,
            ))
            .child(list)
            .child(editor);
        if !self.mode_note.is_empty() {
            page = page.child(settings_notice(&self.mode_note, false, palette));
        }
        page
    }
}
impl DshFlowApp {
    fn render_mode_plugins(
        &self,
        mode: &crate::config::ChatMode,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        for plugin in &self.installed_plugins {
            rows.push(self.mode_resource_row(
                &plugin.id,
                &plugin.name,
                &plugin.id,
                self.ai.plugin_enabled(&plugin.id) && plugin.problem.is_empty(),
                mode.plugins.iter().any(|id| id == &plugin.id),
                !plugin.problem.is_empty(),
                palette,
                cx,
                ModeResource::Plugin,
            ));
        }
        if rows.is_empty() {
            rows.push(settings_note("没有安装外部插件。", palette));
        }
        settings_card("外部插件", palette)
            .child(settings_note(
                "只在这里勾选的插件会加载；插件页面手动关闭的项始终不可用。",
                palette,
            ))
            .child(div().flex().flex_col().gap_1().children(rows))
    }

    fn render_mode_skills(
        &self,
        mode: &crate::config::ChatMode,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        for skill in &self.installed_skills {
            rows.push(self.mode_resource_row(
                &skill.id,
                &skill.name,
                &skill.id,
                self.ai.skill_enabled(&skill.id) && skill.problem.is_empty(),
                mode.skills.iter().any(|id| id == &skill.id),
                !skill.problem.is_empty(),
                palette,
                cx,
                ModeResource::Skill,
            ));
        }
        if rows.is_empty() {
            rows.push(settings_note("没有安装 Skill。", palette));
        }
        settings_card("Skill", palette)
            .child(settings_note(
                "只在这里勾选的 Skill 指令会进入系统提示；Skill 页面手动关闭的项始终不可用。",
                palette,
            ))
            .child(div().flex().flex_col().gap_1().children(rows))
    }

    fn render_mode_mcp(
        &self,
        mode: &crate::config::ChatMode,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        for server in &self.installed_mcp {
            rows.push(self.mode_resource_row(
                &server.id,
                &server.name,
                &server.detail(),
                self.ai.mcp_server_enabled(&server.id) && server.problem.is_empty(),
                mode.mcp_servers.iter().any(|id| id == &server.id),
                !server.problem.is_empty(),
                palette,
                cx,
                ModeResource::Mcp,
            ));
        }
        if rows.is_empty() {
            rows.push(settings_note("没有安装 MCP 服务器。", palette));
        }
        settings_card("MCP", palette)
            .child(settings_note(
                "只有勾选且手动启用的 MCP 会打开，并发送 mcp 工具；未勾选时连 mcp 网关也不会出现。",
                palette,
            ))
            .child(div().flex().flex_col().gap_1().children(rows))
    }
}
impl DshFlowApp {
    #[allow(clippy::too_many_arguments)]
    fn mode_resource_row(
        &self,
        id: &str,
        label: &str,
        detail: &str,
        available: bool,
        chosen: bool,
        broken: bool,
        palette: Palette,
        cx: &mut Context<Self>,
        kind: ModeResource,
    ) -> AnyElement {
        let id = id.to_string();
        let checkbox_colour = if chosen {
            if available {
                palette.accent
            } else {
                palette.faint
            }
        } else {
            palette.border
        };
        let note = if broken {
            "配置有误"
        } else if !available {
            "全局关闭"
        } else if chosen {
            "已选择"
        } else {
            "未选择"
        };
        let note_colour = if broken {
            palette.failed
        } else if !available {
            palette.faint
        } else if chosen {
            palette.accent
        } else {
            palette.faint
        };
        let mut row = div()
            .id(format!("mode-resource-{id}"))
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_2()
            .py(px(5.))
            .rounded_md()
            .when(available, |this| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
            })
            .child(
                div()
                    .size(px(14.))
                    .flex_shrink_0()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(checkbox_colour))
                    .bg(rgb(if chosen { checkbox_colour } else { palette.panel })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(rgb(if available { palette.text } else { palette.faint }))
                            .child(label.to_string()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(detail.to_string()),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(rgb(note_colour))
                    .child(note),
            );
        if available {
            row = row.on_click(cx.listener(move |view, _, _, cx| {
                view.toggle_mode_resource(kind, &id, cx);
            }));
        }
        row.into_any_element()
    }
}

#[derive(Clone, Copy)]
enum ModeResource {
    Plugin,
    Skill,
    Mcp,
}
impl DshFlowApp {
    fn toggle_mode_resource(&mut self, kind: ModeResource, id: &str, cx: &mut Context<Self>) {
        if !self.ai.mode_exists(&self.mode_selected)
            || self.mode_selected == crate::config::MODE_BASIC
            || self.mode_selected == crate::config::MODE_FULL
        {
            return;
        }
        let selected = self.mode_selected.clone();
        let Some(mode) = self.ai.modes.iter_mut().find(|mode| mode.id == selected) else {
            return;
        };
        let list = match kind {
            ModeResource::Plugin => &mut mode.plugins,
            ModeResource::Skill => &mut mode.skills,
            ModeResource::Mcp => &mut mode.mcp_servers,
        };
        if let Some(index) = list.iter().position(|entry| entry == id) {
            list.remove(index);
        } else {
            list.push(id.to_string());
            list.sort_unstable();
        }
        if let Err(error) = self.ai.save() {
            self.mode_note = error;
        } else {
            self.mode_note = "模式已保存；下一轮提问生效。".to_string();
        }
        cx.notify();
    }

    pub(crate) fn create_custom_mode(&mut self, cx: &mut Context<Self>) {
        if self.ai.modes.len() >= crate::config::MAX_MODES {
            self.mode_note = format!("最多只能有 {} 个自定义模式", crate::config::MAX_MODES);
            cx.notify();
            return;
        }
        let id = self.ai.fresh_mode_id();
        let number = self.ai.modes.len() + 1;
        self.ai
            .modes
            .push(crate::config::ChatMode::empty(id.clone(), format!("自定义模式 {number}")));
        if let Err(error) = self.ai.save() {
            self.mode_note = error;
        } else {
            self.mode_note = "已新建模式，选择下方三个列表中的资源。".to_string();
        }
        self.mode_selected = id;
        cx.notify();
    }

    pub(crate) fn delete_custom_mode(&mut self, id: &str, cx: &mut Context<Self>) {
        let before = self.ai.modes.len();
        self.ai.modes.retain(|mode| mode.id != id);
        if before == self.ai.modes.len() {
            return;
        }
        for chat in &mut self.chats {
            if chat.mode == id {
                chat.mode = crate::config::MODE_FULL.to_string();
            }
        }
        self.save_chats();
        if let Err(error) = self.ai.save() {
            self.mode_note = error;
        } else {
            self.mode_note = "模式已删除；使用它的对话已回到全量模式。".to_string();
        }
        if self.mode_selected == id {
            self.mode_selected = crate::config::MODE_FULL.to_string();
        }
        self.mode_menu = false;
        cx.notify();
    }

    pub(crate) fn set_chat_mode(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = if self.ai.mode_exists(id) {
            id.to_string()
        } else {
            crate::config::MODE_FULL.to_string()
        };
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            if chat.mode == id {
                self.mode_menu = false;
                cx.notify();
                return;
            }
            chat.mode = id;
        }
        self.save_chats();
        self.mode_menu = false;
        cx.notify();
    }
}
impl DshFlowApp {
    /// The composer's mode picker: basic/full and every custom mode.
    pub(crate) fn render_mode_menu(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let current = self.chat().mode.clone();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (id, name, note) in [
            (
                crate::config::MODE_BASIC,
                "基础模式",
                "只带内置工具，不带外部资源",
            ),
            (
                crate::config::MODE_FULL,
                "全量模式",
                "所有手动启用的外部插件、Skill、MCP",
            ),
        ] {
            rows.push(self.mode_menu_row(id, name, note, current == id, palette, cx));
        }
        for mode in &self.ai.modes {
            let note = format!(
                "插件 {} · Skill {} · MCP {}",
                mode.plugins.len(),
                mode.skills.len(),
                mode.mcp_servers.len()
            );
            rows.push(self.mode_menu_row(&mode.id, &mode.name, &note, current == mode.id, palette, cx));
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
                    .child("本轮模式"),
            )
            .children(rows)
    }

    fn mode_menu_row(
        &self,
        id: &str,
        name: &str,
        note: &str,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.to_string();
        div()
            .id(format!("mode-menu-{id}"))
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
            .on_click(cx.listener(move |view, _, _, cx| view.set_chat_mode(&id, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if selected {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(name.to_string()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(note.to_string()),
                    ),
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
            .into_any_element()
    }
}

/// One row in Settings → 模式's mode list.
fn mode_row(
    id: &str,
    name: &str,
    note: &str,
    selected: bool,
    custom: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    let id = id.to_string();
    let select = id.clone();
    let delete = id.clone();
    let mut row = div()
        .id(format!("mode-row-{id}"))
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
            view.mode_selected = select.clone();
            view.mode_note.clear();
            cx.notify();
        }))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(if selected { palette.accent } else { palette.text }))
                        .child(name.to_string()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(palette.faint))
                        .child(note.to_string()),
                ),
        );
    if custom {
        row = row.child(
            settings_button(
                format!("mode-delete-{id}"),
                "删除",
                SettingsButton::Danger,
                palette,
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                view.delete_custom_mode(&delete, cx);
            })),
        );
    } else if selected {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.accent))
                .child("内置"),
        );
    }
    row.into_any_element()
}
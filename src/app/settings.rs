//! The settings pages and the colour editor.

use super::*;

/// Folder name of the example plugin the settings page can write.
pub(crate) const EXAMPLE_PLUGIN_ID: &str = "hello-example";

/// Its `plugin.json`: the smallest manifest the engine accepts.
pub(crate) const EXAMPLE_PLUGIN_MANIFEST: &str =
    "{\n  \"name\": \"示例：回显\",\n  \"version\": \"1.0.0\",\n  \"api\": 1\n}\n";

/// Its entry module: one tool and one prompt fragment, both wired for real.
pub(crate) const EXAMPLE_PLUGIN_SOURCE: &str = "// 一个最小可用的插件：注册一个工具，并在系统提示里加一句话。\n\
// 改完保存，下一轮提问就会用新版本（引擎每次都重新读这个文件夹）。\n\
export function activate(api) {\n\
  api.registerTool(\n\
    {\n\
      name: 'plugin_echo',\n\
      description: '把参数 text 原样返回，用来确认插件链路通了',\n\
      parameters: {\n\
        type: 'object',\n\
        properties: { text: { type: 'string', description: '要回显的文字' } },\n\
        required: ['text']\n\
      }\n\
    },\n\
    (args) => `echo: ${String(args.text ?? '')}`\n\
  )\n\
  api.injectPrompt('示例插件：需要确认工具链路时可以用 plugin_echo 回显一段文字。')\n\
}\n";

impl DshFlowApp {
    /// Settings: a left index of the blocks, and the chosen block on the right.
    ///
    /// Drawn inside the settings window, which owns the title bar and the
    /// rounded frame, so the page itself has no header and no inner border.
    pub(crate) fn render_settings(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let body = match self.settings_tab {
            SettingsTab::Interface => self.render_interface_settings(palette, cx),
            SettingsTab::Ai => self.render_ai_settings(palette, cx),
            SettingsTab::Instructions => self.render_instructions_settings(palette, cx),
            SettingsTab::Data => self.render_data_settings(palette, cx),
            SettingsTab::Maintenance => self.render_maintenance_settings(palette, cx),
        };

        div()
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .child(self.render_settings_nav(palette, cx))
            .child(
                div()
                    .id("settings-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    // The page is a sheet laid over the panel: its top-left corner is rounded
                    // so the navigation colour shows through the notch, and the
                    // bottom-right corner keeps the window's own 12px radius.
                    .bg(rgb(palette.canvas))
                    .rounded_tl(px(12.))
                    .rounded_br(px(12.))
                    // One rhythm for every page: the same page margin, the same
                    // gap between cards, whatever the page holds.
                    .child(div().flex().flex_col().gap_4().px_6().py_5().child(body)),
            )
    }

    /// The settings index: one row per block, the open one filled with the
    /// accent colour.
    pub(crate) fn render_settings_nav(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        for tab in SettingsTab::ALL {
            let selected = tab == self.settings_tab;
            let mut row = div()
                .id(tab.label())
                .px_3()
                .py_2()
                .rounded_md()
                .bg(rgb(if selected {
                    palette.accent
                } else {
                    palette.canvas
                }))
                .text_sm()
                .text_color(rgb(if selected {
                    palette.accent_text
                } else {
                    palette.muted
                }))
                .cursor_pointer();
            if !selected {
                row = row.hover(move |style| style.bg(rgb(palette.panel_soft)));
            }
            rows.push(
                row.on_click(cx.listener(move |view, _, _, cx| {
                    view.settings_tab = tab;
                    if tab == SettingsTab::Ai {
                        view.refresh_model_lists(cx);
                    }
                    if tab == SettingsTab::Instructions {
                        view.load_instructions(cx);
                    }
                    cx.notify();
                }))
                .child(tab.label())
                .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .w(px(196.))
            .flex_shrink_0()
            .h_full()
            .min_h(px(0.))
            .px_2()
            .py_3()
            .bg(rgb(palette.panel))
            .children(rows)
    }

    /// Switch to one of the palettes the window ships and drop any custom colour.
    ///
    /// Picking light or dark means "the system palette", so the overrides go with
    /// it: the third card is what the window reports while they exist, and a card
    /// that kept its colours would make the two real choices indistinguishable.
    pub(crate) fn set_theme(&mut self, theme: ThemeKind, cx: &mut Context<Self>) {
        if self.theme == theme && self.ai.colors.is_empty() {
            return;
        }
        self.theme = theme;
        self.ai.theme = theme.name().to_string();
        self.ai.clear_colors();
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        self.sync_color_fields(cx);
    }

    /// Swap light and dark, keeping the custom colours (`Ctrl+T`).
    ///
    /// The shortcut is a quick switch between the two backgrounds, not a way to
    /// throw away a palette the reader spent time on.
    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme.toggled();
        self.theme = theme;
        self.ai.theme = theme.name().to_string();
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
        // The rows show the colour they are painting with, and the base palette
        // just changed under them.
        self.sync_color_fields(cx);
    }

    /// The palette in use: the chosen theme with any custom colours laid over it.
    pub(crate) fn palette(&self) -> Palette {
        crate::theme::themed(self.theme, &self.ai.colors)
    }

    /// 界面: the palettes the window ships with, plus the custom one.
    pub(crate) fn render_interface_settings(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let custom = crate::theme::custom_mode(&self.ai.colors);
        let mut theme_cards: Vec<AnyElement> = Vec::new();
        for kind in ThemeKind::ALL {
            let selected = !custom && self.theme == kind;
            theme_cards.push(theme_card(kind, selected, palette, cx).into_any_element());
        }
        theme_cards.push(custom_theme_card(&palette, custom, palette, cx).into_any_element());

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings_page_header(
                "界面",
                "配色与自定义颜色，应用到整个窗口。",
                palette,
            ))
            .child(
                settings_section("主题", palette).child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_3()
                        .children(theme_cards),
                ),
            )
            .child(self.render_color_settings(palette, cx))
    }

    /// 自定义配色: one clickable swatch per role, with an inline colour grid.
    ///
    /// Clicking a swatch opens a grid right under that row instead of a dialog:
    /// gpui has none, and a helper process' window can come up behind this one
    /// (which read as "clicking does nothing"). A colour is applied and saved the
    /// moment it is picked.
    pub(crate) fn render_color_settings(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let open = self.color_menu;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, role) in crate::theme::COLOR_ROLES.iter().enumerate() {
            let key = role.key;
            let value = (role.get)(&palette);
            // A label in the accent colour means "this one is not the theme's
            // colour any more"; no extra text needed to say it.
            let changed = self.ai.color_of(key).is_some();
            let expanded = open == Some(index);
            rows.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(126.))
                            .flex_shrink_0()
                            .text_base()
                            .text_color(rgb(if changed {
                                palette.accent
                            } else {
                                palette.text
                            }))
                            .child(role.label),
                    )
                    .child(div().w(px(104.)).flex_shrink_0().child(self.render_field(
                        Field::ColorHex(index),
                        "#rrggbb",
                        palette,
                        cx,
                    )))
                    .child(
                        div()
                            .id(("color-swatch", index))
                            .w(px(96.))
                            .h(px(22.))
                            .flex_shrink_0()
                            .rounded_sm()
                            .border_1()
                            .border_color(rgb(if expanded {
                                palette.accent
                            } else {
                                palette.border
                            }))
                            .bg(rgb(value))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.color_menu = if view.color_menu == Some(index) {
                                    None
                                } else {
                                    Some(index)
                                };
                                cx.notify();
                            })),
                    )
                    .into_any_element(),
            );
            if expanded {
                rows.push(color_grid(key, palette, cx));
            }
        }

        settings_section("自定义配色", palette)
            .child(div().flex().flex_col().gap_2().children(rows))
    }

    /// 指令: the open conversation's own `AGENTS.md`, editable in place.
    ///
    /// The page is two cards: what the file affects, and the text itself. The
    /// path is shown because the assistant's rules are otherwise invisible, and
    /// the line under the buttons is the only place a save reports back.
    pub(crate) fn render_instructions_settings(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let file = self.instructions_dir().join(CHAT_RULES);
        let title = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.title.clone())
            .unwrap_or_default();
        let existing = file.is_file();

        let scope: Vec<AnyElement> = vec![
            settings_text_row(
                "当前对话",
                if title.is_empty() {
                    "（未命名）".to_string()
                } else {
                    title.clone()
                },
                SettingsTone::Normal,
                palette,
            ),
            settings_row(
                "保存位置",
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(settings_code(file.display().to_string(), palette))
                    .when(!existing, |this| {
                        this.child(settings_note("文件还不存在：保存一次后才会创建。", palette))
                    })
                    .into_any_element(),
                palette,
            ),
            div()
                .flex()
                .flex_row()
                .gap_2()
                .pt_1()
                .child(
                    settings_button(
                        "instructions-open",
                        "打开文件",
                        SettingsButton::Secondary,
                        palette,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.open_instructions_file(cx))),
                )
                .into_any_element(),
        ];

        let status = self
            .instructions_status
            .clone()
            .map(|(text, error)| settings_notice(&text, error, palette));

        let editor = settings_card("指令内容", palette)
            .child(settings_note(
                "每轮提问开始时读取；留空保存会删除文件。工作区根目录的 AGENTS.md 仍是兜底，\
                 超过 2 万字符的部分不会进入模型。",
                palette,
            ))
            .child(self.render_field(
                Field::ChatInstructions,
                "例如：回答用中文；改完代码先跑 cargo test",
                palette,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        settings_button(
                            "instructions-save",
                            "保存",
                            SettingsButton::Primary,
                            palette,
                        )
                        .on_click(cx.listener(|view, _, _, cx| view.save_instructions(cx))),
                    )
                    .child(
                        settings_button(
                            "instructions-clear",
                            "清空",
                            SettingsButton::Danger,
                            palette,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.instructions.clear();
                            view.save_instructions(cx);
                        })),
                    )
                    .when_some(status, |this, status| this.child(status)),
            );

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings_page_header(
                "对话指令",
                "只影响当前对话：保存到该对话目录的 AGENTS.md，下一轮提问开始时生效。",
                palette,
            ))
            .child(settings_card("作用范围", palette).children(scope))
            .child(editor)
    }

    /// Open the native file picker for one attachment kind.
    ///
    /// The dialog blocks its thread, and the chosen files are attached when it
    /// answers. The menu closes first so the dialog is not drawn under a popup.
    pub(crate) fn open_attach_picker(&mut self, kind: AttachmentKind, cx: &mut Context<Self>) {
        if self.picker_busy(cx) {
            return;
        }
        // The dialog belongs to the conversation that was open when it started;
        // a switch while it is up must not move the files into another question.
        let dir = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.dir.clone())
            .unwrap_or_default();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let answer = match crate::folder::pick_files(attach_filter(kind)) {
                Some(paths) => PickResult::Files(paths),
                None => PickResult::Cancelled,
            };
            let _ = sender.send(answer);
        });
        self.picker = Some((receiver, Instant::now(), dir));
        self.spawn_picker_pump(cx);
        self.attach_menu = false;
        cx.notify();
    }

    /// Forget a dialog that can no longer answer, then say whether one is open.
    ///
    /// An answer that has arrived but has not been collected yet must not swallow
    /// the click either, so it is pumped first.
    fn picker_busy(&mut self, cx: &mut Context<Self>) -> bool {
        self.pump_picker(cx);
        if self
            .picker
            .as_ref()
            .is_some_and(|(_, opened, _)| picker_stale(opened.elapsed()))
        {
            self.picker = None;
        }
        self.picker.is_some()
    }

    /// Poll the native dialog until it closes.
    pub(crate) fn spawn_picker_pump(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(60))
                    .await;
                match this.update(cx, |view, cx| {
                    view.pump_picker(cx);
                    cx.notify();
                    view.picker.is_some()
                }) {
                    Ok(true) => continue,
                    // A tick that could not borrow the app is not the end of the
                    // dialog: giving up here would leave `picker` set, and every
                    // later click would be ignored.
                    Err(_) => continue,
                    Ok(false) => break,
                }
            }
        })
        .detach();
    }

    /// Apply what the dialog answered, if it has closed.
    pub(crate) fn pump_picker(&mut self, cx: &mut Context<Self>) {
        let Some((receiver, _, dir)) = self.picker.as_ref() else {
            return;
        };
        let dir = dir.clone();
        let answer = match receiver.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => PickResult::Cancelled,
        };
        self.picker = None;
        match answer {
            PickResult::Cancelled => {}
            PickResult::Files(paths) => {
                let active = self
                    .chats
                    .get(self.active_chat)
                    .map(|chat| chat.dir.clone());
                if active.as_deref() == Some(dir.as_str()) {
                    self.attach_files(&paths, cx);
                } else {
                    // The files were chosen for a conversation that is no longer
                    // on screen; attaching them here would put them in the wrong
                    // question.
                    self.show_toast("对话已切换，已忽略刚选的文件", true, cx);
                }
            }
        }
    }

    /// Remember one picked colour and repaint.
    ///
    /// Picking the colour a role already has clears the override, so the file
    /// only keeps the roles the reader really changed.
    pub(crate) fn apply_color(&mut self, key: &'static str, value: u32) {
        let theme = self.theme;
        if self.ai.set_color(key, Some(value), theme).is_none() {
            return;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
    }

    /// Apply the colour one row's value box spells, if it spells one.
    ///
    /// Typing is live: a half-typed value simply does not apply, and the box keeps
    /// what was typed so the reader can finish it.
    pub(crate) fn commit_color_edit(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(role) = crate::theme::COLOR_ROLES.get(index) else {
            return;
        };
        let Some(text) = self
            .color_fields
            .get(index)
            .map(|field| field.text.trim().to_string())
        else {
            return;
        };
        let Some(value) = crate::theme::parse_hex(&text) else {
            return;
        };
        if (role.get)(&self.palette()) == value {
            return;
        }
        let key = role.key;
        self.apply_color(key, value);
        cx.notify();
    }

    /// Bring every row's value box in line with the palette in use.
    ///
    /// Only called when the colour changed somewhere other than a value box (the
    /// grid, a reset, a theme switch), so typing never has its caret moved.
    pub(crate) fn sync_color_fields(&mut self, cx: &mut Context<Self>) {
        let palette = self.palette();
        for (index, role) in crate::theme::COLOR_ROLES.iter().enumerate() {
            let text = crate::theme::hex((role.get)(&palette));
            if let Some(field) = self.color_fields.get_mut(index)
                && field.text != text
            {
                field.set_text(text);
            }
        }
        cx.notify();
    }

    /// Put one role back on the theme's own colour.
    pub(crate) fn reset_color(&mut self, key: &'static str) {
        if self.ai.set_color(key, None, self.theme).is_none() {
            return;
        }
        if let Err(error) = self.ai.save() {
            self.banner = Some(Banner::error(error));
        }
    }

    /// 数据: where the records live, and the backup / restore / export actions.
    pub(crate) fn render_data_settings(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let home = AiSettings::home();
        let data = self.ai.data_dir();
        let workspace = self.ai.workspace_dir();
        let backups = crate::backup::backup_root(&home);
        let exports = crate::backup::export_root(&home);
        let stats = crate::stats_store::ledger_dir();
        let data_for_open = data.clone();
        let workspace_for_open = workspace.clone();
        let stats_for_open = stats.clone();
        let location = settings_card("数据位置", palette)
            .child(settings_text_row(
                "数据目录",
                data.display().to_string(),
                SettingsTone::Normal,
                palette,
            ))
            .child(settings_text_row(
                "统计账本",
                format!("{}（明细 30 天，日汇总长期保留）", stats.display()),
                SettingsTone::Normal,
                palette,
            ))
            .child(settings_text_row(
                "工作区",
                format!("{}（不参与备份）", workspace.display()),
                SettingsTone::Normal,
                palette,
            ))
            .child(settings_text_row(
                "备份目录",
                backups.display().to_string(),
                SettingsTone::Normal,
                palette,
            ))
            .child(settings_text_row(
                "导出目录",
                exports.display().to_string(),
                SettingsTone::Normal,
                palette,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .pt_1()
                    .child(
                        settings_button(
                            "open-data",
                            "打开数据目录",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(move |_, _, _, _| {
                            let _ = open_path(&data_for_open);
                        })),
                    )
                    .child(
                        settings_button(
                            "open-workspace",
                            "打开工作区",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(move |_, _, _, _| {
                            let _ = open_path(&workspace_for_open);
                        })),
                    )
                    .child(
                        settings_button(
                            "open-stats",
                            "打开统计账本",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(move |_, _, _, _| {
                            let _ = open_path(&stats_for_open);
                        })),
                    ),
            );

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(settings_page_header(
                "数据",
                "记录放在哪里，以及备份、恢复和导出当前对话。",
                palette,
            ))
            .child(location)
            .child(self.render_backup_card(palette, cx))
    }

    /// The backup/restore/export card shown by the 数据 page.
    fn render_backup_card(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let home = AiSettings::home();
        let latest_restore = crate::backup::latest_backup(&home);
        let restore_pending = crate::backup::pending_restore(&home);
        let mut card = settings_card("备份与导出", palette)
            .child(settings_note(
                "备份复制可执行文件同目录的 data 目录、技能、插件、MCP 与 config.json 到 backups/（不含 workspace）；\
                 恢复会在下次启动时替换 data 与附加组件，并先自动备份当前状态；\
                 导出把当前对话写成 Markdown 到 exports/。",
                palette,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        settings_button("run-backup", "立即备份", SettingsButton::Primary, palette)
                            .on_click(cx.listener(|view, _, _, cx| view.run_backup(cx))),
                    )
                    .child(
                        settings_button(
                            "export-chat",
                            "导出当前对话",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(|view, _, _, cx| view.export_current_chat(cx))),
                    )
                    .child(
                        settings_button(
                            "open-backups",
                            "打开备份目录",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(|_, _, _, _| {
                            let _ = open_path(&crate::backup::backup_root(&AiSettings::home()));
                        })),
                    )
                    .when(latest_restore.is_some(), |this| {
                        this.child(
                            settings_button(
                                "restore-backup",
                                if self.restore_armed {
                                    "确认恢复最近备份"
                                } else {
                                    "恢复最近备份"
                                },
                                if self.restore_armed {
                                    SettingsButton::Danger
                                } else {
                                    SettingsButton::Secondary
                                },
                                palette,
                            )
                            .on_click(cx.listener(|view, _, _, cx| view.restore_latest(cx))),
                        )
                    })
                    .when(restore_pending.is_some(), |this| {
                        this.child(
                            settings_button(
                                "cancel-restore",
                                "取消恢复计划",
                                SettingsButton::Secondary,
                                palette,
                            )
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.cancel_restore_plan(cx)
                            })),
                        )
                    })
                    .child(
                        settings_button(
                            "open-exports",
                            "打开导出目录",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(|_, _, _, _| {
                            let _ = open_path(&crate::backup::export_root(&AiSettings::home()));
                        })),
                    )
                    .child(
                        settings_button(
                            "open-logs",
                            "打开日志目录",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(|_, _, _, _| {
                            let dir = crate::logging::path()
                                .and_then(|path| path.parent().map(Path::to_path_buf))
                                .unwrap_or_else(|| AiSettings::home().join("logs"));
                            let _ = open_path(&dir);
                        })),
                    ),
            );
        if !self.maintenance_note.is_empty() {
            card = card.child(settings_notice(
                &self.maintenance_note,
                self.maintenance_failed,
                palette,
            ));
        }
        card
    }

    /// 诊断: where everything lives, what is missing, and the two copies worth
    /// having — a backup of the records and an export of one conversation.
    pub(crate) fn render_maintenance_settings(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let node = self.node_version.clone();
        let healthy = node.is_some() && self.engine_ready;
        let mut environment: Vec<AnyElement> = vec![
            settings_text_row(
                "版本",
                format!("PAgent {}", env!("CARGO_PKG_VERSION")),
                SettingsTone::Normal,
                palette,
            ),
            settings_text_row(
                "Node.js",
                node.clone()
                    .unwrap_or_else(|| "未找到：请安装 Node.js，并确保 node 在 PATH 中".into()),
                if node.is_some() {
                    SettingsTone::Good
                } else {
                    SettingsTone::Bad
                },
                palette,
            ),
            settings_text_row(
                "引擎",
                if self.engine_ready {
                    self.engine_dir.display().to_string()
                } else {
                    format!("{}（缺少 src/cli.ts）", self.engine_dir.display())
                },
                if self.engine_ready {
                    SettingsTone::Good
                } else {
                    SettingsTone::Bad
                },
                palette,
            ),
            settings_text_row(
                "HTTP 代理",
                if self.ai.proxy.is_empty() {
                    "直连".to_string()
                } else {
                    self.ai.proxy.clone()
                },
                SettingsTone::Normal,
                palette,
            ),
            settings_text_row(
                "日志",
                crate::logging::path()
                    .map_or_else(|| "未初始化".to_string(), |path| path.display().to_string()),
                SettingsTone::Normal,
                palette,
            ),
            settings_text_row(
                "用量上限",
                if self.ai.budget_tokens == 0 {
                    "不限制".to_string()
                } else if self.chat_metrics().billed >= self.ai.budget_tokens {
                    format!(
                        "{} tok（当前对话已超）",
                        token_amount(self.ai.budget_tokens)
                    )
                } else {
                    format!("{} tok", token_amount(self.ai.budget_tokens))
                },
                if self.ai.budget_tokens > 0 && self.chat_metrics().billed >= self.ai.budget_tokens {
                    SettingsTone::Warn
                } else {
                    SettingsTone::Normal
                },
                palette,
            ),
        ];
        environment.push(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .pt_1()
                .child(
                    settings_button(
                        "copy-diagnostics",
                        "复制诊断信息",
                        SettingsButton::Secondary,
                        palette,
                    )
                    .on_click(cx.listener(|view, _, _, cx| view.copy_diagnostics(cx))),
                )
                .into_any_element(),
        );

        // The paths are no longer repeated here: the 数据 page owns them.

        // Snippets: one chip per file, inserted into the composer on click.
        let snippets_dir = crate::snippet::dir();
        let for_example = snippets_dir.clone();
        let mut snippet_card = settings_card("常用片段", palette)
            .child(settings_note(
                "把常用指令写成 snippets/ 里的 .md，点一下插入输入框。",
                palette,
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        settings_button(
                            "open-snippets",
                            "打开片段目录",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(move |_, _, _, _| {
                            let _ = open_path(&snippets_dir);
                        })),
                    )
                    .child(
                        settings_button(
                            "example-snippet",
                            "生成示例",
                            SettingsButton::Secondary,
                            palette,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.create_example_snippet(&for_example, cx);
                        })),
                    ),
            );
        let installed = crate::snippet::installed();
        if installed.is_empty() {
            snippet_card = snippet_card.child(settings_note("还没有片段。", palette));
        } else {
            let mut chips: Vec<AnyElement> = Vec::new();
            for (position, (name, file)) in installed.into_iter().enumerate() {
                chips.push(
                    settings_button(("snippet", position), &name, SettingsButton::Secondary, palette)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.insert_snippet(&file, cx);
                        }))
                        .into_any_element(),
                );
            }
            snippet_card =
                snippet_card.child(div().flex().flex_row().flex_wrap().gap_2().children(chips));
        }
        if !self.maintenance_note.is_empty() {
            // Snippet insertion still reports here; backup/restore notes live on
            // the 数据 page, but showing the shared status here costs nothing.
            snippet_card = snippet_card.child(settings_notice(
                &self.maintenance_note,
                self.maintenance_failed,
                palette,
            ));
        }

        let mut cards: Vec<AnyElement> = vec![
            settings_card("运行环境", palette)
                .children(environment)
                .into_any_element(),
            snippet_card.into_any_element(),
        ];
        if let Some(banner) = &self.banner {
            cards.push(
                settings_card("最近错误", palette)
                    .child(
                        div()
                            .text_xs()
                            .line_height(px(17.))
                            .whitespace_normal()
                            .text_color(rgb(palette.failed))
                            .child(banner.text.clone()),
                    )
                    .into_any_element(),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_3()
                    .child(
                        div().flex_1().min_w(px(0.)).child(settings_page_header(
                            "诊断",
                            "检查运行环境、文件位置，以及备份与导出。",
                            palette,
                        )),
                    )
                    .child(settings_status_pill(
                        healthy,
                        if healthy { "环境正常" } else { "需要处理" },
                        palette,
                    )),
            )
            .children(cards)
    }

    /// Copy the records into `backups/` and say where they went.
    pub(crate) fn run_backup(&mut self, cx: &mut Context<Self>) {
        self.restore_armed = false;
        let result = crate::backup::backup(&AiSettings::home(), &self.ai.data_dir());
        self.maintenance_failed = result.is_err();
        self.maintenance_note = match result {
            Ok(path) => format!("已备份到 {}", path.display()),
            Err(error) => error,
        };
        cx.notify();
    }

    /// Plan the newest backup for the next launch; the first press only warns.
    pub(crate) fn restore_latest(&mut self, cx: &mut Context<Self>) {
        let home = AiSettings::home();
        if !self.restore_armed {
            match crate::backup::latest_backup(&home) {
                Some(source) => {
                    self.restore_armed = true;
                    self.maintenance_failed = false;
                    self.maintenance_note = format!(
                        "再次点击将计划恢复 {}：下次启动时替换 data 与插件/技能/MCP，并先自动备份当前状态。",
                        source.display()
                    );
                }
                None => {
                    self.restore_armed = false;
                    self.maintenance_failed = true;
                    self.maintenance_note = "还没有可恢复的备份".to_string();
                }
            }
            cx.notify();
            return;
        }
        self.restore_armed = false;
        let result = crate::backup::plan_restore(&home);
        self.maintenance_failed = result.is_err();
        self.maintenance_note = match result {
            Ok(source) => format!(
                "已计划从 {} 恢复；关闭并重新启动应用后生效。",
                source.display()
            ),
            Err(error) => error,
        };
        cx.notify();
    }

    /// Forget a restore planned for the next launch.
    pub(crate) fn cancel_restore_plan(&mut self, cx: &mut Context<Self>) {
        let result = crate::backup::cancel_restore(&AiSettings::home());
        self.restore_armed = false;
        self.maintenance_failed = result.is_err();
        self.maintenance_note = match result {
            Ok(()) => "已取消下次启动的恢复计划".to_string(),
            Err(error) => error,
        };
        cx.notify();
    }

    /// Write the open conversation as Markdown and say where it went.
    pub(crate) fn export_current_chat(&mut self, cx: &mut Context<Self>) {
        let chat = self.chat();
        let body = transcript_markdown(chat);
        let title = chat.title.clone();
        let result = crate::backup::export_markdown(&AiSettings::home(), &title, &body);
        self.maintenance_failed = result.is_err();
        self.maintenance_note = match result {
            Ok(path) => format!("已导出到 {}", path.display()),
            Err(error) => error,
        };
        cx.notify();
    }

    /// Put the same facts on the clipboard, for a bug report.
    pub(crate) fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.diagnostics_text()));
        self.maintenance_note = "诊断信息已复制到剪贴板".to_string();
        self.maintenance_failed = false;
        cx.notify();
    }

    /// The diagnostics as plain text.
    fn diagnostics_text(&self) -> String {
        let home = AiSettings::home();
        let mut text = format!("PAgent {}\n", env!("CARGO_PKG_VERSION"));
        text.push_str(&format!(
            "node: {}\n",
            self.node_version.as_deref().unwrap_or("未找到")
        ));
        text.push_str(&format!(
            "engine: {}（{}）\n",
            self.engine_dir.display(),
            if self.engine_ready { "可用" } else { "缺少 src/cli.ts" }
        ));
        text.push_str(&format!("config: {}\n", home.join("config.json").display()));
        text.push_str(&format!("data: {}\n", self.ai.data_dir().display()));
        text.push_str(&format!(
            "stats: {}\n",
            crate::stats_store::ledger_dir().display()
        ));
        text.push_str(&format!("workspace: {}\n", self.ai.workspace_dir().display()));
        text.push_str(&format!("skills: {}\n", self.skills_dir.display()));
        text.push_str(&format!("mcp: {}\n", self.mcp_dir.display()));
        text.push_str(&format!(
            "proxy: {}\n",
            if self.ai.proxy.is_empty() { "无" } else { &self.ai.proxy }
        ));
        text.push_str(&format!("budget: {} tok\n", self.ai.budget_tokens));
        if let Some(banner) = &self.banner {
            text.push_str(&format!("last error: {}\n", banner.text));
        }
        for (id, message) in &self.plugin_errors {
            text.push_str(&format!("plugin {id}: {message}\n"));
        }
        for (id, message) in &self.mcp_errors {
            text.push_str(&format!("mcp {id}: {message}\n"));
        }
        if let Some(path) = crate::logging::path() {
            text.push_str(&format!("log: {}\n", path.display()));
            let tail = crate::logging::tail();
            if !tail.is_empty() {
                text.push_str("--- log tail ---\n");
                text.push_str(&tail);
                text.push('\n');
            }
        }
        text
    }

    /// Insert one snippet into the composer.
    pub(crate) fn insert_snippet(&mut self, file: &Path, cx: &mut Context<Self>) {
        let Some(text) = crate::snippet::read(file) else {
            self.maintenance_note = format!("读不到 {}", file.display());
            self.maintenance_failed = true;
            cx.notify();
            return;
        };
        self.composer.insert(&text);
        let name = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        self.maintenance_note = format!("已插入片段「{name}」");
        self.maintenance_failed = false;
        cx.notify();
    }

    /// Write the example snippet, once.
    pub(crate) fn create_example_snippet(&mut self, dir: &Path, cx: &mut Context<Self>) {
        let result = crate::snippet::create_example(dir);
        self.maintenance_failed = result.is_err();
        self.maintenance_note = match result {
            Ok((_, true)) => "已生成示例片段".to_string(),
            Ok((_, false)) => "示例片段已存在".to_string(),
            Err(error) => error,
        };
        cx.notify();
    }
}

/// The Windows file-dialog filter for one attachment kind.
///
/// Stays ASCII: the dialog script is handed to PowerShell as one command-line
/// argument, and a local code page must not be able to mangle it.
fn attach_filter(kind: AttachmentKind) -> &'static str {
    match kind {
        AttachmentKind::Text => {
            "Text and code|*.txt;*.md;*.markdown;*.rs;*.ts;*.tsx;*.js;*.jsx;*.json;*.yaml;*.yml;*.toml;*.ini;*.cfg;*.conf;*.csv;*.log;*.py;*.go;*.c;*.h;*.cpp;*.hpp;*.java;*.kt;*.cs;*.rb;*.php;*.sh;*.ps1;*.bat;*.html;*.css;*.scss;*.xml;*.sql|All files|*.*"
        }
        AttachmentKind::Image => "Images|*.png;*.jpg;*.jpeg;*.gif;*.webp;*.bmp|All files|*.*",
        AttachmentKind::Pdf => "PDF|*.pdf|All files|*.*",
    }
}

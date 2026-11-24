//! The MCP page: servers linked to the assistant.
//!
//! A server is a folder (or one entry in `mcp.json`) under the program's
//! `mcpserver` folder. The cards show what is linked, whether it is switched on
//! for the next question, and why an entry cannot be used. Nothing is started
//! here: the engine opens the enabled servers when a question asks for tools.

use super::*;

/// Width of one card in the wrapping grid.
const MCP_CARD_WIDTH: f32 = 340.;

/// Height of a card while its detail is collapsed.
const MCP_CARD_HEIGHT: f32 = 112.;

/// Inner padding on either side of a card (`px_3`).
const MCP_PADDING: f32 = 12.;

/// Width of the skills panel. Wider than a card so parameter JSON wraps less.
pub(crate) const MCP_SKILLS_WIDTH: f32 = 560.;

/// Tallest the skills panel gets, header included.
pub(crate) const MCP_SKILLS_HEIGHT: f32 = 540.;

/// Height left for the skill list inside the panel.
const MCP_SKILLS_BODY: f32 = 420.;

/// Width of the skills panel's scrollbar column.
const MCP_SKILLS_BAR: f32 = 10.;

/// Gap between the pieces of a card's header row (`gap_2`).
const MCP_GAP: f32 = 8.;

/// Width units one line of a card's text holds; a CJK glyph counts twice.
const MCP_LINE_UNITS: f32 = 56.;

/// How many width units a string takes, counting wide characters twice.
fn text_units(text: &str) -> f32 {
    text.chars()
        .map(|character| if character.is_ascii() { 1. } else { 2. })
        .sum()
}

/// Whether a description needs more than the two lines a card shows.
fn overflows(text: &str) -> bool {
    text_units(text) > MCP_LINE_UNITS * 2.
}

/// Width left for a card's name once the dot, the switch and the arrow have
/// taken theirs.
fn name_width(dot: bool, trailing: Option<f32>, arrow: bool) -> f32 {
    let mut width = MCP_CARD_WIDTH - MCP_PADDING * 2.;
    if dot {
        width -= 7. + MCP_GAP;
    }
    if let Some(trailing) = trailing {
        width -= trailing + MCP_GAP;
    }
    if arrow {
        width -= 16. + MCP_GAP;
    }
    // The "…" delete menu always sits at the far right.
    width -= 18. + MCP_GAP;
    width.max(32.)
}

impl DshFlowApp {
    /// MCP: the servers found under `mcpserver`, and their switches.
    pub(crate) fn render_mcp(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut cards: Vec<AnyElement> = Vec::new();
        for (slot, server) in self.installed_mcp.iter().enumerate() {
            let on = self.ai.mcp_server_enabled(&server.id);
            let engine_error = self
                .mcp_errors
                .iter()
                .find(|(id, _)| *id == server.id)
                .map(|(_, message)| message.clone());
            let problem = if server.problem.is_empty() {
                engine_error
            } else {
                Some(server.problem.clone())
            };
            let failed = problem.is_some();
            let dot = if failed {
                palette.failed
            } else if on {
                palette.ok
            } else {
                palette.faint
            };
            let border = if failed {
                palette.failed
            } else {
                palette.border
            };
            let toggle_id = server.id.clone();
            // A manifest that cannot be read has nothing worth switching on;
            // a server that merely failed at runtime can still be turned off.
            let trailing = if !server.problem.is_empty() {
                None
            } else {
                Some((
                    mcp_switch(slot, on, palette, cx, move |view, cx| {
                        let want = !view.ai.mcp_server_enabled(&toggle_id);
                        view.ai.set_mcp_server(&toggle_id, want);
                        if let Err(error) = view.ai.save() {
                            view.banner = Some(Banner::error(error));
                        }
                        // The failure belonged to the run it happened in;
                        // after a switch the next question reports afresh.
                        view.mcp_errors.retain(|(id, _)| *id != toggle_id);
                        view.mcp_note =
                            format!("{}：{}", if want { "已启用" } else { "已停用" }, toggle_id);
                        cx.notify();
                    }),
                    34.,
                ))
            };
            let status = if failed {
                "配置有误"
            } else if on {
                "已启用"
            } else {
                "未启用"
            };
            let skills = server.skills.len();
            let mut summary = format!("{status} · {}", server.detail());
            if !server.description.is_empty() {
                summary.push_str(" · ");
                summary.push_str(&server.description);
            }
            if failed {
                summary = format!("{} · {}", problem.unwrap_or_default(), server.detail());
            }
            cards.push(self.mcp_card(
                &server.id,
                palette,
                cx,
                slot,
                failed,
                dot,
                &server.name,
                trailing,
                &summary,
                if failed {
                    palette.failed
                } else {
                    palette.muted
                },
                border,
                skills,
                server,
            ));
        }

        let mut body = div().flex().flex_col().gap_2().px_6().py_4();
        if self.installed_mcp.is_empty() {
            body = body.child(empty_state(&self.mcp_dir, palette));
        } else {
            body = body.child(mcp_grid(cards));
        }
        if !self.mcp_note.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(self.mcp_note.clone()),
            );
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .child(page_title(
                "MCP",
                vec![
                    icon_button(
                        "mcp-folder",
                        "icons/folder-open.svg",
                        palette,
                        cx,
                        |view, cx| {
                            let _ = std::fs::create_dir_all(&view.mcp_dir);
                            let _ = open_path(&view.mcp_dir);
                            view.refresh_mcp(cx);
                        },
                    ),
                    icon_button(
                        "mcp-rescan",
                        "icons/regenerate2.svg",
                        palette,
                        cx,
                        |view, cx| view.refresh_mcp(cx),
                    ),
                    icon_button("mcp-example", "icons/plus.svg", palette, cx, |view, cx| {
                        view.create_example_mcp(cx)
                    }),
                ],
                palette,
            ))
            .child(
                div()
                    .id("mcp-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    /// One card: name and switch on top, then the grey line about the server.
    #[allow(clippy::too_many_arguments)]
    fn mcp_card(
        &self,
        key: &str,
        palette: Palette,
        cx: &mut Context<Self>,
        slot: usize,
        tinted: bool,
        dot: u32,
        name: &str,
        trailing: Option<(AnyElement, f32)>,
        description: &str,
        description_colour: u32,
        border: u32,
        skills: usize,
        server: &crate::mcp::McpServer,
    ) -> AnyElement {
        let open = self.open_mcp_cards.iter().any(|entry| entry == key);
        let arrow = overflows(description) || tinted;
        let name_width = name_width(true, trailing.as_ref().map(|(_, width)| *width), arrow);
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .child(status_dot(dot))
            .child(
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
            header = header.child(mcp_arrow(slot, open, palette, cx, move |view, cx| {
                view.toggle_mcp_card(&key, cx);
            }));
        }
        let menu_key = key.to_string();
        header = header.child(mcp_menu_button(
            slot,
            palette,
            cx,
            move |view, position, cx| {
                let same = view
                    .mcp_menu
                    .as_ref()
                    .is_some_and(|(id, _)| *id == menu_key);
                // The press was kept from the root so the toggle could see the old
                // state; close the sidebar menus explicitly for the same reason.
                view.category_menu = None;
                view.card_menu = None;
                view.confirm_delete = None;
                view.plugin_menu = None;
                view.plugin_delete_confirm = None;
                view.skill_menu = None;
                view.skill_delete_confirm = None;
                view.mcp_skills = None;
                view.mcp_delete_confirm = None;
                view.mcp_menu = if same {
                    None
                } else {
                    Some((menu_key.clone(), position))
                };
                cx.notify();
            },
        ));
        let mut card = div()
            .flex()
            .flex_col()
            .gap_1()
            .w(px(MCP_CARD_WIDTH))
            .min_w(px(0.))
            .px_3()
            .py_2()
            .rounded_lg()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(if tinted { border } else { palette.border }))
            .when(!open, |this| this.h(px(MCP_CARD_HEIGHT)).overflow_hidden())
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
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(format!("{skills} 个技能")),
            );
        if open {
            let mut detail = format!("配置文件：{}", server.source.display());
            if !server.cwd.is_empty() {
                detail.push_str(&format!("\n运行目录：{}", server.cwd));
            }
            if !server.env.is_empty() {
                let names: Vec<&str> = server.env.iter().map(|(key, _)| key.as_str()).collect();
                detail.push_str(&format!("\n环境变量：{}", names.join(", ")));
            }
            if !server.headers.is_empty() {
                let names: Vec<&str> = server.headers.iter().map(|(key, _)| key.as_str()).collect();
                detail.push_str(&format!("\nHTTP 请求头：{}", names.join(", ")));
            }
            if !server.version.is_empty() {
                detail.push_str(&format!("\n版本：{}", server.version));
            }
            card = card.child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .pt_1()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .whitespace_normal()
                    .child(detail),
            );
        }
        card.into_any_element()
    }

    /// Open or close one card's detail.
    pub(crate) fn toggle_mcp_card(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.open_mcp_cards.iter().position(|entry| entry == key) {
            self.open_mcp_cards.remove(index);
        } else {
            self.open_mcp_cards.push(key.to_string());
        }
        cx.notify();
    }

    /// Rescan the MCP folder, when the page is opened or after an action.
    pub(crate) fn refresh_mcp(&mut self, cx: &mut Context<Self>) {
        self.mcp_dir = crate::mcp::locate_dir();
        self.installed_mcp = crate::mcp::scanned(&self.mcp_dir);
        // A failure reported for a server that no longer exists would outlive it.
        self.mcp_errors
            .retain(|(id, _)| self.installed_mcp.iter().any(|server| server.id == *id));
        // A fetch owns the note until it finishes; a rescan should not hide it.
        if self.mcp_fetch.is_none() {
            self.mcp_note.clear();
        }
        cx.notify();
    }

    /// Write a self-contained example server into the MCP folder.
    pub(crate) fn create_example_mcp(&mut self, cx: &mut Context<Self>) {
        self.mcp_dir = crate::mcp::locate_dir();
        match crate::mcp::create_example(&self.mcp_dir) {
            Ok((folder, true)) => {
                self.mcp_note = format!(
                    "已生成 {}：打开右边的开关，下一次提问就能用 mcp_example_server_echo。",
                    folder.display()
                );
            }
            Ok((folder, false)) => {
                let _ = open_path(&folder);
                self.mcp_note = format!(
                    "{} 已存在，已打开它的文件夹（不会覆盖你改过的内容）。",
                    folder.display()
                );
            }
            Err(error) => self.mcp_note = error,
        }
        self.installed_mcp = crate::mcp::scanned(&self.mcp_dir);
        cx.notify();
    }

    /// Fetch one server's skills without sending a question.
    ///
    /// The engine connects the server once and writes `skills/<id>/…`; the card
    /// count then comes from the folder like any other scan. Only one fetch runs
    /// at a time, and none while a question is open, so two connections never
    /// write the same files.
    pub(crate) fn fetch_mcp_skills(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.mcp_fetch.is_some() {
            self.mcp_note = "正在获取技能，请稍候。".to_string();
            cx.notify();
            return;
        }
        if !self.pending.is_empty() {
            self.mcp_note = "正在提问，等这次回答结束后再获取技能。".to_string();
            cx.notify();
            return;
        }
        let Some(server) = self
            .installed_mcp
            .iter()
            .find(|server| server.id == id)
            .cloned()
        else {
            self.mcp_note = format!("找不到 MCP 服务器「{id}」，已重新扫描。");
            self.refresh_mcp(cx);
            return;
        };
        if !server.problem.is_empty() {
            self.mcp_note = server.problem;
            cx.notify();
            return;
        }
        self.mcp_skills = None;
        self.mcp_note = format!("正在获取「{}」的技能…", server.name);
        let engine_dir = self.engine_dir.clone();
        let dir = self.mcp_dir.clone();
        let target = id.to_string();
        let (sender, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(crate::mcp::fetch_skills(&engine_dir, &dir, &target));
        });
        self.mcp_fetch = Some((id.to_string(), Instant::now(), result));
        self.spawn_mcp_fetch_pump(cx);
        cx.notify();
    }

    /// Poll the one-shot skill fetch while it runs.
    fn spawn_mcp_fetch_pump(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        view.pump_mcp_fetch(cx);
                        cx.notify();
                        view.mcp_fetch.is_some()
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Record a finished skill fetch.
    pub(crate) fn pump_mcp_fetch(&mut self, cx: &mut Context<Self>) {
        let Some((id, started, result)) = self
            .mcp_fetch
            .as_ref()
            .map(|(id, started, result)| (id.clone(), *started, result))
        else {
            return;
        };
        let outcome = match result.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if started.elapsed() > Duration::from_secs(90) {
                    self.mcp_fetch = None;
                    self.mcp_note =
                        format!("获取「{id}」的技能超时（90 秒），检查服务器是否能启动。");
                    cx.notify();
                }
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(format!("获取「{id}」的技能失败：引擎进程意外退出。"))
            }
        };
        self.mcp_fetch = None;
        match outcome {
            Ok(fetched) => {
                self.installed_mcp = crate::mcp::scanned(&self.mcp_dir);
                // A successful connection is fresher evidence than a failure
                // from an earlier question.
                self.mcp_errors.retain(|(entry, _)| *entry != id);
                let name = self
                    .installed_mcp
                    .iter()
                    .find(|server| server.id == id)
                    .map_or_else(|| id.clone(), |server| server.name.clone());
                self.mcp_note = if fetched.count == 0 {
                    fetched.text
                } else {
                    format!(
                        "已获取「{name}」的 {} 个技能，打开右上角菜单里的「技能」查看。",
                        fetched.count
                    )
                };
            }
            Err(error) => self.mcp_note = error,
        }
        cx.notify();
    }

    /// The card menu: fetch skills, show them, or delete the server.
    pub(crate) fn render_mcp_menu(
        &self,
        id: &str,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let armed = self.mcp_delete_confirm.as_deref() == Some(id);
        let fetching = self
            .mcp_fetch
            .as_ref()
            .is_some_and(|(fetching, _, _)| fetching == id);
        let target = id.to_string();
        let skills_target = target.clone();
        let fetch_target = target.clone();
        let skills = self
            .installed_mcp
            .iter()
            .find(|server| server.id == id)
            .map_or(0, |server| server.skills.len());
        let panel_at = point(at.x, at.y);
        // Fetching is the only way skills appear without asking a question, so
        // it sits above the list and says what it is doing while it runs.
        let fetch_item = if fetching {
            div()
                .id("mcp-fetch")
                .px_3()
                .py_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("正在获取技能…")
        } else {
            div()
                .id("mcp-fetch")
                .px_3()
                .py_1()
                .rounded_sm()
                .text_xs()
                .text_color(rgb(palette.text))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    view.mcp_menu = None;
                    view.mcp_delete_confirm = None;
                    view.fetch_mcp_skills(&fetch_target, cx);
                }))
                .child("获取技能")
        };
        div()
            .id("mcp-menu")
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
            // Presses inside the menu must not reach the root, which closes it.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(fetch_item)
            .child(
                div()
                    .id("mcp-skills")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.text))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.mcp_menu = None;
                        view.mcp_delete_confirm = None;
                        view.mcp_skills_open = None;
                        view.mcp_skills_drag = None;
                        view.mcp_skills_scroll.set_offset(point(px(0.), px(0.)));
                        view.mcp_skills = Some((skills_target.clone(), panel_at));
                        view.spawn_mcp_skills_repaint(cx);
                        cx.notify();
                    }))
                    .child(format!("技能（{skills}）")),
            )
            .child(
                div()
                    .id("mcp-delete")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.failed))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        // Deleting a folder is not undoable, so the first press
                        // only arms the item.
                        if view.mcp_delete_confirm.as_deref() == Some(target.as_str()) {
                            view.mcp_delete_confirm = None;
                            view.mcp_menu = None;
                            view.delete_mcp_server(&target, cx);
                        } else {
                            view.mcp_delete_confirm = Some(target.clone());
                            cx.notify();
                        }
                    }))
                    .child(if armed {
                        "再点一次确认删除"
                    } else {
                        "删除 MCP 服务器"
                    }),
            )
            .into_any_element()
    }

    /// The skills panel: every tool the last successful connection reported.
    pub(crate) fn render_mcp_skills(
        &self,
        id: &str,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let server = self.installed_mcp.iter().find(|server| server.id == id);
        let name = server.map_or_else(|| id.to_string(), |server| server.name.clone());
        let skills: &[crate::mcp::McpSkill] = server.map_or(&[], |server| server.skills.as_slice());
        let count = skills.len();
        let folder = self.mcp_dir.join("skills").join(id);
        let rows: Vec<AnyElement> = if skills.is_empty() {
            vec![
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(
                        "还没有技能：点这张卡右上角「…」菜单里的「获取技能」让引擎连接一次，或启用它后提一次问——连接成功时会把技能写进 mcpserver\\skills\\<id>。",
                    )
                    .into_any_element(),
            ]
        } else {
            skills
                .iter()
                .enumerate()
                .map(|(index, skill)| {
                    let open = self.mcp_skills_open.as_deref() == Some(skill.name.as_str());
                    let key = skill.name.clone();
                    let last = index + 1 == skills.len();
                    let mut row = div()
                        .id(("mcp-skill", index))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_3()
                        .cursor_pointer()
                        .when(!last, |this| {
                            this.border_b_1().border_color(rgb(palette.border))
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            let same = view.mcp_skills_open.as_deref() == Some(key.as_str());
                            view.mcp_skills_open = if same { None } else { Some(key.clone()) };
                            view.spawn_mcp_skills_repaint(cx);
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .w_full()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .text_sm()
                                        .text_color(rgb(palette.text))
                                        .whitespace_normal()
                                        .child(skill.name.clone()),
                                )
                                .when(!skill.schema.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette.faint))
                                            .child(if open { "▾" } else { "▸" }),
                                    )
                                }),
                        );
                    if !skill.description.is_empty() {
                        row = row.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .whitespace_normal()
                                .child(skill.description.clone()),
                        );
                    }
                    if open && !skill.schema.is_empty() {
                        row = row.child(
                            div()
                                .pt_1()
                                .text_xs()
                                .text_color(rgb(palette.faint))
                                .whitespace_normal()
                                .child(format!("参数：\n{}", skill.schema)),
                        );
                    }
                    row.into_any_element()
                })
                .collect()
        };
        div()
            .id("mcp-skills-panel")
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .w(px(MCP_SKILLS_WIDTH))
            .max_h(px(MCP_SKILLS_HEIGHT))
            .rounded_lg()
            .bg(rgb(palette.canvas))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(palette.text))
                            .child(format!("技能 · {name}")),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .child(format!("{count} 个")),
                    )
                    .child(icon_button(
                        "mcp-skill-folder",
                        "icons/folder-open.svg",
                        palette,
                        cx,
                        move |view, cx| {
                            let _ = std::fs::create_dir_all(&folder);
                            let _ = open_path(&folder);
                            view.mcp_skills = None;
                            view.mcp_skills_open = None;
                            view.mcp_skills_drag = None;
                            cx.notify();
                        },
                    ))
                    .child(
                        div()
                            .id("mcp-skills-close")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(20.))
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgb(palette.panel_soft)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.mcp_skills = None;
                                view.mcp_skills_open = None;
                                view.mcp_skills_drag = None;
                                cx.notify();
                            }))
                            .child(div().text_sm().text_color(rgb(palette.muted)).child("×")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .px_4()
                    .pb_3()
                    .child(
                        div()
                            .id("mcp-skills-scroll")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .max_h(px(MCP_SKILLS_BODY))
                            .overflow_y_scroll()
                            .track_scroll(&self.mcp_skills_scroll)
                            .children(rows),
                    )
                    .child(self.render_mcp_skills_bar(palette, cx)),
            )
            .into_any_element()
    }

    /// A thin scrollbar down the right edge of the skills list.
    ///
    /// gpui has no scrollbar element, so the thumb is worked out from the
    /// list's own scroll handle; the bar is a real column, not an overlay, so
    /// it never covers the parameter text.
    fn render_mcp_skills_bar(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some((track, thumb, max, _)) = tool_bar_metrics(&self.mcp_skills_scroll) else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-self.mcp_skills_scroll.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = self.mcp_skills_drag.is_some();
        let handle = self.mcp_skills_scroll.clone();
        div()
            .id("mcp-skills-bar")
            .w(px(MCP_SKILLS_BAR))
            .flex_shrink_0()
            .h(track + px(4.))
            .flex()
            .flex_col()
            .items_center()
            .block_mouse_except_scroll()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    let Some((track, thumb, max, top)) = tool_bar_metrics(&handle) else {
                        return;
                    };
                    let travel = (track - thumb).max(px(1.));
                    let progress = if max > px(0.) {
                        (-handle.offset().y / max).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let thumb_top = top + travel * progress;
                    let on_thumb =
                        event.position.y >= thumb_top && event.position.y <= thumb_top + thumb;
                    if !on_thumb {
                        // A track press jumps there; the drag carries on from
                        // the new place, so click-and-pull works in one gesture.
                        let ratio = ((event.position.y - top) / travel).clamp(0., 1.);
                        let current = handle.offset();
                        handle.set_offset(point(current.x, -max * ratio));
                    }
                    view.mcp_skills_drag = Some((event.position.y, handle.offset().y));
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

    /// Follow the pointer while the skills scrollbar is held.
    pub(crate) fn drag_mcp_skills_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((start_y, start_offset)) = self.mcp_skills_drag else {
            return;
        };
        let Some((track, thumb, max, _)) = tool_bar_metrics(&self.mcp_skills_scroll) else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = self.mcp_skills_scroll.offset();
        self.mcp_skills_scroll.set_offset(point(current.x, offset));
        cx.notify();
    }

    /// Repaint once the list has been laid out, so a scrollbar that was zero
    /// pixels tall on the opening frame gets a second chance to appear.
    fn spawn_mcp_skills_repaint(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(40))
                .await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Delete one server's manifest (or folder) and forget its switch.
    pub(crate) fn delete_mcp_server(&mut self, id: &str, cx: &mut Context<Self>) {
        self.mcp_skills = None;
        self.mcp_skills_open = None;
        self.mcp_skills_drag = None;
        if self
            .mcp_fetch
            .as_ref()
            .is_some_and(|(fetching, _, _)| fetching == id)
        {
            self.mcp_note = "正在获取技能，等它结束后再删除。".to_string();
            cx.notify();
            return;
        }
        let Some(server) = self
            .installed_mcp
            .iter()
            .find(|server| server.id == id)
            .cloned()
        else {
            self.mcp_note = format!("找不到 MCP 服务器「{id}」，已重新扫描。");
            self.refresh_mcp(cx);
            return;
        };
        match crate::mcp::delete_server(&self.mcp_dir, &server) {
            Ok(()) => {
                if self.ai.mcp_server_enabled(id) {
                    self.ai.set_mcp_server(id, false);
                    if let Err(error) = self.ai.save() {
                        self.banner = Some(Banner::error(error));
                    }
                }
                self.mcp_errors.retain(|(entry, _)| entry != id);
                self.open_mcp_cards.retain(|entry| entry != id);
                self.refresh_mcp(cx);
                self.mcp_note = format!("已删除 MCP 服务器「{}」", server.name);
                cx.notify();
            }
            Err(error) => {
                self.mcp_note = error;
                cx.notify();
            }
        }
    }
}

/// The empty page: where to put a manifest and what the folder looks like.
fn empty_state(dir: &Path, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.panel_soft))
        .child(
            div()
                .text_sm()
                .text_color(rgb(palette.text))
                .child("还没有链接 MCP 服务器"),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(
                    "在下面的文件夹里放一个 mcp.json（mcpServers 对象，和常见 MCP 客户端一致），\
                     或者为每个服务器建一个子文件夹放 mcp.json，然后点「重新扫描」。",
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .whitespace_normal()
                .child(dir.display().to_string()),
        )
        .into_any_element()
}

/// Wrap the cards into as many columns as the pane has room for.
fn mcp_grid(cards: Vec<AnyElement>) -> Div {
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

/// The corner arrow: down when closed, up when open.
fn mcp_arrow(
    slot: usize,
    open: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(("mcp-card-more", slot))
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

/// The "…" button at the card's top-right corner; opens the one-item menu.
fn mcp_menu_button(
    slot: usize,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, Point<Pixels>, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(("mcp-card-menu", slot))
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

/// The on/off switch on a server card.
fn mcp_switch(
    slot: usize,
    on: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(("mcp-switch", slot))
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

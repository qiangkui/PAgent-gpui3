//! The Skill page: prompt packages installed next to the program.
//!
//! A skill is a folder with `skill.json` (name, description, optional inline
//! instructions) and, usually, `SKILL.md`. Its switch decides whether the text
//! is injected into every question; the folder itself is read afresh each time
//! the page opens.

use super::*;

impl DshFlowApp {
    /// Skills: installed folders, their switches and their delete menu.
    pub(crate) fn render_skills(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let enabled = self
            .installed_skills
            .iter()
            .filter(|skill| self.ai.skill_enabled(&skill.id))
            .count();
        // Installed folders, plus any skill still switched on whose folder is
        // gone: leaving that one invisible would hide why its text vanished.
        let mut entries = self.installed_skills.clone();
        for id in &self.ai.skills {
            if !entries.iter().any(|skill| skill.id == *id) {
                entries.push(crate::skill::InstalledSkill {
                    id: id.clone(),
                    name: id.clone(),
                    description: String::new(),
                    instructions: String::new(),
                    problem: "技能文件夹不存在（仍标记为启用）".to_string(),
                });
            }
        }
        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut cards: Vec<AnyElement> = Vec::new();
        for (slot, skill) in entries.iter().enumerate() {
            let on = self.ai.skill_enabled(&skill.id);
            let failed = !skill.problem.is_empty();
            let toggle_id = skill.id.clone();
            // Every card keeps its switch, including a broken or missing folder:
            // that is how an id left switched on in `config.json` is turned off.
            let trailing = Some((
                switch(slot, on, palette, cx, move |view, cx| {
                    let want = !view.ai.skill_enabled(&toggle_id);
                    view.ai.set_skill(&toggle_id, want);
                    if let Err(error) = view.ai.save() {
                        view.banner = Some(Banner::error(error));
                    }
                    cx.notify();
                }),
                34.,
            ));
            let detail = if skill.description.is_empty() {
                format!(
                    "{} · {} 字指令",
                    skill.id,
                    skill.instructions.chars().count()
                )
            } else {
                format!(
                    "{} · {} · {} 字指令",
                    skill.id,
                    skill.description,
                    skill.instructions.chars().count()
                )
            };
            let (description, colour) = if failed {
                (format!("{}　{}", skill.id, skill.problem), palette.failed)
            } else {
                (detail, palette.muted)
            };
            let menu_id = skill.id.clone();
            let menu = plugin_menu_button(slot, palette, cx, move |view, position, cx| {
                let same = view
                    .skill_menu
                    .as_ref()
                    .is_some_and(|(id, _)| *id == menu_id);
                view.category_menu = None;
                view.card_menu = None;
                view.confirm_delete = None;
                view.mcp_menu = None;
                view.mcp_delete_confirm = None;
                view.mcp_skills = None;
                view.plugin_menu = None;
                view.plugin_delete_confirm = None;
                view.skill_delete_confirm = None;
                view.skill_menu = if same {
                    None
                } else {
                    Some((menu_id.clone(), position))
                };
                cx.notify();
            });
            // The plugin card is the same two-line card the plugin page uses;
            // the key is prefixed so a skill and a plugin with the same id do not
            // share their open/closed state.
            cards.push(self.plugin_card(
                &format!("skill:{}", skill.id),
                palette,
                cx,
                slot,
                failed,
                None,
                &skill.name,
                trailing,
                &description,
                colour,
                if failed {
                    palette.failed
                } else {
                    palette.border
                },
                Some(menu),
            ));
        }

        let mut body = div().flex().flex_col().gap_2().px_6().py_4().child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(format!(
                    "{} 个已安装 · {enabled} 个已启用",
                    self.installed_skills.len()
                )),
        );
        if !self.skill_note.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .whitespace_normal()
                    .child(self.skill_note.clone()),
            );
        }
        if cards.is_empty() {
            body = body.child(
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
                            .child("还没有技能"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .whitespace_normal()
                            .child(
                                "每个技能一个文件夹，里面放 skill.json（name / description，可选 instructions），\
                                 或再放一个 SKILL.md 写指令；然后点「重新扫描」并打开开关。",
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .whitespace_normal()
                            .child(self.skills_dir.display().to_string()),
                    ),
            );
        } else {
            body = body.child(grid(cards));
        }

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_w(px(0.))
            .child(page_title(
                "Skill",
                vec![
                    icon_button(
                        "skills-folder",
                        "icons/folder-open.svg",
                        palette,
                        cx,
                        |view, cx| {
                            let _ = std::fs::create_dir_all(&view.skills_dir);
                            let _ = open_path(&view.skills_dir);
                            view.refresh_skills(cx);
                        },
                    ),
                    icon_button(
                        "skills-rescan",
                        "icons/regenerate2.svg",
                        palette,
                        cx,
                        |view, cx| view.refresh_skills(cx),
                    ),
                    icon_button(
                        "skills-example",
                        "icons/plus.svg",
                        palette,
                        cx,
                        |view, cx| view.create_example_skill(cx),
                    ),
                ],
                palette,
            ))
            .child(
                div()
                    .id("skills-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(body),
            )
    }

    /// Read the skill folder again, when the page is opened or after an action.
    pub(crate) fn refresh_skills(&mut self, cx: &mut Context<Self>) {
        self.skills_dir =
            AiSettings::skill_dir().unwrap_or_else(|| self.ai.data_dir().join("skills"));
        let _ = std::fs::create_dir_all(&self.skills_dir);
        self.installed_skills = crate::skill::scanned_skills(&self.skills_dir);
        self.skill_menu = None;
        self.skill_delete_confirm = None;
        self.skill_note.clear();
        cx.notify();
    }

    /// Write the example skill into the folder.
    pub(crate) fn create_example_skill(&mut self, cx: &mut Context<Self>) {
        self.skills_dir =
            AiSettings::skill_dir().unwrap_or_else(|| self.ai.data_dir().join("skills"));
        match crate::skill::create_example() {
            Ok((folder, true)) => {
                self.skill_note = format!(
                    "已生成 {}：打开开关后，下一次提问会把它的指令一起交给助手。",
                    folder.display()
                );
            }
            Ok((folder, false)) => {
                let _ = open_path(&folder);
                self.skill_note = format!(
                    "{} 已存在，已打开它的文件夹（不会覆盖你改过的内容）。",
                    folder.display()
                );
            }
            Err(error) => self.skill_note = error,
        }
        self.installed_skills = crate::skill::scanned_skills(&self.skills_dir);
        cx.notify();
    }

    /// The one-item card menu: delete this skill folder from disk.
    pub(crate) fn render_skill_menu(
        &self,
        id: &str,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let armed = self.skill_delete_confirm.as_deref() == Some(id);
        let target = id.to_string();
        div()
            .id("skill-menu")
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
                    .id("skill-delete")
                    .px_3()
                    .py_1()
                    .rounded_sm()
                    .text_xs()
                    .text_color(rgb(palette.failed))
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        if view.skill_delete_confirm.as_deref() == Some(target.as_str()) {
                            view.skill_delete_confirm = None;
                            view.skill_menu = None;
                            view.delete_skill(&target, cx);
                        } else {
                            view.skill_delete_confirm = Some(target.clone());
                            cx.notify();
                        }
                    }))
                    .child(if armed {
                        "再点一次确认删除"
                    } else {
                        "删除技能"
                    }),
            )
            .into_any_element()
    }

    /// Delete one skill folder and forget its switch.
    pub(crate) fn delete_skill(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(error) = crate::skill::delete_skill(id) {
            self.skill_note = error;
            cx.notify();
            return;
        }
        if self.ai.skill_enabled(id) {
            self.ai.set_skill(id, false);
            if let Err(error) = self.ai.save() {
                self.banner = Some(Banner::error(error));
            }
        }
        self.open_plugin_cards
            .retain(|entry| entry != &format!("skill:{id}"));
        self.installed_skills = crate::skill::scanned_skills(&self.skills_dir);
        self.skill_menu = None;
        self.skill_delete_confirm = None;
        // The card disappearing says it; no success line is needed.
        self.skill_note.clear();
        cx.notify();
    }
}

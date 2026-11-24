//! The window: root layout, view dispatch, and the first-frame window fit.

use super::*;

impl DshFlowApp {
    /// The right-hand pane, by view.
    pub(crate) fn render_main(
        &self,
        palette: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match self.view {
            View::Chat => self.render_chat(palette, window, cx),
            View::Flow => self.render_flow(palette, cx),
            View::Plugins => self.render_plugins(palette, cx),
            View::Mcp => self.render_mcp(palette, cx),
            View::Skills => self.render_skills(palette, cx),
            View::Stats => self.render_stats(palette, cx),
        }
    }
}

impl Render for DshFlowApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.fitted {
            self.fitted = true;
            fit_once(window, cx);
            // A restored conversation opens on its newest message: the transcript
            // has no size until the first layout, so the jump has to wait for it.
            self.scroll_chat_to_newest();
        }

        let palette = self.palette();
        // The OS title (taskbar, Alt+Tab) follows the conversation; the drawn title
        // bar stays the brand. Only set it when it changes.
        let want_title = {
            let chat = self.chat();
            format!("PAgent · {}", header_title(&chat.title))
        };
        if self.window_title != want_title {
            self.window_title = want_title.clone();
            window.set_window_title(&want_title);
        }
        let viewport = window.viewport_size();
        self.scale_factor = window.scale_factor();
        // The pane is what the sidebar and its handle leave. It is measured
        // again every frame, so a resized window or a dragged splitter gives the
        // answers the room they now have.
        let pane = px(f32::from(viewport.width) - self.sidebar_width - SPLITTER_WIDTH);
        self.answer_width = answer_text_width(pane);
        self.chat_gutter = transcript_pad(pane);
        let mut root = div()
            .relative()
            .flex()
            .flex_col()
            .w(viewport.width)
            .h(viewport.height)
            // The window behind everything: only the rounded corner of the
            // main panel lets it show, and it matches the chrome.
            .bg(rgb(palette.chrome))
            .text_color(rgb(palette.text))
            .on_key_down(cx.listener(|view, event, window, cx| {
                view.handle_shortcut(event, window, cx);
            }))
            // A click that never reaches a text field drops the caret: the fields
            // stop propagation on their own mouse-down, so whatever lands here is
            // "outside" every input (blank sidebar, a card, the page background).
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    let mut changed = false;
                    if view.focused.is_some() {
                        view.leave_fields(window, cx);
                        changed = true;
                    }
                    // Any press outside a card puts its menu away; the card and
                    // its items stop propagation.
                    if view.category_menu.take().is_some() {
                        changed = true;
                    }
                    if view.card_menu.take().is_some() {
                        changed = true;
                    }
                    // The MCP card menu is a one-item dropdown: a press outside
                    // folds it away and forgets a pending delete.
                    if view.mcp_menu.take().is_some() {
                        view.mcp_delete_confirm = None;
                        changed = true;
                    }
                    // The skills panel is also a dropdown: a press outside it
                    // folds it away.
                    if view.mcp_skills.take().is_some() {
                        view.mcp_skills_open = None;
                        view.mcp_skills_drag = None;
                        changed = true;
                    }
                    // The external plugin card menu works the same way.
                    if view.plugin_menu.take().is_some() {
                        view.plugin_delete_confirm = None;
                        changed = true;
                    }
                    // The skill card menu is a one-item dropdown as well.
                    if view.skill_menu.take().is_some() {
                        view.skill_delete_confirm = None;
                        changed = true;
                    }
                    // The round picker in the 上下文 tab is a dropdown too.
                    if view.context_round_menu.take().is_some() {
                        changed = true;
                    }
                    // The composer's pickers close on a press outside them; the
                    // chips stop propagation, so this never toggles one twice.
                    if view.model_menu || view.tools_menu || view.budget_menu || view.attach_menu {
                        view.close_composer_menus();
                        changed = true;
                    }
                    // Closing the menu also disarms a pending delete.
                    if view.confirm_delete.take().is_some() {
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                }),
            )
            .child(self.render_titlebar(palette))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .track_focus(&self.content_focus)
                    .child(self.render_sidebar(palette, cx))
                    .child(splitter(palette, cx))
                    .child(
                        // One surface for every view: pure white in the light
                        // theme, with only the top-left corner rounded so the
                        // sidebar still reads as chrome.
                        div()
                            .id("main-panel")
                            .flex()
                            .flex_col()
                            .flex_1()
                            .h_full()
                            .min_w(px(0.))
                            .bg(rgb(palette.canvas))
                            .rounded_tl(px(PANEL_RADIUS))
                            .child(self.render_main(palette, window, cx)),
                    ),
            );

        // While the splitter is held, a transparent sheet takes every mouse move so
        // the pointer can leave the 6px handle without ending the drag.
        if self.resizing_sidebar {
            root = root.child(
                div()
                    .id("splitter-drag")
                    .absolute()
                    .top(px(0.))
                    .left(px(0.))
                    .w_full()
                    .h_full()
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                        view.set_sidebar_width(f32::from(event.position.x), cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.resizing_sidebar = false;
                            // The width is worth remembering; the drag itself is
                            // not the moment to write the file, its end is.
                            view.save_categories();
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.resizing_sidebar = false;
                            view.save_categories();
                            cx.notify();
                        }),
                    ),
            );
        }

        // While a scrollbar is held the pointer leaves the 8px handle at once, so
        // a transparent sheet over the content keeps the moves coming. Both bars
        // share it: only one of them can be held at a time.
        if self.scroll_drag.is_some()
            || self.tool_drag.is_some()
            || self.mcp_skills_drag.is_some()
            || self.totals_drag.is_some()
            || self.table_drag.is_some()
        {
            root = root.child(
                div()
                    .id("scrollbar-drag")
                    .absolute()
                    .top(px(TITLEBAR_HEIGHT))
                    .left(px(0.))
                    .w_full()
                    .h(viewport.height - px(TITLEBAR_HEIGHT))
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _, cx| {
                        if view.tool_drag.is_some() {
                            view.drag_tool_bar(event.position.y, cx);
                        } else if view.mcp_skills_drag.is_some() {
                            view.drag_mcp_skills_bar(event.position.y, cx);
                        } else if view.totals_drag.is_some() {
                            view.drag_totals_bar(event.position.y, cx);
                        } else if view.table_drag.is_some() {
                            view.drag_table_bar(event.position.y, cx);
                        } else {
                            view.drag_scrollbar(event.position.y, cx);
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.scroll_drag = None;
                            view.tool_drag = None;
                            view.mcp_skills_drag = None;
                            view.totals_drag = None;
                            view.table_drag = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            view.scroll_drag = None;
                            view.tool_drag = None;
                            view.mcp_skills_drag = None;
                            view.totals_drag = None;
                            view.table_drag = None;
                            cx.notify();
                        }),
                    ),
            );
        }

        if let Some((id, position)) = self.category_menu.clone() {
            let at = point(
                position.x.min(viewport.width - px(120.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(70.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_category_menu(&id, at, palette, cx));
        }

        if let Some((menu, position)) = self.card_menu {
            let at = point(
                position.x.min(viewport.width - px(156.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(200.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_card_menu(menu, at, palette, cx));
        }

        if let Some((id, position)) = self.mcp_menu.clone() {
            let at = point(
                position.x.min(viewport.width - px(176.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(80.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_mcp_menu(&id, at, palette, cx));
        }

        if let Some((id, position)) = self.mcp_skills.clone() {
            let at = point(
                position
                    .x
                    .min(viewport.width - px(MCP_SKILLS_WIDTH + 16.))
                    .max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(MCP_SKILLS_HEIGHT + 16.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_mcp_skills(&id, at, palette, cx));
        }

        if let Some((id, position)) = self.plugin_menu.clone() {
            let at = point(
                position.x.min(viewport.width - px(176.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(80.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_plugin_menu(&id, at, palette, cx));
        }

        if let Some((id, position)) = self.skill_menu.clone() {
            let at = point(
                position.x.min(viewport.width - px(176.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(80.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_skill_menu(&id, at, palette, cx));
        }

        if let Some(position) = self.context_round_menu {
            let at = point(
                position.x.min(viewport.width - px(316.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(376.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            root = root.child(self.render_context_round_menu(at, palette, cx));
        }

        if let Some((slot, position)) = self.field_menu {
            let at = point(
                position.x.min(viewport.width - px(130.)).max(px(0.)),
                position
                    .y
                    .min(viewport.height - px(100.))
                    .max(px(TITLEBAR_HEIGHT)),
            );
            // The sheet closes the menu on a click anywhere below the title bar;
            // it deliberately does not cover the title bar, whose drag and window
            // buttons belong to the platform.
            root = root
                .child(
                    div()
                        .id("menu-catcher")
                        .absolute()
                        .top(px(TITLEBAR_HEIGHT))
                        .left(px(0.))
                        .w_full()
                        .h(viewport.height - px(TITLEBAR_HEIGHT))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, event: &gpui::MouseDownEvent, _, cx| {
                                if let Some((_, at)) = view.field_menu {
                                    let card = Bounds::new(
                                        at,
                                        size(px(FIELD_MENU_WIDTH), px(FIELD_MENU_HEIGHT)),
                                    );
                                    if card.contains(&event.position) {
                                        // The menu keeps the click and runs its item.
                                        return;
                                    }
                                }
                                view.field_menu = None;
                                cx.notify();
                            }),
                        ),
                )
                .child(self.render_field_menu(slot, at, palette, cx));
        }

        if self.renaming.is_some() {
            root = root.child(self.render_rename_dialog(viewport.height, palette, cx));
        }

        root
    }
}

/// Keep the window inside the display.
///
/// A window is created at the size it asks for in device pixels and only then
/// does the platform apply the display's scale factor, so on a scaled display it
/// can end up larger than the screen and its bottom, where the composer sits,
/// becomes unreachable. Shrinking it to the largest size that still fits where it
/// already is keeps the whole layout on screen.
pub(crate) fn fit_once(window: &mut Window, cx: &App) {
    let scale = window.scale_factor().max(0.5);
    let Some(display) = window.display(cx) else {
        return;
    };
    let available = display.bounds().size;
    let screen_width: f32 = available.width.into();
    let screen_height: f32 = available.height.into();

    let current = window.bounds();
    let origin_x: f32 = current.origin.x.into();
    let origin_y: f32 = current.origin.y.into();
    let width: f32 = current.size.width.into();
    let height: f32 = current.size.height.into();

    if (origin_x + width) * scale <= screen_width && (origin_y + height) * scale <= screen_height {
        return;
    }

    let room_width = screen_width / scale * 0.97 - origin_x;
    let room_height = screen_height / scale * 0.97 - origin_y;
    window.resize(size(px(width.min(room_width)), px(height.min(room_height))));
}

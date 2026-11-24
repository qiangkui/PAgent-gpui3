//! Text fields: layout, caret, selection, editing keys, and the right-click menu.

use super::*;

/// Space the box adds above the first line of a multi-line field (`py_1`).
const BLOCK_PAD_Y: f32 = 4.;

/// Where the 14px caret sits inside its 20px line.
const CARET_INSET_Y: f32 = 3.;

/// True when a field is drawn as a growing block of lines.
///
/// Everything that turns pixels into a character index, or a caret into a
/// position, has to agree with the layout about this.
pub(crate) fn field_multiline(slot: Field) -> bool {
    matches!(slot, Field::ChatInstructions)
}

/// Where the text starts inside a field, in pixels below its top edge.
///
/// A block of lines starts under the box's own top padding; a one-line field
/// centres its text in the row, which is also what makes a pasted multi-line
/// draft in the composer sit in the middle of its card.
pub(crate) fn field_text_top(box_height: f32, lines: usize, block: bool) -> f32 {
    if block {
        return BLOCK_PAD_Y;
    }
    let height = COMPOSER_LINE_HEIGHT * lines.max(1) as f32;
    ((box_height - height) / 2.).max(0.)
}

/// Which line a point `offset` pixels below a field's top lands on.
///
/// Clamped so a click in the padding still picks the nearest line.
pub(crate) fn field_row_at(offset: f32, box_height: f32, lines: usize, block: bool) -> usize {
    let count = lines.max(1);
    let top = field_text_top(box_height, count, block);
    let row = ((offset - top) / COMPOSER_LINE_HEIGHT).max(0.) as usize;
    row.min(count - 1)
}

impl DshFlowApp {
    /// What the caret painter needs: the text as drawn, the caret offset in it, and
    /// the colour of the bar.
    ///
    /// A key field draws dots, so the offset is measured in that string instead.
    pub(crate) fn field_caret(&self, slot: Field) -> Option<(String, usize, Hsla)> {
        let field = self.field(slot)?;
        let masked = matches!(slot, Field::ConnKey(_)) && !field.text.is_empty();
        let text = if masked {
            "\u{2022}".repeat(field.text.chars().count())
        } else {
            field.text.clone()
        };
        let caret = if masked {
            text.len()
        } else {
            field.caret_byte().min(text.len())
        };
        let colour: Hsla = rgb(self.palette().accent).into();
        Some((text, caret, colour))
    }

    /// The text field for a slot, for reading.
    pub(crate) fn field(&self, slot: Field) -> Option<&TextField> {
        match slot {
            Field::Composer => Some(&self.composer),
            Field::Input(index) => self.inputs.get(index).map(|input| &input.field),
            Field::ConnModel(index) => self.conn_fields.get(index).map(|fields| &fields.model),
            Field::ConnKey(index) => self.conn_fields.get(index).map(|fields| &fields.api_key),
            Field::ConnUrl(index) => self.conn_fields.get(index).map(|fields| &fields.base_url),
            Field::ChatFilter => Some(&self.chat_filter),
            Field::ChatFind => Some(&self.chat_find),
            Field::ChatTitle => self.renaming.as_ref().map(|(_, field)| field),
            Field::NewCategory => self.new_category.as_ref(),
            Field::CategoryName => self.rename_category.as_ref().map(|(_, field)| field),
            Field::ChatInstructions => Some(&self.instructions),
            Field::ColorHex(index) => self.color_fields.get(index),
            Field::Proxy => Some(&self.proxy_field),
        }
    }

    /// The text field for a slot, for editing.
    pub(crate) fn field_mut(&mut self, slot: Field) -> Option<&mut TextField> {
        match slot {
            Field::Composer => Some(&mut self.composer),
            Field::Input(index) => self.inputs.get_mut(index).map(|input| &mut input.field),
            Field::ConnModel(index) => self
                .conn_fields
                .get_mut(index)
                .map(|fields| &mut fields.model),
            Field::ConnKey(index) => self
                .conn_fields
                .get_mut(index)
                .map(|fields| &mut fields.api_key),
            Field::ConnUrl(index) => self
                .conn_fields
                .get_mut(index)
                .map(|fields| &mut fields.base_url),
            Field::ChatFilter => Some(&mut self.chat_filter),
            Field::ChatFind => Some(&mut self.chat_find),
            Field::ChatTitle => self.renaming.as_mut().map(|(_, field)| field),
            Field::NewCategory => self.new_category.as_mut(),
            Field::CategoryName => self.rename_category.as_mut().map(|(_, field)| field),
            Field::ChatInstructions => Some(&mut self.instructions),
            Field::ColorHex(index) => self.color_fields.get_mut(index),
            Field::Proxy => Some(&mut self.proxy_field),
        }
    }

    /// The byte range an IME edit replaces: the reported one, else the
    /// composition, else the caret.
    pub(crate) fn input_target(&self, range: Option<Range<usize>>) -> Option<Range<usize>> {
        let field = self.field(self.focused?)?;
        Some(match range {
            Some(range) => field.byte_range(range),
            None => field.marked().unwrap_or_else(|| {
                let caret = field.caret_byte();
                caret..caret
            }),
        })
    }

    /// Apply one key press to the focused text field.
    pub(crate) fn handle_field_key(
        &mut self,
        slot: Field,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.caret_on = true;
        // An empty composer turns the arrows into history: everything else
        // (typing, deleting, Enter) ends a recall, so the next arrow moves the
        // caret again instead of replacing typed text.
        if slot == Field::Composer {
            let keys = &event.keystroke.modifiers;
            let plain =
                !keys.control && !keys.platform && !keys.alt && !keys.shift && !keys.function;
            if plain && event.keystroke.key == "up" && self.recall_question(true, cx) {
                cx.stop_propagation();
                return;
            }
            if plain && event.keystroke.key == "down" && self.recall_question(false, cx) {
                cx.stop_propagation();
                return;
            }
            self.composer_recall = None;
        }
        let Some(field) = self.field_mut(slot) else {
            return;
        };
        let key = field.handle_key(event);
        self.ensure_caret_scroll(slot, window);
        self.commit_edit(slot, cx);
        match key {
            FieldKey::Handled => {
                cx.stop_propagation();
                cx.notify();
            }
            FieldKey::Submit => {
                cx.stop_propagation();
                // While an IME is composing, Enter belongs to the candidate list;
                // sending here would fire off half-typed pinyin.
                if self
                    .field(slot)
                    .is_some_and(|field| field.marked().is_some())
                {
                    return;
                }
                if slot == Field::Composer {
                    self.send_message(cx);
                } else if slot == Field::ChatFind {
                    let step = if event.keystroke.modifiers.shift {
                        -1
                    } else {
                        1
                    };
                    self.search_chat(step, cx);
                } else if slot == Field::ChatTitle {
                    self.finish_rename(true, window, cx);
                } else if slot == Field::NewCategory {
                    self.commit_new_category();
                    cx.notify();
                } else if slot == Field::CategoryName {
                    self.commit_category_rename();
                    self.focused = None;
                    cx.notify();
                } else if slot == Field::ChatInstructions {
                    // Enter breaks the line here; the save button commits.
                    if let Some(field) = self.field_mut(slot) {
                        field.insert("\n");
                    }
                    self.ensure_caret_scroll(slot, window);
                    cx.notify();
                } else {
                    cx.notify();
                }
            }
            FieldKey::Paste => {
                self.field_paste(slot, cx);
                self.ensure_caret_scroll(slot, window);
                cx.stop_propagation();
            }
            key @ (FieldKey::Copy | FieldKey::Cut) => {
                // With nothing selected in the field the key belongs to the
                // transcript selection, so it is left to bubble.
                let done = if key == FieldKey::Cut {
                    self.field_cut(slot, cx)
                } else {
                    self.field_copy(slot, cx)
                };
                if !done {
                    return;
                }
                cx.stop_propagation();
            }
            FieldKey::Escape => {
                // A composition owns Escape: cancelling it must not drop the caret.
                if self
                    .field(slot)
                    .is_some_and(|field| field.marked().is_some())
                {
                    return;
                }
                // Escape in the search box empties it first, so one key press
                // brings the whole list back; in a title it drops the edit.
                if slot == Field::ChatFilter {
                    self.chat_filter.clear();
                } else if slot == Field::ChatFind {
                    self.chat_find.clear();
                    self.chat_find_open = false;
                } else if slot == Field::ChatTitle {
                    self.cancel_rename();
                } else if slot == Field::NewCategory {
                    self.cancel_new_category(cx);
                } else if slot == Field::CategoryName {
                    self.cancel_category_rename(cx);
                }
                if let Some(field) = self.field_mut(slot) {
                    field.deselect();
                }
                self.focused = None;
                window.focus(&self.content_focus, cx);
                cx.stop_propagation();
                cx.notify();
            }
            FieldKey::Ignored => {}
        }
    }

    /// Window-level shortcuts, reached by bubbling key presses.
    pub(crate) fn handle_shortcut(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let control = keystroke.modifiers.control;
        match keystroke.key.as_str() {
            "n" if control => {
                self.new_chat(window, cx);
                cx.stop_propagation();
            }
            "t" if control => {
                self.toggle_theme(cx);
                cx.stop_propagation();
                cx.notify();
            }
            "f" if control => {
                if self.view == View::Chat && self.renaming.is_none() {
                    self.start_chat_find(window, cx);
                    cx.stop_propagation();
                }
            }
            "r" if control => {
                if self.view == View::Flow {
                    self.start_run(cx);
                    cx.stop_propagation();
                }
            }
            "l" if control => {
                if self.view == View::Flow {
                    self.clear_log();
                    cx.stop_propagation();
                    cx.notify();
                }
            }
            "pageup" => {
                self.scroll_transcript(-0.9, cx);
                cx.stop_propagation();
            }
            "pagedown" => {
                self.scroll_transcript(0.9, cx);
                cx.stop_propagation();
            }
            "home" if control => {
                self.scroll_transcript_to(false, cx);
                cx.stop_propagation();
            }
            "end" if control => {
                self.scroll_transcript_to(true, cx);
                cx.stop_propagation();
            }
            "c" if control => {
                if self.copy_selected(cx) {
                    cx.stop_propagation();
                }
            }
            "a" if control => {
                // A field owns `Ctrl+A` while it has the keyboard; with nothing
                // focused it takes the whole conversation.
                if self.focused.is_none() && self.select_whole_transcript(cx) {
                    cx.stop_propagation();
                }
            }
            "escape" => {
                if self.field_menu.is_some() {
                    self.field_menu = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if self.renaming.is_some() {
                    // The dialog owns Escape even when its field lost the keyboard.
                    self.finish_rename(false, window, cx);
                    cx.stop_propagation();
                    return;
                }
                if self.selected_text.is_some() {
                    // A selection is the innermost thing Escape can cancel, and it
                    // costs nothing to drop.
                    self.clear_selection(cx);
                } else if self.confirm_delete.is_some() {
                    self.confirm_delete = None;
                } else if self.model_menu {
                    self.model_menu = false;
                } else if self.tools_menu {
                    self.tools_menu = false;
                } else if self.budget_menu {
                    self.budget_menu = false;
                } else if self.attach_menu {
                    self.attach_menu = false;
                } else if self.chat_find_open {
                    self.close_chat_find(window, cx);
                } else if self.shown_chat_tab() == ChatTab::Tasks
                    || self.shown_chat_tab() == ChatTab::Context
                {
                    // Both are places you went to; Escape goes back.
                    self.show_chat_tab(ChatTab::Chat, cx);
                } else if self.chat_busy(self.active_chat) {
                    // Same path as the stop button, so the written text survives.
                    self.cancel_answer(cx);
                } else if self.active.is_some() {
                    self.cancel_run(cx);
                } else if self.view != View::Chat {
                    // Nothing left to cancel: leave the page for the conversation.
                    self.view = View::Chat;
                }
                cx.notify();
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// One text field with its caret.
    ///
    /// `boxed` draws the field's own frame; the floating composer turns it off so
    /// the card around it is the only surface.
    pub(crate) fn render_field(
        &self,
        slot: Field,
        placeholder: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> FieldElement {
        let chrome = slot.chrome();
        let Some(field) = self.field(slot) else {
            return FieldElement {
                inner: div().id(("field", field_slot(slot))).into_any_element(),
                focus: self.content_focus.clone(),
                view: cx.entity(),
                slot,
                bounds: self.field_bounds.clone(),
            };
        };
        let focused = self.focused == Some(slot);
        let selection = field.selection();
        let key = field_slot(slot);
        let boxed = chrome == FieldChrome::Boxed;
        // The instruction, allowlist and colour editors grow with their text
        // instead of staying one line.
        let multiline = field_multiline(slot);
        let mut row = div()
            .id(("field", key))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .when(boxed && !multiline, |this| this.h(px(30.)))
            .when(chrome == FieldChrome::Plain || multiline, |this| {
                this.min_h(px(34.))
            })
            .when(chrome == FieldChrome::Row, |this| this.h(px(22.)))
            .when(multiline, |this| this.items_start().py_1())
            .when(chrome != FieldChrome::Row, |this| this.px_3())
            .rounded_md()
            .overflow_hidden()
            .cursor_text()
            .when(boxed, |this| {
                this.bg(rgb(palette.field))
                    .border_1()
                    .border_color(rgb(if focused {
                        palette.accent
                    } else {
                        palette.border
                    }))
            })
            .track_focus(field.focus_handle())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                    view.focused = Some(slot);
                    if let Some(field) = view.field(slot) {
                        window.focus(&field.focus_handle().clone(), cx);
                    }
                    view.start_caret_blink(cx);
                    // gpui has no caret hit-testing, so the click is measured
                    // against the field's own bounds and the text is shaped here.
                    if let Some(index) = view.field_index_at(slot, event.position, window) {
                        let double = event.click_count >= 2;
                        if let Some(field) = view.field_mut(slot) {
                            if double {
                                field.select_word_at(index);
                            } else {
                                field.caret_from_click(index);
                            }
                        }
                        // A double click already made a selection; a drag would
                        // throw it away, so only single clicks start one.
                        view.field_drag = (!double).then_some(slot);
                        view.ensure_caret_scroll(slot, window);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |view, event: &gpui::MouseDownEvent, window, cx| {
                    view.focused = Some(slot);
                    if let Some(field) = view.field(slot) {
                        window.focus(&field.focus_handle().clone(), cx);
                    }
                    view.start_caret_blink(cx);
                    view.field_menu = Some((slot, event.position));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(
                cx.listener(move |view, event: &MouseMoveEvent, window, cx| {
                    if view.field_drag != Some(slot)
                        || event.pressed_button != Some(MouseButton::Left)
                    {
                        return;
                    }
                    if let Some(index) = view.field_index_at(slot, event.position, window) {
                        if let Some(field) = view.field_mut(slot) {
                            field.extend_to(index);
                        }
                        view.ensure_caret_scroll(slot, window);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| {
                    if view.field_drag != Some(slot) {
                        return;
                    }
                    view.field_drag = None;
                    if let Some(field) = view.field_mut(slot) {
                        field.end_drag();
                    }
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |view, event, window, cx| {
                view.handle_field_key(slot, event, window, cx);
            }));

        if matches!(slot, Field::ConnKey(_)) && !field.text.is_empty() {
            // The key is drawn as dots: the caret position is not shown, but the
            // field still edits like any other.
            row = row.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child("•".repeat(field.text.chars().count())),
            );
        } else if field.text.is_empty() {
            // The search row's hint doubles as its own label: it is drawn in the
            // normal ink, and it steps aside the moment the caret arrives (a
            // focused, empty search shows only the caret). Other rows — naming a
            // new category, for one — keep their hint while focused.
            let row_chrome = chrome == FieldChrome::Row;
            let hint_hidden = row_chrome && focused && slot == Field::ChatFilter;
            if !hint_hidden {
                row = row.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if row_chrome {
                            palette.text
                        } else {
                            palette.faint
                        }))
                        .child(placeholder.to_string()),
                );
            }
        } else if focused || selection.is_some() || multiline {
            // The text is cut into spans only where a selection or an IME
            // composition has to be coloured, never at the caret.
            row = row.child(field_spans(field, selection, palette));
        } else {
            row = row.child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child(field.text.clone()),
            );
        }

        FieldElement {
            inner: row.into_any_element(),
            focus: field.focus_handle().clone(),
            view: cx.entity(),
            slot,
            bounds: self.field_bounds.clone(),
        }
    }

    /// Copy a field's selection; false when nothing is selected.
    pub(crate) fn field_copy(&mut self, slot: Field, cx: &mut Context<Self>) -> bool {
        let Some(text) = self.field(slot).and_then(|field| field.selection_text()) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        cx.notify();
        true
    }

    /// Cut a field's selection; false when nothing is selected.
    pub(crate) fn field_cut(&mut self, slot: Field, cx: &mut Context<Self>) -> bool {
        if !self.field_copy(slot, cx) {
            return false;
        }
        if let Some(field) = self.field_mut(slot) {
            field.cut();
        }
        self.commit_edit(slot, cx);
        cx.notify();
        true
    }

    /// Paste the clipboard into a field.
    pub(crate) fn field_paste(&mut self, slot: Field, cx: &mut Context<Self>) {
        let raw = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        // The composer keeps line breaks; every other field is one line.
        let pasted = clean_paste(
            &raw,
            matches!(slot, Field::Composer | Field::ChatInstructions),
        );
        if !pasted.is_empty() {
            if let Some(field) = self.field_mut(slot) {
                field.insert(&pasted);
            }
            self.commit_edit(slot, cx);
            cx.notify();
            return;
        }
        // No text on the clipboard: a screenshot can still go in as an image.
        if slot == Field::Composer
            && let Some(bytes) = crate::clipboard_image::png_from_clipboard()
        {
            self.attach_image(bytes, cx);
        }
    }

    /// Select everything in a field.
    pub(crate) fn field_select_all(&mut self, slot: Field, cx: &mut Context<Self>) {
        if let Some(field) = self.field_mut(slot) {
            field.select_all();
        }
        cx.notify();
    }

    /// Move the field's horizontal scroll so the caret stays inside the visible
    /// part of a long line.
    ///
    /// Only one-line fields scroll: in a block of lines the shift would slide
    /// every line sideways, so a long line there is clipped instead.
    pub(crate) fn ensure_caret_scroll(&mut self, slot: Field, window: &mut Window) {
        if field_multiline(slot) {
            return;
        }
        let Some(bounds) = self
            .field_bounds
            .borrow()
            .iter()
            .find(|(key, _)| *key == field_slot(slot))
            .map(|(_, bounds)| *bounds)
        else {
            return;
        };
        let Some((text, caret, _)) = self.field_caret(slot) else {
            return;
        };
        let (caret_x, line_width) = caret_metrics(window, &text, caret);
        let visible = (bounds.size.width - slot.pad() * 2.).max(px(24.));
        let current = self.field(slot).map(|field| field.scroll()).unwrap_or(0.);

        let caret_x = f32::from(caret_x);
        let visible = f32::from(visible);
        let mut scroll = current;
        if caret_x - scroll > visible {
            scroll = caret_x - visible;
        }
        if caret_x < scroll {
            scroll = caret_x;
        }
        let max = (f32::from(line_width) - visible).max(0.);
        scroll = scroll.clamp(0., max);
        if let Some(field) = self.field_mut(slot) {
            field.set_scroll(scroll);
        }
    }

    /// The field's right-click menu: cut, copy, paste, select all.
    ///
    /// gpui has no context menu of its own, so it is a small floating card; the
    /// caller places it where the click was and dims nothing behind it.
    pub(crate) fn render_field_menu(
        &self,
        slot: Field,
        at: Point<Pixels>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Cut and copy need a selection, select-all needs some text; paste is left
        // alone because the clipboard is not something the view can inspect here.
        let has_selection = self
            .field(slot)
            .and_then(|field| field.selection())
            .is_some();
        let has_text = self.field(slot).is_some_and(|field| !field.text.is_empty());

        let item = |id: &'static str,
                    label: &'static str,
                    action: &'static str,
                    enabled: bool,
                    cx: &mut Context<Self>|
         -> AnyElement {
            let mut item = div()
                .id(id)
                .px_3()
                .py(px(1.))
                .rounded_sm()
                .text_xs()
                .text_color(rgb(if enabled { palette.text } else { palette.faint }));
            if enabled {
                let action = action.to_string();
                item = item
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        match action.as_str() {
                            "cut" => {
                                view.field_cut(slot, cx);
                            }
                            "copy" => {
                                view.field_copy(slot, cx);
                            }
                            "paste" => {
                                view.field_paste(slot, cx);
                                view.ensure_caret_scroll(slot, window);
                            }
                            _ => {
                                view.field_select_all(slot, cx);
                                view.ensure_caret_scroll(slot, window);
                            }
                        }
                        view.field_menu = None;
                        cx.notify();
                    }));
            }
            item.child(label).into_any_element()
        };

        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .flex()
            .flex_col()
            .gap(px(1.))
            .p_1()
            .w(px(FIELD_MENU_WIDTH))
            .rounded_md()
            .bg(rgb(palette.panel))
            .border_1()
            .border_color(rgb(palette.border))
            .shadow_md()
            .child(item("menu-cut", "剪切", "cut", has_selection, cx))
            .child(item("menu-copy", "复制", "copy", has_selection, cx))
            .child(item("menu-paste", "粘贴", "paste", true, cx))
            .child(item("menu-select-all", "全选", "select-all", has_text, cx))
            .into_any_element()
    }

    /// The character index under a pointer position inside a field.
    ///
    /// The field is drawn with plain divs, so there is no layout to ask; the text
    /// is shaped again here and the pointer is measured from the field's own
    /// bounds, line by line.
    pub(crate) fn field_index_at(
        &self,
        slot: Field,
        position: Point<Pixels>,
        window: &mut Window,
    ) -> Option<usize> {
        let bounds = self
            .field_bounds
            .borrow()
            .iter()
            .find(|(key, _)| *key == field_slot(slot))
            .map(|(_, bounds)| *bounds)?;
        let field = self.field(slot)?;

        let lines: Vec<&str> = field.text.split('\n').collect();
        let block = field_multiline(slot);
        let offset = f32::from(position.y - bounds.top());
        let row = field_row_at(offset, f32::from(bounds.size.height), lines.len(), block);
        let start: usize = lines.iter().take(row).map(|line| line.len() + 1).sum();
        let line = lines.get(row).copied().unwrap_or_default();

        let runs = vec![gpui::TextRun {
            len: line.len(),
            font: gpui::font(".SystemUIFont"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let shaped = window.text_system().shape_line(
            line.to_string().into(),
            px(FIELD_FONT_SIZE),
            &runs,
            None,
        );
        // The row's padding is where the text starts, and the text may be
        // scrolled left to keep a long line's caret in view.
        let scroll = px(field.scroll());
        let x = (position.x - bounds.left() - slot.pad() + scroll).max(px(0.));
        Some(start + shaped.closest_index_for_x(x))
    }
}

/// Wraps a field's row so the platform gets an input handler while it paints.
///
/// `Window::handle_input` may only be called during paint, and only an element
/// paints, so the row is wrapped instead of returned directly. Without it the
/// IME has nowhere to deliver a composition and typed CJK text never arrives.
pub(crate) struct FieldElement {
    inner: AnyElement,
    focus: FocusHandle,
    view: Entity<DshFlowApp>,
    /// Slot this field belongs to, so its bounds (and its caret) can be drawn.
    slot: Field,
    /// Bounds of every field drawn last frame, used to turn a click into a
    /// character index (there is no text hit-testing in gpui-pre 0.3.14).
    bounds: Rc<RefCell<Vec<(usize, Bounds<Pixels>)>>>,
}

impl Element for FieldElement {
    type RequestLayoutState = <AnyElement as Element>::RequestLayoutState;
    type PrepaintState = <AnyElement as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.inner)
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        self.inner.source_location()
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        Element::request_layout(&mut self.inner, id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        Element::prepaint(
            &mut self.inner,
            id,
            inspector_id,
            bounds,
            request_layout,
            window,
            cx,
        )
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        Element::paint(
            &mut self.inner,
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
        let key = field_slot(self.slot);
        {
            let mut known = self.bounds.borrow_mut();
            match known.iter_mut().find(|(slot, _)| *slot == key) {
                Some(entry) => entry.1 = bounds,
                None => known.push((key, bounds)),
            }
        }
        // The caret is painted over the line, never inside it: an element in the
        // flow (or a text run split at the caret) makes the rest of the line
        // re-render, so every arrow key would visibly reshuffle the text.
        {
            let app = self.view.read(cx);
            if app.focused == Some(self.slot)
                && app.caret_on
                && let Some((text, caret, colour)) = app.field_caret(self.slot)
            {
                let line_index = text[..caret].matches('\n').count();
                let scroll = app
                    .field(self.slot)
                    .map(|field| field.scroll())
                    .unwrap_or(0.);
                let (caret_x, _) = caret_metrics(window, &text, caret);
                let x = (caret_x - px(scroll)).max(px(0.));
                // A block of lines starts under the box's top padding; a
                // one-line field centres its line in the row. Measuring the
                // same way the layout does is what keeps a multi-line caret
                // on its own line.
                let lines = text.split('\n').count().max(1);
                let top = f32::from(bounds.size.height);
                let text_top = field_text_top(top, lines, field_multiline(self.slot));
                let origin = point(
                    bounds.left() + self.slot.pad() + x,
                    bounds.top()
                        + px(text_top + COMPOSER_LINE_HEIGHT * line_index as f32 + CARET_INSET_Y),
                );
                window.paint_quad(PaintQuad {
                    bounds: Bounds::new(origin, size(px(1.), px(14.))),
                    corner_radii: Corners::default(),
                    background: Background::from(colour),
                    border_widths: Edges::default(),
                    border_color: colour,
                    border_style: BorderStyle::default(),
                });
            }
        }
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
    }
}

/// The text caret.
/// Stable element id per field slot.
///
/// The fixed slots live above any possible input index, and connection fields get
/// their own block, so adding an input never renumbers a settings field.
pub(crate) fn field_slot(slot: Field) -> usize {
    match slot {
        Field::Composer => 0,
        Field::Input(index) => index + 1,
        Field::ConnModel(index) => 2000 + index * 3,
        Field::ConnKey(index) => 2001 + index * 3,
        Field::ConnUrl(index) => 2002 + index * 3,
        Field::ChatFilter => 3000,
        Field::ChatFind => 3001,
        Field::ChatTitle => 4000,
        Field::NewCategory => 5000,
        Field::CategoryName => 5001,
        Field::ChatInstructions => 6000,
        Field::ColorHex(index) => 7000 + index,
        Field::Proxy => 8000,
    }
}

/// The draft of the composer, line by line, with the caret in place.
///
/// Only explicit line breaks split rows: the field has no soft wrapping, so a long
/// line simply runs on as before.
/// The caret's x offset inside its line, and how wide that line is.
///
/// Both the caret painter and the scroll keeper need these, and both must measure
/// the text exactly the way `shape_line` lays it out.
pub(crate) fn caret_metrics(window: &Window, text: &str, caret: usize) -> (Pixels, Pixels) {
    let caret = caret.min(text.len());
    let line_start = text[..caret].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[caret..]
        .find('\n')
        .map_or(text.len(), |offset| caret + offset);
    let line_text = &text[line_start..line_end];
    let runs = vec![TextRun {
        len: line_text.len(),
        font: gpui::font(".SystemUIFont"),
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let shaped = window.text_system().shape_line(
        line_text.to_string().into(),
        px(FIELD_FONT_SIZE),
        &runs,
        None,
    );
    (
        shaped.x_for_index(caret - line_start),
        shaped.x_for_index(line_text.len()),
    )
}

pub(crate) fn field_spans(
    field: &TextField,
    selection: Option<Range<usize>>,
    palette: Palette,
) -> AnyElement {
    let marked = field.marked();
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut start = 0;
    for line in field.text.split('\n') {
        let end = start + line.len();
        rows.push(field_line(
            &field.text,
            start..end,
            selection.clone(),
            marked.clone(),
            palette,
        ));
        start = end + 1;
    }
    div()
        .flex()
        .flex_col()
        .items_start()
        .min_w(px(0.))
        // Shifted rather than translated: the row clips it, so a long line scrolls.
        .relative()
        .left(px(-field.scroll()))
        .children(rows)
        .into_any_element()
}

/// One line of a field, coloured by its selection and composition.
///
/// The line is shaped **once**, as a single text element with styled runs: a
/// row of separate spans would re-shape the text after every cut, so the tail
/// would sit at a slightly different subpixel offset and the whole line would
/// visibly shift the moment a selection appeared. The caret is not part of this
/// flow either — it is painted on top of the line (see [`FieldElement`]).
pub(crate) fn field_line(
    text: &str,
    line: Range<usize>,
    selection: Option<Range<usize>>,
    marked: Option<Range<usize>>,
    palette: Palette,
) -> AnyElement {
    let mut cuts = vec![line.start, line.end];
    let mut add = |value: usize| {
        if value > line.start && value < line.end {
            cuts.push(value);
        }
    };
    if let Some(range) = &selection {
        add(range.start);
        add(range.end);
    }
    if let Some(range) = &marked {
        add(range.start);
        add(range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> = Vec::new();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if from >= to {
            continue;
        }
        let composing = marked
            .as_ref()
            .is_some_and(|range| range.start <= from && to <= range.end);
        let selected = selection
            .as_ref()
            .is_some_and(|range| range.start <= from && to <= range.end);
        if !composing && !selected {
            continue;
        }
        highlights.push((
            (from - line.start)..(to - line.start),
            gpui::HighlightStyle {
                color: composing.then(|| rgb(palette.accent).into()),
                background_color: selected.then(|| rgb(palette.selection).into()),
                ..Default::default()
            },
        ));
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .min_h(px(COMPOSER_LINE_HEIGHT))
        .text_sm()
        .text_color(rgb(palette.text))
        // One line stays one line: a long row is clipped by the field, not
        // re-wrapped into rows the caret maths would not know about.
        .whitespace_nowrap()
        .child(gpui::StyledText::new(text[line].to_string()).with_highlights(highlights))
        .into_any_element()
}

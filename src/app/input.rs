//! The platform text-input bridge, so IME composition reaches the focused field.

use super::*;

impl EntityInputHandler for DshFlowApp {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let field = self.focused.and_then(|slot| self.field(slot))?;
        *adjusted_range = Some(range.clone());
        Some(field.text_in(range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let field = self.focused.and_then(|slot| self.field(slot))?;
        // The real selection, not just the caret: a collapsed range would make a
        // keystroke miss the text the reader had selected.
        if let Some(range) = field.selection_utf16() {
            return Some(UTF16Selection {
                range,
                reversed: field.selection_reversed(),
            });
        }
        let caret = field.caret_utf16();
        Some(UTF16Selection {
            range: caret..caret,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.focused
            .and_then(|slot| self.field(slot))
            .and_then(|field| field.marked_utf16())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.focused.and_then(|slot| self.field_mut(slot)) {
            field.unmark();
        }
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.focused else {
            return;
        };
        let Some(target) = self.input_target(range) else {
            return;
        };
        if let Some(field) = self.field_mut(slot) {
            field.replace(target, text);
        }
        if slot == Field::Composer {
            self.composer_recall = None;
        }
        self.commit_edit(slot, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(slot) = self.focused else {
            return;
        };
        let Some(target) = self.input_target(range) else {
            return;
        };
        if let Some(field) = self.field_mut(slot) {
            field.replace(target, new_text);
            if !new_text.is_empty() {
                let end = field.caret_byte();
                field.mark(end - new_text.len()..end);
            }
        }
        if slot == Field::Composer {
            self.composer_recall = None;
        }
        self.commit_edit(slot, cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The IME only needs somewhere sensible for its candidate window; the
        // field's own box is close enough for a single-line input.
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

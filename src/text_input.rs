//! A one-line text field that also accepts explicit line breaks.
//!
//! GPUI (gpui-pre 0.3.14) exposes the platform input plumbing but no ready-made field, and
//! this window only needs short values (a question, a path, a key). Rather than
//! pulling a component library for that, the field handles typed characters,
//! caret movement, paste and Escape itself, and keeps a *marked* range so an IME
//! composition can be shown and then replaced by the committed text.

use std::ops::Range;

use gpui::{Context, FocusHandle, KeyDownEvent};

/// What a key press meant to the field.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FieldKey {
    /// The field consumed the key.
    Handled,
    /// Not ours: let the window's shortcuts see it.
    Ignored,
    /// Ctrl-V: the app reads the clipboard and calls [`TextField::insert`].
    Paste,
    /// Ctrl-C: the app copies what is selected (the whole field when all of it is).
    Copy,
    /// Ctrl-X: the app copies and then [`TextField::cut`]s the selection.
    Cut,
    /// Enter: the app decides what committing means.
    Submit,
    /// Escape: leave the field.
    Escape,
}

/// One editable line of text plus its focus handle.
pub struct TextField {
    focus: FocusHandle,
    /// Current contents.
    pub text: String,
    /// Insertion point as a byte offset, always on a char boundary.
    caret: usize,
    /// Byte range an IME is composing into; empty when nothing is being composed.
    marked: Range<usize>,
    /// Horizontal offset in pixels, so the caret can stay inside a long line.
    scroll: f32,
    /// Snapshots taken before each edit, newest last.
    undo: Vec<(String, usize)>,
    /// Snapshots undone, so Ctrl+Y can walk back into them.
    redo: Vec<(String, usize)>,
    /// Where the selection started, when there is one.
    ///
    /// gpui-pre 0.3.14 has no text selection at all, so the field keeps the pair
    /// itself: [`TextField::anchor`] and [`TextField::caret`] bracket the selected
    /// range. A click or a drag sets it, `Ctrl+A` puts it at the start, and any
    /// edit or caret move drops it.
    anchor: Option<usize>,
}

impl TextField {
    /// How far the text is scrolled to the left, in pixels.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// Scroll the text so the caret can stay in view.
    pub fn set_scroll(&mut self, scroll: f32) {
        self.scroll = scroll.max(0.);
    }

    /// Create an empty field.
    pub fn new<T>(cx: &mut Context<T>) -> Self {
        Self {
            focus: cx.focus_handle(),
            text: String::new(),
            caret: 0,
            marked: 0..0,
            scroll: 0.,
            undo: Vec::new(),
            redo: Vec::new(),
            anchor: None,
        }
    }

    /// Handle used to give the field keyboard focus.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Replace the contents and put the caret at the end.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.caret = self.text.len();
        self.marked = 0..0;
        self.anchor = None;
        self.scroll = 0.;
    }

    /// Remove every character.
    pub fn clear(&mut self) {
        self.set_text(String::new());
    }

    /// The selected byte range, if anything is selected.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(if anchor < self.caret {
            anchor..self.caret
        } else {
            self.caret..anchor
        })
    }

    /// Select everything and put the caret at the end.
    pub fn select_all(&mut self) {
        self.marked = 0..0;
        self.caret = self.text.len();
        self.anchor = Some(0);
    }

    /// Drop the selection without changing the text.
    pub fn deselect(&mut self) {
        self.anchor = None;
        self.marked = 0..0;
    }

    /// Put the caret at `index`, without a selection: a plain click.
    pub fn caret_from_click(&mut self, index: usize) {
        let index = self.clamp(index);
        self.caret = index;
        self.anchor = Some(index);
        self.marked = 0..0;
    }

    /// Drag the caret's end of the selection to `index`.
    pub fn extend_to(&mut self, index: usize) {
        if self.anchor.is_none() {
            self.anchor = Some(self.caret);
        }
        self.caret = self.clamp(index);
    }

    /// Finish a drag; a click that selected nothing leaves no selection.
    pub fn end_drag(&mut self) {
        if self.selection().is_none() {
            self.anchor = None;
        }
    }

    /// Remember the current state so Ctrl+Z can come back to it.
    fn remember(&mut self) {
        self.undo.push((self.text.clone(), self.caret));
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Go back one edit; true when something changed.
    pub fn undo(&mut self) -> bool {
        let Some((text, caret)) = self.undo.pop() else {
            return false;
        };
        self.redo.push((self.text.clone(), self.caret));
        self.text = text;
        self.caret = self.clamp(caret);
        self.anchor = None;
        self.marked = 0..0;
        true
    }

    /// Go forward again; true when something changed.
    pub fn redo(&mut self) -> bool {
        let Some((text, caret)) = self.redo.pop() else {
            return false;
        };
        self.undo.push((self.text.clone(), self.caret));
        self.text = text;
        self.caret = self.clamp(caret);
        self.anchor = None;
        self.marked = 0..0;
        true
    }

    /// Select the word (or the run of punctuation) around `index`.
    pub fn select_word_at(&mut self, index: usize) {
        let range = word_range(&self.text, index);
        self.anchor = Some(range.start);
        self.caret = range.end;
        self.marked = 0..0;
    }

    /// Move the caret, keeping the other end of a selection when asked to.
    fn move_caret(&mut self, index: usize, keep_selection: bool) {
        if keep_selection {
            self.extend_to(index);
        } else {
            self.anchor = None;
            self.caret = self.clamp(index);
        }
    }

    /// What Ctrl-C should put on the clipboard: the selection, or nothing.
    pub fn selection_text(&self) -> Option<String> {
        self.selection().map(|range| self.text[range].to_string())
    }

    /// The selection as UTF-16 offsets, for the platform input handler.
    ///
    /// Reporting the real range is what lets a keystroke replace the selection:
    /// a collapsed range would tell the platform nothing is selected.
    pub fn selection_utf16(&self) -> Option<Range<usize>> {
        self.selection().map(|range| {
            byte_to_utf16(&self.text, range.start)..byte_to_utf16(&self.text, range.end)
        })
    }

    /// True when the anchor sits after the caret, i.e. the selection was made
    /// backwards.
    pub fn selection_reversed(&self) -> bool {
        self.anchor.is_some_and(|anchor| anchor > self.caret)
    }

    /// Remove the selection, as Ctrl-X does after copying it.
    pub fn cut(&mut self) {
        self.delete_selection();
    }

    /// Remove the selected range and leave the caret where it started.
    pub fn delete_selection(&mut self) -> bool {
        if self.selection().is_none() {
            return false;
        }
        self.remember();
        self.remove_selection();
        true
    }

    /// Drop the selected range; the caller has already taken a snapshot.
    fn remove_selection(&mut self) {
        let Some(range) = self.selection() else {
            return;
        };
        self.caret = range.start;
        self.text.replace_range(range, "");
        self.anchor = None;
    }

    /// The caret as a byte offset.
    pub fn caret_byte(&self) -> usize {
        self.caret
    }

    /// The caret as a UTF-16 offset, which is what IME messages use.
    pub fn caret_utf16(&self) -> usize {
        byte_to_utf16(&self.text, self.caret)
    }

    /// The byte range an IME is composing into, if any.
    pub fn marked(&self) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then(|| self.marked.clone())
    }

    /// The composing range as UTF-16 offsets.
    pub fn marked_utf16(&self) -> Option<Range<usize>> {
        self.marked().map(|range| {
            byte_to_utf16(&self.text, range.start)..byte_to_utf16(&self.text, range.end)
        })
    }

    /// Record an IME composition over a byte range.
    pub fn mark(&mut self, range: Range<usize>) {
        self.marked = range;
    }

    /// Drop the composition mark, keeping whatever text it left behind.
    pub fn unmark(&mut self) {
        self.marked = 0..0;
    }

    /// Replace a byte range, leaving the caret after the new text.
    ///
    /// Whatever is selected is replaced as well: the platform reports a
    /// collapsed caret for a keystroke, so the range alone would miss the text
    /// the reader had selected.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let (body, caret) = edited(&self.text, range, self.selection(), text);
        self.remember();
        self.text = body;
        self.caret = caret;
        self.anchor = None;
        self.marked = 0..0;
    }

    /// Turn a UTF-16 range into a byte range.
    pub fn byte_range(&self, range: Range<usize>) -> Range<usize> {
        let start = self.clamp(utf16_to_byte(&self.text, range.start));
        let end = self.clamp(utf16_to_byte(&self.text, range.end)).max(start);
        start..end
    }

    /// The text inside a UTF-16 range.
    pub fn text_in(&self, range: Range<usize>) -> String {
        let bytes = self.byte_range(range);
        self.text[bytes].to_string()
    }

    /// Snap a byte offset into the text, on a char boundary.
    fn clamp(&self, index: usize) -> usize {
        floor_boundary(&self.text, index)
    }

    /// Insert text at the caret.
    pub fn insert(&mut self, addition: &str) {
        self.replace(self.caret..self.caret, addition);
    }

    /// Apply one key press.
    pub fn handle_key(&mut self, event: &KeyDownEvent) -> FieldKey {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;

        if modifiers.control || modifiers.platform || modifiers.function {
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("v") {
                return FieldKey::Paste;
            }
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("a") {
                self.select_all();
                return FieldKey::Handled;
            }
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("c") {
                return FieldKey::Copy;
            }
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("x") {
                return FieldKey::Cut;
            }
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("z") {
                if modifiers.shift {
                    self.redo();
                } else {
                    self.undo();
                }
                return FieldKey::Handled;
            }
            if modifiers.control && keystroke.key.eq_ignore_ascii_case("y") {
                self.redo();
                return FieldKey::Handled;
            }
            if modifiers.control && keystroke.key == "backspace" {
                if !self.delete_selection() {
                    self.remember();
                    delete_word_backward(&mut self.text, &mut self.caret);
                }
                return FieldKey::Handled;
            }
            return FieldKey::Ignored;
        }

        match keystroke.key.as_str() {
            // A delete key over a selection removes the selection, as in any editor.
            "backspace" | "delete" if self.selection().is_some() => {
                self.delete_selection();
                FieldKey::Handled
            }
            "backspace" => {
                self.remember();
                delete_backward(&mut self.text, &mut self.caret);
                FieldKey::Handled
            }
            "delete" => {
                self.remember();
                delete_forward(&mut self.text, &mut self.caret);
                FieldKey::Handled
            }
            "left" => {
                let target = previous_boundary(&self.text, self.caret);
                self.move_caret(target, modifiers.shift);
                FieldKey::Handled
            }
            "right" => {
                let target = next_boundary(&self.text, self.caret);
                self.move_caret(target, modifiers.shift);
                FieldKey::Handled
            }
            "up" | "down" => {
                let mut caret = self.caret;
                move_line(
                    &self.text,
                    &mut caret,
                    if keystroke.key == "up" { -1 } else { 1 },
                );
                self.move_caret(caret, modifiers.shift);
                FieldKey::Handled
            }
            "home" => {
                let target = line_start(&self.text, self.caret);
                self.move_caret(target, modifiers.shift);
                FieldKey::Handled
            }
            "end" => {
                let target = line_end(&self.text, self.caret);
                self.move_caret(target, modifiers.shift);
                FieldKey::Handled
            }
            "escape" => FieldKey::Escape,
            // Shift+Enter breaks the line; Enter alone means "done".
            "enter" if modifiers.shift => {
                self.insert("\n");
                FieldKey::Handled
            }
            "enter" => FieldKey::Submit,
            "tab" | "pageup" | "pagedown" | "insert" => FieldKey::Ignored,
            // Printable characters do not arrive here: the platform sends them to
            // the window's input handler (WM_CHAR → `replace_text_in_range`), which
            // is also how an IME commits its composition. Inserting `key_char` too
            // would double every keystroke.
            _ => FieldKey::Ignored,
        }
    }
}

/// How many edits Ctrl+Z can walk back.
const MAX_UNDO: usize = 200;

/// Apply one edit: the reported range, plus whatever is selected.
///
/// A keystroke, a paste or an IME commit replaces the selection, and the platform
/// reports a collapsed caret in that case — so the selection has to be taken
/// into account here rather than trusting the range alone.
fn edited(
    text: &str,
    range: Range<usize>,
    selection: Option<Range<usize>>,
    addition: &str,
) -> (String, usize) {
    let mut start = range.start;
    let mut end = range.end.max(start);
    if let Some(selection) = selection {
        start = start.min(selection.start);
        end = end.max(selection.end);
    }
    let start = floor_boundary(text, start);
    let end = floor_boundary(text, end).max(start);
    let mut body = text.to_string();
    body.replace_range(start..end, addition);
    (body, start + addition.len())
}

/// `index`, moved back to the nearest char boundary at or before it.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The run of word characters (or of non-word characters) that `index` is in.
///
/// A double click uses this — in a text field and in the transcript alike:
/// letters, digits, `_` and CJK characters are one kind, everything else
/// (spaces, punctuation) is another, so double clicking next to a comma selects
/// the comma rather than the word beside it.
pub(crate) fn word_range(text: &str, index: usize) -> Range<usize> {
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    if characters.is_empty() {
        return 0..0;
    }

    // The character under the index; past the end, the last one.
    let anchor = characters
        .iter()
        .position(|(offset, _)| *offset >= index.min(text.len()))
        .unwrap_or(characters.len() - 1);

    let word = is_word_char(characters[anchor].1);
    let mut start = anchor;
    let mut end = anchor;
    while start > 0 && is_word_char(characters[start - 1].1) == word {
        start -= 1;
    }
    while end + 1 < characters.len() && is_word_char(characters[end + 1].1) == word {
        end += 1;
    }

    characters[start].0..characters[end].0 + characters[end].1.len_utf8()
}

/// Whether a character belongs to a "word" for double-click selection.
fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Byte offset where the line containing `caret` starts.
fn line_start(text: &str, caret: usize) -> usize {
    text[..caret].rfind('\n').map_or(0, |index| index + 1)
}

/// Byte offset where the line containing `caret` ends, before its newline.
fn line_end(text: &str, caret: usize) -> usize {
    text[caret..]
        .find('\n')
        .map_or(text.len(), |offset| caret + offset)
}

/// Move the caret a line up (-1) or down (1), keeping it in one line.
fn move_line(text: &str, caret: &mut usize, delta: i32) {
    let start = line_start(text, *caret);
    let column = *caret - start;

    if delta < 0 {
        if start == 0 {
            *caret = 0;
            return;
        }
        let previous_end = start - 1;
        let previous_start = line_start(text, previous_end);
        *caret = (previous_start + column).min(previous_end);
        return;
    }

    let end = line_end(text, *caret);
    if end == text.len() {
        *caret = text.len();
        return;
    }
    let next_start = end + 1;
    let next_end = line_end(text, next_start);
    let target = next_start + column;
    // Snap back to a char boundary if the column landed inside a character.
    let mut target = target.min(next_end);
    while target > next_start && !text.is_char_boundary(target) {
        target -= 1;
    }
    *caret = target;
}

/// Byte offset of a UTF-16 offset, rounding down inside a surrogate pair.
fn utf16_to_byte(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        if units >= index {
            return byte;
        }
        units += character.len_utf16();
    }
    text.len()
}

/// UTF-16 offset of a byte offset.
fn byte_to_utf16(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].encode_utf16().count()
}

/// Move to the start of the previous character.
fn previous_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map_or(0, |(offset, _)| offset)
}

/// Move to the start of the next character.
fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| index + offset)
}

/// Remove the character before the caret.
fn delete_backward(text: &mut String, caret: &mut usize) {
    if *caret == 0 {
        return;
    }
    let start = previous_boundary(text, *caret);
    text.replace_range(start..*caret, "");
    *caret = start;
}

/// Remove the character after the caret.
fn delete_forward(text: &mut String, caret: &mut usize) {
    if *caret >= text.len() {
        return;
    }
    let end = next_boundary(text, *caret);
    text.replace_range(*caret..end, "");
}

/// Remove the word before the caret, as Ctrl-Backspace does elsewhere.
fn delete_word_backward(text: &mut String, caret: &mut usize) {
    let mut start = *caret;
    while start > 0 {
        let previous = previous_boundary(text, start);
        if !text[previous..start].chars().all(char::is_whitespace) {
            break;
        }
        start = previous;
    }
    while start > 0 {
        let previous = previous_boundary(text, start);
        if text[previous..start].chars().all(char::is_whitespace) {
            break;
        }
        start = previous;
    }
    text.replace_range(start..*caret, "");
    *caret = start;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str, caret: usize) -> (String, usize) {
        (text.to_string(), caret)
    }

    #[test]
    fn caret_moves_by_characters_not_bytes() {
        let (text, mut caret) = state("héllo", 0);
        caret = next_boundary(&text, caret);
        assert_eq!(caret, 1);
        caret = next_boundary(&text, caret);
        assert_eq!(caret, 3, "é is two bytes");
        assert_eq!(previous_boundary(&text, caret), 1);
    }

    #[test]
    fn deletes_respect_char_boundaries() {
        let (mut text, mut caret) = state("aé", 3);
        delete_backward(&mut text, &mut caret);
        assert_eq!((text.clone(), caret), ("a".to_string(), 1));

        let (mut text, mut caret) = state("aé", 1);
        delete_forward(&mut text, &mut caret);
        assert_eq!((text, caret), ("a".to_string(), 1));
    }

    #[test]
    fn deletes_whole_words_backwards() {
        let (mut text, mut caret) = state("python steps/count.py", 21);
        delete_word_backward(&mut text, &mut caret);
        assert_eq!((text.clone(), caret), ("python ".to_string(), 7));
        delete_word_backward(&mut text, &mut caret);
        assert_eq!((text, caret), (String::new(), 0));
    }

    #[test]
    fn word_range_selects_the_run_around_the_index() {
        // "hello world": word, space and second word are three separate runs.
        assert_eq!(word_range("hello world", 0), 0..5);
        assert_eq!(word_range("hello world", 3), 0..5);
        assert_eq!(word_range("hello world", 5), 5..6);
        assert_eq!(word_range("hello world", 7), 6..11);
    }

    #[test]
    fn word_range_handles_cjk_and_punctuation() {
        // CJK ideographs count as word characters, so a double click takes the run.
        assert_eq!(word_range("中文，测试", 0), 0..6);
        assert_eq!(word_range("中文，测试", 6), 6..9);
        assert_eq!(word_range("a_b c", 1), 0..3);
    }

    #[test]
    fn word_range_stays_inside_the_text() {
        assert_eq!(word_range("", 0), 0..0);
        assert_eq!(word_range("abc", 99), 0..3);
    }

    #[test]
    fn never_moves_past_the_ends() {
        let (text, mut caret) = state("ab", 2);
        caret = next_boundary(&text, caret);
        assert_eq!(caret, 2);
        caret = previous_boundary(&text, 0);
        assert_eq!(caret, 0);
    }

    #[test]
    fn converts_between_utf16_and_bytes() {
        let text = "建a😀b";
        assert_eq!(utf16_to_byte(text, 0), 0);
        assert_eq!(utf16_to_byte(text, 1), 3, "one CJK char is three bytes");
        assert_eq!(utf16_to_byte(text, 2), 4);
        assert_eq!(utf16_to_byte(text, 4), 8, "the emoji is two UTF-16 units");
        assert_eq!(utf16_to_byte(text, 99), text.len());
        assert_eq!(byte_to_utf16(text, 0), 0);
        assert_eq!(byte_to_utf16(text, 3), 1);
        assert_eq!(byte_to_utf16(text, 4), 2);
        assert_eq!(byte_to_utf16(text, 8), 4);
    }

    #[test]
    fn moves_between_lines() {
        // "ab\ncd\n" with the caret after "c" on the second line.
        let text = "ab\ncd\nef";
        let mut caret = 5;
        move_line(text, &mut caret, -1);
        assert_eq!(caret, 2, "up keeps the column");
        move_line(text, &mut caret, -1);
        assert_eq!(caret, 0, "up on the first line goes to its start");
        // Going down from column 0 lands on column 0 of the next line: the column
        // is not remembered, it is measured from where the caret is.
        move_line(text, &mut caret, 1);
        assert_eq!(caret, 3);
        // Down from the end of the last line stays there.
        let mut caret = text.len();
        move_line(text, &mut caret, 1);
        assert_eq!(caret, text.len());
    }

    #[test]
    fn a_short_line_clamps_the_column() {
        // Move from the end of "abcd" down onto "e", which has no column 4.
        let text = "abcd\ne";
        let mut caret = 4;
        move_line(text, &mut caret, 1);
        assert_eq!(caret, 6, "clamped to the end of the shorter line");
    }

    #[test]
    fn home_and_end_stay_on_the_line() {
        let text = "one\ntwo";
        assert_eq!(line_start(text, 5), 4);
        assert_eq!(line_end(text, 5), 7);
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_end(text, 0), 3);
    }

    #[test]
    fn typing_over_a_selection_replaces_it_in_place() {
        // A mouse drag leaves the caret at the end of the selection, and the
        // platform reports that collapsed caret for the keystroke.
        let (text, caret) = edited("0123456789", 6..6, Some(3..6), "X");
        assert_eq!(text, "012X6789");
        assert_eq!(caret, 4);
    }

    #[test]
    fn pasting_over_a_selection_replaces_it_in_place() {
        let (text, caret) = edited("hello world", 5..5, Some(0..5), "bye");
        assert_eq!(text, "bye world");
        assert_eq!(caret, 3);
    }

    #[test]
    fn an_edit_without_a_selection_uses_the_reported_range() {
        // A caret-shaped range appends where the caret is.
        let (text, caret) = edited("abc", 3..3, None, "XY");
        assert_eq!((text.as_str(), caret), ("abcXY", 5));
        // An IME commit replaces the composition it marked.
        let (text, caret) = edited("abc", 1..2, None, "XY");
        assert_eq!((text.as_str(), caret), ("aXYc", 3));
    }

    #[test]
    fn an_edit_never_splits_a_character() {
        // A range reported from outside can land inside a multi-byte character.
        let (text, caret) = edited("中文", 1..1, None, "x");
        assert_eq!((text.as_str(), caret), ("x中文", 1));
    }
}

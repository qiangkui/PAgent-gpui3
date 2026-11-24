//! Where `*`, `_` and `~~` mark emphasis: the flanking rules that keep `snake_case` and `5 ~ 10` as text.

use super::super::script;

/// How many `~` start at `index`.
pub(crate) fn tilde_run(characters: &[char], index: usize) -> usize {
    characters[index..]
        .iter()
        .take_while(|tilde| **tilde == '~')
        .count()
}

/// Whether a `==` at `index` opens a highlight: the text inside may not start
/// with a space, and the pair is exactly two marks — `a === b` in prose stays
/// the comparison it is.
pub(crate) fn opens_mark(characters: &[char], index: usize) -> bool {
    marker_run(characters, index, '=') == 2
        && characters
            .get(index + 2)
            .is_some_and(|next| !next.is_whitespace())
}

/// Whether a `==` at `index` closes a highlight: text comes before it, and the
/// pair is exactly two marks.
pub(crate) fn closes_mark(characters: &[char], index: usize) -> bool {
    marker_run(characters, index, '=') == 2
        && index
            .checked_sub(1)
            .and_then(|at| characters.get(at))
            .is_some_and(|before| !before.is_whitespace())
}

/// Whether some later `==` closes the highlight opened at `index`.
///
/// The search looks past code spans and escapes, like the other closers: an
/// unmatched `==` stays the characters it is instead of marking the rest of the
/// line.
pub(crate) fn has_mark_closer(characters: &[char], index: usize) -> bool {
    let mut cursor = index + 2;
    let mut code = false;
    while let Some(character) = characters.get(cursor).copied() {
        match character {
            '`' => {
                code = !code;
                cursor += 1;
            }
            '\\' => cursor += 2,
            '=' if !code => {
                if closes_mark(characters, cursor) {
                    return true;
                }
                cursor += marker_run(characters, cursor, '=');
            }
            _ => cursor += 1,
        }
    }
    false
}

/// A `^上标^` or `~下标~` run at `index` → its Unicode form and where the text
/// after it starts.
///
/// A `TextRun` has no baseline offset, so the characters carry the position
/// themselves: `x^2^` draws `x²` and `H~2~O` draws `H₂O`, the same shapes the
/// HTML tags produce. The marks have to touch text on both sides and the whole
/// run has to convert — `5~10 个`, `2^10` and `^_^` stay the characters they are,
/// and so does anything with no script form (Chinese).
pub(crate) fn script_run(
    characters: &[char],
    index: usize,
    marker: char,
    raised: bool,
) -> Option<(String, usize)> {
    if characters.get(index) != Some(&marker) {
        return None;
    }
    // `~~` is a strikethrough and `^^` is nothing: only one mark starts a run.
    if characters.get(index + 1) == Some(&marker) {
        return None;
    }
    // A pair of markers is not a marker pair when it is one half of a longer
    // run, which is why the character before has to differ too.
    if index > 0 && characters.get(index - 1) == Some(&marker) {
        return None;
    }
    let first = characters.get(index + 1)?;
    // `^[说明]` is a note, and a space after the mark is not a script.
    if first.is_whitespace() || (marker == '^' && *first == '[') {
        return None;
    }
    let mut end = index + 1;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '`' | '\n' => return None,
            ' ' | '\t' => return None,
            found if found == marker => {
                let inner: String = characters[index + 1..end].iter().collect();
                if inner.is_empty() || inner.ends_with(' ') {
                    return None;
                }
                let text = script(&inner, raised)?;
                return Some((text, end + 1));
            }
            // Emphasis markers inside would have to be re-read as Markdown;
            // keeping them out means the run is plain characters.
            '*' | '_' | '~' | '^' | '[' | ']' => return None,
            _ => end += 1,
        }
    }
    None
}

/// Whether some later `~~` closes a strikethrough opened at `index`.
///
/// Tildes inside backticks do not count, and neither does an escaped one: an
/// unmatched `~~` stays the text it is instead of striking the rest of the line.
pub(crate) fn has_strike_closer(characters: &[char], index: usize) -> bool {
    let mut cursor = index + 2;
    let mut code = false;
    while let Some(character) = characters.get(cursor).copied() {
        match character {
            '`' => {
                code = !code;
                cursor += 1;
            }
            '\\' => cursor += 2,
            '~' if !code => {
                if tilde_run(characters, cursor) >= 2 {
                    return true;
                }
                cursor += 1;
            }
            _ => cursor += 1,
        }
    }
    false
}

/// How many `marker` characters start at `index`.
pub(crate) fn marker_run(characters: &[char], index: usize, marker: char) -> usize {
    characters[index..]
        .iter()
        .take_while(|found| **found == marker)
        .count()
}

/// Whether a character is punctuation rather than a word.
///
/// Used for the emphasis flanking rules: a `*` next to a space is a star, not a
/// marker, so "5 * 3" keeps its asterisks.
pub(crate) fn is_punct(character: char) -> bool {
    !character.is_alphanumeric() && !character.is_whitespace()
}

/// Whether the `marker` at `index` can open emphasis: something that is not a
/// space follows it.
///
/// An underscore directly after a letter or digit is part of the word — the `_`
/// of `snake_case` is not emphasis — which is the one place `_` differs from
/// `*`.
pub(crate) fn opens_emphasis(characters: &[char], index: usize, marker: char) -> bool {
    let next = characters.get(index + 1).copied();
    let previous = index
        .checked_sub(1)
        .and_then(|at| characters.get(at).copied());
    match next {
        Some(next) if !next.is_whitespace() => {
            if marker == '_' && previous.is_some_and(|previous| previous.is_alphanumeric()) {
                return false;
            }
            match previous {
                Some(previous) => !is_punct(next) || previous.is_whitespace() || is_punct(previous),
                None => true,
            }
        }
        _ => false,
    }
}

/// Whether the `marker` at `index` can close emphasis: something that is not a
/// space comes before it, and an underscore may not end inside a word.
pub(crate) fn closes_emphasis(characters: &[char], index: usize, marker: char) -> bool {
    closes_run(characters, index, 1, marker)
}

/// Whether the run of `marks` markers starting at `start` can close emphasis.
///
/// The characters that matter are the ones *around* the run, not between its
/// marks: `**重点**普通` closes because a word comes before it, even though the
/// mark just before the closing pair is a star.
pub(crate) fn closes_run(characters: &[char], start: usize, marks: usize, marker: char) -> bool {
    let previous = start
        .checked_sub(1)
        .and_then(|at| characters.get(at).copied());
    let next = characters.get(start + marks).copied();
    match previous {
        Some(previous) if !previous.is_whitespace() => {
            if marker == '_' && next.is_some_and(|next| next.is_alphanumeric()) {
                return false;
            }
            match next {
                Some(next) => !is_punct(previous) || next.is_whitespace() || is_punct(next),
                None => true,
            }
        }
        _ => false,
    }
}

/// Whether some later `marker` can close emphasis opened at `index`.
///
/// Marks inside backticks and the pairs that make bold do not count: an
/// unmatched marker stays the character it is instead of turning the rest of the
/// text italic.
pub(crate) fn has_closer(characters: &[char], index: usize, marker: char) -> bool {
    let mut cursor = index + 1;
    let mut code = false;
    while let Some(character) = characters.get(cursor).copied() {
        match character {
            '`' => {
                code = !code;
                cursor += 1;
            }
            '\\' => cursor += 2,
            '*' | '_' if !code => {
                let marks = marker_run(characters, cursor, character);
                // An odd run can spare one mark to close italic, which is what
                // `**粗 *斜***` needs: the closing three close both.
                if character == marker
                    && marks % 2 == 1
                    && closes_run(characters, cursor, marks, marker)
                {
                    return true;
                }
                cursor += marks;
            }
            _ => cursor += 1,
        }
    }
    false
}

/// Whether some later run of two or more closes the bold opened at `index`.
///
/// Without this `2**10` would turn everything after it bold, because a pair of
/// stars is arithmetic as often as it is a marker.
pub(crate) fn has_bold_closer(characters: &[char], index: usize, marker: char) -> bool {
    let mut cursor = index + 2;
    let mut code = false;
    while let Some(character) = characters.get(cursor).copied() {
        match character {
            '`' => {
                code = !code;
                cursor += 1;
            }
            '\\' => cursor += 2,
            '*' | '_' if !code => {
                let marks = marker_run(characters, cursor, character);
                if character == marker
                    && marks >= 2
                    && closes_run(characters, cursor, marks, marker)
                {
                    return true;
                }
                cursor += marks;
            }
            _ => cursor += 1,
        }
    }
    false
}

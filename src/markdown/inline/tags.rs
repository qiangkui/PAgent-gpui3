//! Inline HTML: styling tags, `<sup>/<sub>`, `<a href>`, `<img>` and attribute values.

use super::*;

/// The style an inline HTML tag turns on, if any.
///
/// Only the styles a run already has: a tag cannot change a run's colour, and
/// `<code>` switches the code *font* on for the run it wraps — not the inline
/// code flag, which would make the rest of the line literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HtmlStyle {
    Bold,
    Italic,
    Strike,
    Underline,
    Mono,
    Mark,
}

/// The run style a tag name stands for, `None` for a tag the app only drops.
pub(crate) fn html_style(name: &str) -> Option<HtmlStyle> {
    Some(match name {
        "b" | "strong" => HtmlStyle::Bold,
        "i" | "em" => HtmlStyle::Italic,
        "s" | "del" | "strike" => HtmlStyle::Strike,
        "u" | "ins" => HtmlStyle::Underline,
        "mark" => HtmlStyle::Mark,
        "code" | "kbd" | "samp" | "tt" | "var" => HtmlStyle::Mono,
        _ => return None,
    })
}

/// An inline HTML tag the app knows how to ignore, starting at `index`.
///
/// Only the few tags that show up in answers count: a `<div>` or a stray `<` in
/// prose stays as it was written, because guessing wrong there would eat text.
/// Returns the tag's name, whether it closes, and where the text after it
/// starts.
pub(crate) fn html_tag(characters: &[char], index: usize) -> Option<(String, bool, usize)> {
    if characters.get(index) != Some(&'<') {
        return None;
    }
    let mut cursor = index + 1;
    let closing = characters.get(cursor) == Some(&'/');
    if closing {
        cursor += 1;
    }
    let name_start = cursor;
    while characters
        .get(cursor)
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        cursor += 1;
    }
    if cursor == name_start {
        return None;
    }
    let name: String = characters[name_start..cursor]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    if !matches!(
        name.as_str(),
        "br" | "b"
            | "strong"
            | "i"
            | "em"
            | "code"
            | "kbd"
            | "samp"
            | "tt"
            | "var"
            | "sub"
            | "sup"
            | "span"
            | "u"
            | "ins"
            | "mark"
            | "s"
            | "del"
            | "strike"
            | "a"
            | "hr"
            | "img"
            | "p"
    ) {
        return None;
    }
    // Attributes are allowed (`<br class="x">`) but a tag longer than this is
    // prose that happens to start with a `<`.
    while let Some(character) = characters.get(cursor).copied() {
        match character {
            '>' => return Some((name, closing, cursor + 1)),
            '<' => return None,
            _ => cursor += 1,
        }
        if cursor - index > 64 {
            return None;
        }
    }
    None
}

/// A `<sup>…</sup>` or `<sub>…</sub>` span → its text with every character
/// swapped for the raised or lowered form, and where the text after it starts.
///
/// A `TextRun` has no baseline offset, so the characters themselves carry the
/// position: `x<sup>2</sup>` draws `x²`, `H<sub>2</sub>O` draws `H₂O`. Text
/// Unicode has no script form for (Chinese, say) stays as written, with the tags
/// dropped, rather than disappearing.
pub(crate) fn scripted_tag(characters: &[char], index: usize) -> Option<(String, usize)> {
    if characters.get(index) != Some(&'<') {
        return None;
    }
    let mut cursor = index + 1;
    let name_start = cursor;
    while characters
        .get(cursor)
        .is_some_and(|character| character.is_ascii_alphabetic())
    {
        cursor += 1;
    }
    let name: String = characters[name_start..cursor].iter().collect();
    let raised = match name.to_ascii_lowercase().as_str() {
        "sup" => true,
        "sub" => false,
        _ => return None,
    };
    // Attributes are allowed; the content starts after the tag's `>`.
    while characters
        .get(cursor)
        .is_some_and(|character| *character != '>')
    {
        if cursor - index > 64 {
            return None;
        }
        cursor += 1;
    }
    if characters.get(cursor) != Some(&'>') {
        return None;
    }
    let content_start = cursor + 1;
    let closing: Vec<char> = format!("</{name}>").chars().collect();
    let matches_at = |at: usize| {
        closing.iter().enumerate().all(|(offset, want)| {
            characters
                .get(at + offset)
                .is_some_and(|found| found.eq_ignore_ascii_case(want))
        })
    };
    let mut end = content_start;
    while end + closing.len() <= characters.len() {
        if matches_at(end) {
            let inner: String = characters[content_start..end].iter().collect();
            let text = script(&inner, raised).unwrap_or(inner);
            return Some((text, end + closing.len()));
        }
        end += 1;
    }
    None
}

/// `<a href="…">` / `</a>` → the address (when the tag opens with one), whether
/// it closes, and where the text after it starts.
pub(crate) fn anchor_tag(
    characters: &[char],
    index: usize,
) -> Option<(Option<String>, bool, usize)> {
    if characters.get(index) != Some(&'<') {
        return None;
    }
    let closing = characters.get(index + 1) == Some(&'/');
    let mut cursor = index + 1 + usize::from(closing);
    if !characters
        .get(cursor)
        .is_some_and(|character| character.eq_ignore_ascii_case(&'a'))
    {
        return None;
    }
    cursor += 1;
    // `<abbr>` must not read as `<a>`.
    if characters
        .get(cursor)
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    let attributes_start = cursor;
    while characters
        .get(cursor)
        .is_some_and(|character| *character != '>')
    {
        // A `<` before the `>` means this is prose, not a tag.
        if characters.get(cursor) == Some(&'<') || cursor - index > 300 {
            return None;
        }
        cursor += 1;
    }
    if characters.get(cursor) != Some(&'>') {
        return None;
    }
    let attributes: String = characters[attributes_start..cursor].iter().collect();
    let href = if closing {
        None
    } else {
        attribute_value(&attributes, "href")
    };
    Some((href, closing, cursor + 1))
}

/// The value of one attribute among a tag's attributes (`href`, `src`, `alt`).
pub(crate) fn attribute_value(attributes: &str, attribute: &str) -> Option<String> {
    let lowered = attributes.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lowered[from..].find(attribute) {
        let at = from + found;
        // `data-href` is not an `href`: the word has to start an attribute.
        let starts_attribute = at == 0
            || attributes[..at]
                .chars()
                .last()
                .is_some_and(|before| before.is_whitespace());
        if starts_attribute
            && let Some(rest) = attributes[at + attribute.len()..]
                .trim_start()
                .strip_prefix('=')
        {
            let rest = rest.trim_start();
            let value = match rest.chars().next()? {
                quote @ ('"' | '\'') => rest[1..].split(quote).next()?,
                _ => rest.split_whitespace().next()?,
            };
            return Some(value.trim().to_string());
        }
        from = at + attribute.len();
    }
    None
}

//! Links: `[label](url)`, reference forms, bare and angled addresses, and footnote references.

use super::*;

/// `<https://…>` starting at `index` → the address and where the text after it
/// starts.
/// `<https://…>` and `<user@example.com>`: an autolink, drawn as the address
/// without its angle brackets → what to draw, the address to open, and where the
/// text after it starts.
///
/// An address in angle brackets opens with `mailto:`; a bare one in prose does
/// not become a link, because `@` is too easy to hit by accident (`npm i @x/y`).
pub(crate) fn angle_link(characters: &[char], index: usize) -> Option<(String, String, usize)> {
    if characters.get(index) != Some(&'<') {
        return None;
    }
    let mut end = index + 1;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '>' => {
                let inner: String = characters[index + 1..end].iter().collect();
                let inner = inner.trim().to_string();
                if is_email(&inner) {
                    return Some((inner.clone(), format!("mailto:{inner}"), end + 1));
                }
                if !is_openable_link(&inner) {
                    return None;
                }
                return Some((inner.clone(), inner, end + 1));
            }
            '<' | ' ' => return None,
            _ => end += 1,
        }
    }
    None
}

/// A bare address starting at `index` → the text to draw, the address to open,
/// and where the text after it starts.
///
/// `www.example.com` counts: answers write it without a scheme, and the address
/// it stands for is the `https` one. Trailing punctuation belongs to the
/// sentence, not to the address — a closing bracket only when it has an opening
/// one to match. A URL that runs into a word (`xhttps://…`) is not one. The
/// address is drawn as it was written, so a copied selection still holds the
/// whole thing.
pub(crate) fn url_at(characters: &[char], index: usize) -> Option<(String, String, usize)> {
    let previous = index
        .checked_sub(1)
        .and_then(|at| characters.get(at).copied());
    if previous.is_some_and(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '-' | '_' | ':')
    }) {
        return None;
    }
    let mut end = index;
    while let Some(character) = characters.get(end).copied() {
        if character.is_whitespace()
            || matches!(
                character,
                '。' | '，'
                    | '、'
                    | '；'
                    | '：'
                    | '！'
                    | '？'
                    | '（'
                    | '）'
                    | '「'
                    | '」'
                    | '《'
                    | '》'
                    | '"'
                    | '\''
                    | '<'
                    | '>'
                    | '|'
                    | '`'
            )
        {
            break;
        }
        end += 1;
    }
    // A `*` cannot be part of an address, so a closing `**` right after one
    // stays outside: `**https://x**` links the address and leaves the bold to
    // the emphasis parser. A `)` is kept while an opening one is still owed, so
    // `https://en.wikipedia.org/wiki/A_(b)` is whole.
    while end > index {
        let last = characters[end - 1];
        if last == ')' {
            let slice = &characters[index..end];
            let opens = slice.iter().filter(|found| **found == '(').count();
            let closes = slice.iter().filter(|found| **found == ')').count();
            if closes <= opens {
                break;
            }
            end -= 1;
            continue;
        }
        if matches!(
            last,
            '.' | ',' | ';' | ':' | '!' | '?' | ']' | '}' | '…' | '*'
        ) {
            end -= 1;
            continue;
        }
        break;
    }
    let drawn: String = characters[index..end].iter().collect();
    if is_openable_link(&drawn) {
        return Some((drawn.clone(), drawn, end));
    }
    // No scheme: `www.…` is close enough to a host to open over `https`.
    if let Some(rest) = drawn.strip_prefix("www.")
        && rest.contains('.')
    {
        return Some((drawn.clone(), format!("https://{drawn}"), end));
    }
    None
}

/// A `[^label]` footnote reference at `index` → the number it draws as and where
/// the text after it starts. An unknown label stays as it was written.
pub(crate) fn note_reference(
    characters: &[char],
    index: usize,
    refs: &Definitions,
) -> Option<(usize, usize)> {
    if characters.get(index) != Some(&'[') || characters.get(index + 1) != Some(&'^') {
        return None;
    }
    let mut end = index + 2;
    while let Some(character) = characters.get(end).copied() {
        match character {
            ']' => break,
            '[' | '\n' => return None,
            _ => end += 1,
        }
    }
    if characters.get(end) != Some(&']') {
        return None;
    }
    let label: String = characters[index + 2..end].iter().collect();
    let number = refs.notes.get(&normalise_label(&label))?;
    Some((*number, end + 1))
}

/// A reference link starting at `index` → its label, the address it resolves to,
/// and where the text after it starts.
///
/// Three spellings share one definition: `[label][ref]`, `[ref][]` (the second
/// pair empty) and the shortcut `[ref]`. `[label](url)` is resolved before this
/// is tried, so a bracket group followed by a URL is never a reference.
pub(crate) fn reference_at(
    characters: &[char],
    index: usize,
    refs: &Definitions,
) -> Option<(String, String, usize)> {
    if refs.links.is_empty() || characters.get(index) != Some(&'[') {
        return None;
    }
    let mut close = index + 1;
    while let Some(character) = characters.get(close).copied() {
        match character {
            ']' => break,
            '[' => return None,
            _ => close += 1,
        }
    }
    if characters.get(close) != Some(&']') {
        return None;
    }
    let label: String = characters[index + 1..close].iter().collect();
    if label.trim().is_empty() {
        return None;
    }
    // `[label][ref]`: the second pair names the definition.
    if characters.get(close + 1) == Some(&'[') {
        let mut end = close + 2;
        while let Some(character) = characters.get(end).copied() {
            match character {
                ']' => break,
                '[' => return None,
                _ => end += 1,
            }
        }
        if characters.get(end) != Some(&']') {
            return None;
        }
        let key: String = characters[close + 2..end].iter().collect();
        let key = if key.trim().is_empty() {
            label.clone()
        } else {
            key
        };
        let url = refs.links.get(&normalise_label(&key))?;
        return Some((label, url.clone(), end + 1));
    }
    let url = refs.links.get(&normalise_label(&label))?;
    Some((label, url.clone(), close + 1))
}

/// `[label](url)` starting at `index` → the label, the address, and where the
/// text after the link starts.
///
/// Only `http` and `https` count: any other scheme would draw as a link that
/// cannot be opened, so its brackets stay as the text they are.
pub(crate) fn link_at(characters: &[char], index: usize) -> Option<(String, String, usize)> {
    let (label, url, after) = link_parts(characters, index)?;
    if label.trim().is_empty() || !is_openable_link(&url) {
        return None;
    }
    Some((label, url, after))
}

/// `![alt](url)` starting at the `[` → its alt text, address and the text after
/// it.
///
/// Any address is accepted here: a picture the app cannot open still has alt
/// text that belongs in the sentence, and the caller only links it when
/// [`is_openable_link`] says so. A local path (`pic.png`) shows its alt text
/// instead of the raw `![]()` syntax.
pub(crate) fn image_at(characters: &[char], index: usize) -> Option<(String, String, usize)> {
    link_parts(characters, index)
}

/// The parts of a `[label](url)` run, whatever the address's scheme.
pub(crate) fn link_parts(characters: &[char], index: usize) -> Option<(String, String, usize)> {
    if characters.get(index) != Some(&'[') {
        return None;
    }
    // The label may hold brackets of its own: `[![图](x)](url)` is a picture
    // wrapped in a link, and the picture's own brackets do not end the label.
    let mut close = index + 1;
    let mut depth = 1usize;
    while let Some(character) = characters.get(close).copied() {
        match character {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        close += 1;
    }
    if characters.get(close) != Some(&']') || characters.get(close + 1) != Some(&'(') {
        return None;
    }
    let label: String = characters[index + 1..close].iter().collect();
    let mut end = close + 2;
    // How many brackets of the address are still open.
    let mut depth = 0usize;
    while let Some(character) = characters.get(end).copied() {
        // A `)` inside a quoted title (`[a](https://x "t)")`) is not the end of
        // the address; the title itself is dropped, as Markdown says it is only
        // a tooltip.
        let quoted = in_title(characters, close + 2, end);
        match character {
            // An escaped marker is the character itself, not the link's end.
            '\\' if !quoted => {
                end += 2;
                continue;
            }
            // A balanced pair belongs to the address: `[a](https://x/(y))` links
            // the whole thing and leaves no `)` behind.
            '(' if !quoted => depth += 1,
            ')' if !quoted => {
                if let Some(open) = depth.checked_sub(1) {
                    depth = open;
                    end += 1;
                    continue;
                }
                // `[a](https://x "标题")`: the address is everything up to the
                // first space, which is where a title would start. In angle
                // brackets (`[a](<地址 带空格>)`) the address is the brackets'
                // contents instead.
                let raw: String = characters[close + 2..end].iter().collect();
                let raw = raw.trim();
                let (url, title) = match raw.strip_prefix('<') {
                    Some(inner) => match inner.split_once('>') {
                        Some((url, rest)) => (url, rest.trim().trim_matches(['"', '\''])),
                        None => (raw, ""),
                    },
                    None => match raw.find(char::is_whitespace) {
                        Some(at) => (&raw[..at], raw[at..].trim().trim_matches(['"', '\''])),
                        None => (raw, ""),
                    },
                };
                // `![](地址 "说明")`: with no alt text the title is what the inline
                // image shows, so it becomes the label.
                let label = if label.trim().is_empty() {
                    title.to_string()
                } else {
                    label
                };
                return Some((label, url.to_string(), end + 1));
            }
            _ => {}
        }
        end += 1;
    }
    None
}

/// Whether a `)` at `end` sits inside a quoted title starting at `from`.
///
/// Quotes open after the address's first space and close at the next matching
/// one, so `[a](https://x "a)b")` keeps its address intact.
pub(crate) fn in_title(characters: &[char], from: usize, end: usize) -> bool {
    let mut quote: Option<char> = None;
    for character in characters.get(from..end).unwrap_or_default() {
        match quote {
            Some(open) if *character == open => quote = None,
            Some(_) => {}
            None if matches!(*character, '"' | '\'') => quote = Some(*character),
            None => {}
        }
    }
    quote.is_some()
}

//! Helpers for the block loop: line joins, list items, table cells, references, footnotes, images and rules.

use super::*;

/// Finish the table in progress, padding every row to the widest one so the grid
/// has a definite column count.
pub(crate) fn close_table(table: &mut Option<Table>, blocks: &mut Vec<Rich>) {
    let Some(mut table) = table.take() else {
        return;
    };
    let columns = std::iter::once(table.header.len())
        .chain(table.rows.iter().map(Vec::len))
        .max()
        .unwrap_or(0);
    table.header.resize(columns, Cell::default());
    for row in &mut table.rows {
        row.resize(columns, Cell::default());
    }
    table.aligns.resize(columns, Align::Left);
    blocks.push(Rich {
        block: Block::Table(table),
        spans: Vec::new(),
        links: Vec::new(),
    });
}

/// Join the source lines of a paragraph or a quote the way Markdown reads them.
///
/// A single line break is a *soft* break: it does not show, and the text wraps
/// to the column instead of to whatever width the model typed at. Two trailing
/// spaces (or a trailing `\`) are a *hard* break and keep the newline, as does
/// an empty line inside a quote.
///
/// A soft break becomes a space between ASCII words and nothing next to a
/// Chinese character, so 「第一行\n第二行」 does not turn into 「第一行 第二行」,
/// while 「abc\ndef」 keeps its space.
pub(crate) fn join_lines(lines: &[String]) -> String {
    let mut out = String::new();
    for (position, raw) in lines.iter().enumerate() {
        // Indentation at the start of a line is layout, not text: a continuation
        // line that happens to be indented reads as one sentence.
        let (text, hard) = strip_break(raw);
        // A lone `\` closing the last line is a character, not a break marker:
        // there is no next line for it to break.
        let text = if position + 1 == lines.len()
            && hard
            && raw.trim_end_matches([' ', '\t']).ends_with('\\')
        {
            raw.trim_start()
        } else {
            text.trim_start()
        };
        out.push_str(text);
        let Some(next) = lines.get(position + 1) else {
            continue;
        };
        let (next_text, _) = strip_break(next);
        if hard || next_text.is_empty() {
            out.push('\n');
            continue;
        }
        // The decision looks past emphasis markers: `**重点**` followed by
        // `普通` is two Chinese words, not a word and a star.
        let before = text
            .trim_end_matches(['*', '_', '~', '`'])
            .chars()
            .last()
            .or_else(|| text.chars().last());
        let after = next_text
            .trim_start_matches(['*', '_', '~', '`'])
            .chars()
            .next()
            .or_else(|| next_text.chars().next());
        if let (Some(before), Some(after)) = (before, after)
            && needs_space(before, after)
        {
            out.push(' ');
        }
    }
    out
}

/// One source line without its break marker, and whether that marker is hard.
pub(crate) fn strip_break(line: &str) -> (&str, bool) {
    if line.ends_with("  ") {
        return (line.trim_end_matches([' ', '\t']), true);
    }
    if let Some(rest) = line.strip_suffix('\\') {
        return (rest, true);
    }
    let text = line.trim_end_matches([' ', '\t']);
    // An empty line is a break of its own: a quote's bare `>` is a blank line,
    // not a place where two lines are glued together.
    (text, text.is_empty())
}

/// Whether a soft break between these two characters reads as a space.
pub(crate) fn needs_space(before: char, after: char) -> bool {
    // No space before closing punctuation, and none between two characters
    // that are both not ASCII: Chinese does not put spaces between words.
    !is_cjk_punct(after) && (before.is_ascii() || after.is_ascii())
}

/// Closing punctuation that takes no space before it.
pub(crate) fn is_cjk_punct(character: char) -> bool {
    matches!(
        character,
        '。' | '，'
            | '、'
            | '；'
            | '：'
            | '！'
            | '？'
            | '）'
            | '」'
            | '』'
            | '》'
            | '〉'
            | '】'
            | '…'
            | '·'
    )
}

/// Whether the line is indented far enough to be code.
pub(crate) fn is_indented(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

/// The line without the four spaces (or one tab) of its indentation.
pub(crate) fn indented_text(line: &str) -> &str {
    line.strip_prefix("    ")
        .or_else(|| line.strip_prefix('\t'))
        .unwrap_or(line)
}

/// Whether an indented line opens a block of its own: a list item, heading,
/// quote, fence or table rule is written indented too, and those win.
pub(crate) fn looks_like_marker(text: &str) -> bool {
    let trimmed = text.trim_start();
    heading(trimmed).is_some()
        || bullet(trimmed).is_some()
        || numbered(trimmed).is_some()
        || quote_line(trimmed).is_some()
        || fence_marker(trimmed).is_some()
        || is_table_rule(trimmed)
        || is_rule(trimmed)
        // Blocks that are recognised before the indented-code rule, so an
        // indented one is not swallowed as the previous list item's text.
        || starts_with_tag(trimmed, "<details")
        || starts_with_tag(trimmed, "<img")
        || starts_with_tag(trimmed, "<pre")
        || starts_with_tag(trimmed, "<table")
        || starts_with_tag(trimmed, "<blockquote")
        || starts_with_tag(trimmed, "<ul")
        || starts_with_tag(trimmed, "<ol")
        || starts_with_tag(trimmed, "<h1")
        || starts_with_tag(trimmed, "<h2")
        || starts_with_tag(trimmed, "<h3")
        || starts_with_tag(trimmed, "<h4")
        || starts_with_tag(trimmed, "<h5")
        || starts_with_tag(trimmed, "<h6")
        || starts_with_tag(trimmed, "<p>")
        || starts_with_tag(trimmed, "<p ")
        || starts_with_tag(trimmed, "<div")
        || is_html_rule(trimmed)
        || image_line(trimmed).is_some()
}

/// A list item whose text is still being collected.
///
/// The text of an item is only known to be complete when the next line turns
/// out not to continue it, so the item is held here until then.
pub(crate) enum Item {
    Task(usize, bool),
    Bullet(usize),
    Numbered(usize, String),
}

/// Finish the list item being collected, if there is one.
pub(crate) fn flush_item(
    item: &mut Option<(Item, Vec<String>)>,
    blocks: &mut Vec<Rich>,
    refs: &Definitions,
) {
    let Some((kind, lines)) = item.take() else {
        return;
    };
    // The lines join like a paragraph's, so an item wrapped over several lines
    // is one item of text and not a second item.
    let cell = styled(&join_lines(&lines), refs);
    let block = match kind {
        Item::Task(level, checked) => Block::Task(level, checked, cell.text),
        Item::Bullet(level) => Block::Bullet(level, cell.text),
        Item::Numbered(level, marker) => {
            // Markdown does not read the number of an item: the first one says
            // where the list starts and the rest count up, which is why a model
            // writing `1.` on every line still draws 1, 2, 3.
            let delimiter = marker.chars().last().unwrap_or('.');
            let marker = number_after(blocks, level, delimiter)
                .map(|last| format!("{}{}", last + 1, delimiter))
                .unwrap_or(marker);
            Block::Numbered(level, marker, cell.text)
        }
    };
    blocks.push(Rich {
        block,
        spans: cell.spans,
        links: cell.links,
    });
}

/// The number of the last item of the numbered list this one continues.
///
/// The blocks in between may be items nested deeper than `level` (a sub-list of
/// the previous item); anything else — a paragraph, a deeper list of bullets, an
/// item at a shallower level, another delimiter — ends the list, so a fresh one
/// starts at whatever number it was written with.
pub(crate) fn number_after(blocks: &[Rich], level: usize, delimiter: char) -> Option<u32> {
    for rich in blocks.iter().rev() {
        match &rich.block {
            Block::Numbered(found, marker, _) => {
                if *found == level {
                    // `1.` and `1)` are two lists, as Markdown says.
                    let digits = marker.strip_suffix(delimiter)?;
                    return digits.parse().ok();
                }
                if *found < level {
                    return None;
                }
            }
            Block::Bullet(found, _) | Block::Task(found, _, _) => {
                if *found <= level {
                    return None;
                }
            }
            _ => return None,
        }
    }
    None
}

/// Whether `line` carries on the text of the list item above it.
pub(crate) fn continues_item(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || !line.starts_with([' ', '\t']) {
        return false;
    }
    // An indented marker is a nested item (or a quote, a heading…), not text.
    !looks_like_marker(trimmed)
}

/// Close the indented code block in progress, if there is one.
pub(crate) fn flush_indented(lines: &mut Vec<String>, blocks: &mut Vec<Rich>) {
    if lines.is_empty() {
        return;
    }
    let body = lines.join("\n");
    lines.clear();
    blocks.push(Rich {
        block: Block::Code(String::new(), body),
        spans: Vec::new(),
        links: Vec::new(),
    });
}

/// A `: 说明` line → the description's text.
///
/// The colon has to be followed by a space (or end the line): `:tada:` is a
/// short code and `:hover` is a CSS pseudo-class, and neither is a definition.
pub(crate) fn definition_item(line: &str) -> Option<&str> {
    // A full-width colon is what a Chinese answer often writes, and it carries
    // its own spacing: `：说明` needs no space after it.
    if let Some(rest) = line.strip_prefix('：') {
        return Some(rest.trim_start());
    }
    let rest = line.strip_prefix(':')?;
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix([' ', '\t']).map(str::trim_start)
}

/// Finish the definition being collected, if there is one.
pub(crate) fn flush_definition(
    definition: &mut Option<(String, Vec<String>)>,
    blocks: &mut Vec<Rich>,
    refs: &Definitions,
) {
    let Some((term, lines)) = definition.take() else {
        return;
    };
    let desc = styled(&join_lines(&lines), refs);
    if term.is_empty() {
        blocks.push(Rich {
            block: Block::Definition(desc.text),
            spans: desc.spans,
            links: desc.links,
        });
        return;
    }
    // The term is its own line above the description, and it is drawn bold: a
    // definition list has no marker column to tell the two apart.
    let term = styled(&term, refs);
    let mut text = term.text.clone();
    text.push('\n');
    let offset = text.len();
    text.push_str(&desc.text);
    let mut spans = term.spans;
    for span in &mut spans {
        span.bold = true;
    }
    // The newline between the two lines is part of the term's run, so the runs
    // still cover the text from the first byte to the last.
    match spans.last_mut() {
        Some(last) => last.len += 1,
        None => spans.push(Span {
            len: 1,
            bold: true,
            ..Span {
                len: 0,
                bold: false,
                italic: false,
                code: false,
                strike: false,
                underline: false,
                marked: false,
                token: None,
            }
        }),
    }
    let mut links = term.links;
    for link in desc.links {
        links.push(Link {
            start: link.start + offset,
            end: link.end + offset,
            url: link.url,
        });
    }
    spans.extend(desc.spans);
    blocks.push(Rich {
        block: Block::Definition(text),
        spans,
        links,
    });
}

/// Close the paragraph in progress, if there is one.
pub(crate) fn flush(paragraph: &mut Vec<String>, blocks: &mut Vec<Rich>, refs: &Definitions) {
    if paragraph.is_empty() {
        return;
    }
    let cell = styled(&join_lines(paragraph), refs);
    paragraph.clear();
    // Text that only held markers the parser drops (`[](地址)`, a comment) has
    // nothing to draw: an empty paragraph would leave a gap.
    if cell.text.is_empty() {
        return;
    }
    blocks.push(Rich {
        block: Block::Paragraph(cell.text),
        spans: cell.spans,
        links: cell.links,
    });
}

/// Close the quote in progress, if there is one.
///
/// The quoted lines are read as Markdown of their own, so a list, a heading or a
/// code block inside a quote keeps its shape instead of being flattened into one
/// line of text. They all end up in the same block, which is what draws the box.
pub(crate) fn flush_quote(quote: &mut Vec<String>, blocks: &mut Vec<Rich>, refs: &Definitions) {
    if quote.is_empty() {
        return;
    }
    let inner = parse_rich_with(&quote.join("\n"), refs);
    quote.clear();
    blocks.push(Rich {
        block: Block::Quote(inner),
        spans: Vec::new(),
        links: Vec::new(),
    });
}

/// The text a `>` line adds to a quote, `None` when the line is not one.
///
/// A bare `>` is an empty line inside the quote; the single space after the
/// marker is the marker's own, so it is dropped.
///
/// Only one marker comes off: `> > 引用` keeps its second `>`, so parsing the
/// quoted lines again gives a quote inside the quote, and the view can tell the
/// two boxes apart by their depth. `> >` on its own is a blank line in the inner
/// quote, not an empty box.
pub(crate) fn quote_line(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    if rest.trim().chars().all(|marker| marker == '>') {
        return Some("");
    }
    Some(rest)
}

/// The marker a fence line is made of, `None` when the line is not one.
///
/// Three backticks or three tildes, as Markdown allows; the rest of the line is
/// the language tag.
pub(crate) fn fence_marker(line: &str) -> Option<char> {
    let line = line.trim_start();
    ['`', '~'].into_iter().find(|&marker| {
        line.chars()
            .take_while(|character| *character == marker)
            .count()
            >= 3
    })
}

/// How far a fence is indented, in columns.
///
/// A fence written under a list item or a quote is indented with it; its body
/// carries the same indentation as layout, so the opening line's own indent is
/// taken off every line inside. A tab counts as four columns.
pub(crate) fn fence_indent(line: &str) -> usize {
    let mut columns = 0;
    for character in line.chars() {
        match character {
            ' ' => columns += 1,
            '\t' => columns += 4,
            _ => break,
        }
    }
    columns
}

/// `line` with up to `indent` columns of leading space taken off.
pub(crate) fn without_indent(line: &str, indent: usize) -> &str {
    let mut rest = line;
    let mut taken = 0;
    while taken < indent {
        match rest.chars().next() {
            Some(' ') => {
                rest = &rest[1..];
                taken += 1;
            }
            Some('\t') => {
                rest = &rest[1..];
                taken += 4;
            }
            _ => break,
        }
    }
    rest
}

/// A `[label]: url` line → the normalised label and the address it points at.
///
/// Only `http` / `https` definitions count, for the same reason inline links do
/// not: anything else would draw as a link that cannot be opened.
pub(crate) fn reference_definition(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let close = rest.find(']')?;
    let label = rest.get(..close)?;
    if label.is_empty() || label.contains(['[', ']']) {
        return None;
    }
    let after = rest.get(close + 1..)?.strip_prefix(':')?;
    // A title may follow the address; only the first word is the address.
    let url = after.split_whitespace().next()?;
    let url = url.trim_start_matches('<').trim_end_matches('>');
    if !is_openable_link(url) {
        return None;
    }
    Some((normalise_label(label), url.to_string()))
}

/// A label as it is compared: case and runs of whitespace do not matter.
pub(crate) fn normalise_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// `<hr>`, `<hr/>` or `<hr …>` on a line of its own → true.
pub(crate) fn is_html_rule(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower == "<hr>" || lower == "<hr/>" || lower == "<hr />" || lower.starts_with("<hr ")
}

/// `<img src="…" alt="…">` alone on a line → the caption and the address.
pub(crate) fn html_image(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.to_ascii_lowercase().starts_with("<img") {
        return None;
    }
    let end = trimmed.find('>')?;
    let attributes = &trimmed[..end];
    let src = attribute_value(attributes, "src")?;
    let alt = attribute_value(attributes, "alt").unwrap_or_default();
    let title = attribute_value(attributes, "title").unwrap_or_default();
    let caption = if alt.trim().is_empty() { title } else { alt };
    Some((caption, src))
}

/// `![说明](地址)` and nothing else on the line → the alt text, the address and
/// the quoted title, if any.
pub(crate) fn image_line(line: &str) -> Option<(String, String, String)> {
    let inner = line.strip_prefix("![")?.strip_suffix(')')?;
    let (alt, rest) = inner.split_once("](")?;
    if alt.contains('[') {
        return None;
    }
    let rest = rest.trim();
    // `![图](x.png "说明")`: the title is a tooltip in Markdown, and here it is
    // also the caption when there is no alt text. Only a quoted run is a title:
    // a local path may hold spaces of its own (`![图](my file.png)`).
    let (url, title) = match rest.find(char::is_whitespace) {
        Some(at) if rest[at..].trim_start().starts_with(['"', '\'']) => (
            rest[..at].trim(),
            rest[at..].trim().trim_matches(['"', '\'']).to_string(),
        ),
        _ => (rest, String::new()),
    };
    let url = url.trim_start_matches('<').trim_end_matches('>');
    if !balanced_parens(url) {
        return None;
    }
    Some((alt.trim().to_string(), url.to_string(), title))
}

/// Whether every `)` in `text` has an `(` of its own: an address may hold a
/// balanced pair (`A_(b)`), and the closing one is then part of it.
pub(crate) fn balanced_parens(text: &str) -> bool {
    let mut depth = 0usize;
    for character in text.chars() {
        match character {
            '(' => depth += 1,
            ')' => match depth.checked_sub(1) {
                Some(left) => depth = left,
                None => return false,
            },
            _ => {}
        }
    }
    depth == 0
}

/// Drop a leading YAML front matter block.
///
/// Answers written for a file sometimes start with `---` / `key: value` / `---`.
/// Markdown has no front matter, so it would draw a rule and then a paragraph of
/// `key: value` lines. The block is dropped only when it is the very first line,
/// a closing `---` exists, and every line between reads as `key: value` — an
/// answer that simply opens with a rule does not match.
pub(crate) fn strip_front_matter(text: &str) -> &str {
    let mut offset = 0;
    let mut opened = false;
    let mut keys = 0usize;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if !opened {
            if body.trim_end() != "---" {
                return text;
            }
            opened = true;
            offset += line.len();
            continue;
        }
        if body.trim_end() == "---" {
            // A block of nothing but `---` is two rules, not front matter.
            if keys == 0 {
                return text;
            }
            return &text[offset + line.len()..];
        }
        if body.trim().is_empty() {
            offset += line.len();
            continue;
        }
        let is_key = body.split_once(':').is_some_and(|(key, _)| {
            key.chars()
                .next()
                .is_some_and(|first| first.is_alphabetic() || first == '_')
                && !key.contains(char::is_whitespace)
        });
        if !is_key {
            return text;
        }
        keys += 1;
        offset += line.len();
    }
    text
}

/// Replace `^[说明]` with a reference to a note defined right there.
///
/// Markdown's inline footnote has no definition line: the note's text sits where
/// it is referred to and is drawn with the other notes at the end. Rewriting it
/// to the `[^label]` form lets one numbering pass cover both kinds, in the order
/// they are referred to.
///
/// Code is left alone: a fence, an inline code span and a definition's own text
/// keep their `^[` as characters.
pub(crate) fn rewrite_inline_notes(text: &str) -> (String, Vec<(String, String)>) {
    let mut notes: Vec<(String, String)> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut fence: Option<char> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(marker) = fence {
            if fence_marker(line) == Some(marker) {
                fence = None;
            }
            lines.push(raw.to_string());
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
            lines.push(raw.to_string());
            continue;
        }
        // A definition's text is a note already; a `^[` inside it is the note's
        // own words, not another note.
        if footnote_definition(line).is_some() {
            lines.push(raw.to_string());
            continue;
        }
        lines.push(rewrite_line_notes(raw, &mut notes));
    }
    (lines.join("\n"), notes)
}

/// One line with its `^[说明]` runs replaced, code spans left as they are.
fn rewrite_line_notes(line: &str, notes: &mut Vec<(String, String)>) -> String {
    let characters: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        match character {
            '`' => {
                let ticks = marker_run(&characters, index, '`');
                match code_span(&characters, index, ticks) {
                    Some((_, after)) => {
                        out.extend(characters[index..after].iter());
                        index = after;
                    }
                    None => {
                        for _ in 0..ticks {
                            out.push('`');
                        }
                        index += ticks;
                    }
                }
            }
            '\\' => {
                out.push('\\');
                if let Some(next) = characters.get(index + 1).copied() {
                    out.push(next);
                }
                index += 2;
            }
            '^' if characters.get(index + 1) == Some(&'[') => {
                match inline_note_text(&characters, index) {
                    Some((text, after)) => {
                        // A label no answer can write: the `\u{1}` keeps it out
                        // of the way of `[^1]` and friends.
                        let label = format!("\u{1}inline{}", notes.len());
                        out.push_str(&format!("[^{label}]"));
                        notes.push((label, text));
                        index = after;
                    }
                    None => {
                        out.push('^');
                        index += 1;
                    }
                }
            }
            _ => {
                out.push(character);
                index += 1;
            }
        }
    }
    out
}

/// A `^[说明]` at `index` → the note's text and where the text after it starts.
fn inline_note_text(characters: &[char], index: usize) -> Option<(String, usize)> {
    let mut end = index + 2;
    let mut depth = 1usize;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '\\' => {
                end += 2;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let text: String = characters[index + 2..end].iter().collect();
                    let text = text.trim().to_string();
                    // `^[]` is an empty note: it would draw a number with
                    // nothing to read, so it stays as written.
                    if text.is_empty() {
                        return None;
                    }
                    return Some((text, end + 1));
                }
            }
            _ => {}
        }
        end += 1;
    }
    None
}

/// A `[^label]: text` line → the label and the note's first line.
pub(crate) fn footnote_definition(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix("[^")?;
    let close = rest.find(']')?;
    let label = rest.get(..close)?;
    if label.is_empty() || label.chars().any(char::is_whitespace) {
        return None;
    }
    // The colon may be full-width, and models sometimes put a space before it.
    let after = rest.get(close + 1..)?.trim_start();
    let after = after
        .strip_prefix(':')
        .or_else(|| after.strip_prefix('：'))?;
    Some((label.to_string(), after.trim().to_string()))
}

/// The `[^label]` labels a line mentions, in order.
pub(crate) fn note_labels(line: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("[^") {
        let after = &rest[at + 2..];
        let Some(end) = after.find(']') else {
            break;
        };
        let label = &after[..end];
        if !label.is_empty() && !label.contains('[') {
            labels.push(label.to_string());
        }
        rest = &after[end + 1..];
    }
    labels
}

/// The footnotes an answer defines, and which lines they were written on.
///
/// `inline` holds the notes written in place as `^[说明]` (already rewritten to
/// references by [`rewrite_inline_notes`]); they take part in the numbering but
/// are not tied to a line of their own.
///
/// A definition is one line; the indented lines under it carry on the same note,
/// and may hold blocks of their own (a list, a fence, a table). The lines are
/// marked as taken because the notes are drawn together at the end instead of
/// where they were written.
pub(crate) fn footnote_definitions(
    text: &str,
    inline: Vec<(String, String)>,
) -> (Vec<(String, String)>, Vec<bool>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut taken = vec![false; lines.len()];
    // Each note keeps the lines it was written as; they are dedented and joined
    // once the note is complete, so a list or a fence inside it keeps its shape.
    let mut notes: Vec<(String, Vec<String>)> = inline
        .into_iter()
        .map(|(label, text)| (label, vec![text]))
        .collect();
    let mut fence: Option<char> = None;
    // The note an indented line would carry on, if the line above was one.
    let mut open: Option<usize> = None;
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        if let Some(marker) = fence {
            let closes = fence_marker(line) == Some(marker);
            // A fence written inside a note is part of the note.
            if let Some(at) = open {
                taken[index] = true;
                notes[at].1.push(line.to_string());
            }
            if closes {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            if let Some(at) = open {
                taken[index] = true;
                notes[at].1.push(line.to_string());
            }
            fence = Some(marker);
            continue;
        }
        if let Some((label, first)) = footnote_definition(line) {
            taken[index] = true;
            let label = normalise_label(&label);
            match notes.iter().position(|(found, _)| *found == label) {
                // A label defined twice is drawn once, where it was first defined.
                Some(at) => open = Some(at),
                None => {
                    notes.push((label, vec![first]));
                    open = Some(notes.len() - 1);
                }
            }
            continue;
        }
        let Some(at) = open else {
            continue;
        };
        // A blank line may sit between blocks of the same note: the next line
        // decides whether the note goes on.
        if line.trim().is_empty() {
            continue;
        }
        // Only an indented line carries the note on: a paragraph written right
        // under a definition is a paragraph. An indented list, fence or table is
        // part of the note.
        if line.starts_with([' ', '\t']) {
            taken[index] = true;
            notes[at].1.push(line.to_string());
            continue;
        }
        open = None;
    }
    // Numbers follow the text: a note is numbered where it is first referred to,
    // so `[^1]` stays 1 even when another note is defined above it. Notes nothing
    // refers to keep the order they were defined in.
    let mut order: Vec<String> = Vec::new();
    let mut fence: Option<char> = None;
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        if let Some(marker) = fence {
            if fence_marker(line) == Some(marker) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
            continue;
        }
        if taken.get(index).copied().unwrap_or(false) {
            continue;
        }
        for label in note_labels(line) {
            let label = normalise_label(&label);
            if !order.contains(&label) && notes.iter().any(|(found, _)| *found == label) {
                order.push(label);
            }
        }
    }
    for (label, _) in &notes {
        if !order.contains(label) {
            order.push(label.clone());
        }
    }
    let mut numbered: Vec<(String, String)> = Vec::with_capacity(notes.len());
    for label in order {
        if let Some(at) = notes.iter().position(|(found, _)| *found == label) {
            let (label, lines) = notes.remove(at);
            numbered.push((label, note_text(&lines)));
        }
    }
    (numbered, taken)
}

/// A note's collected lines as the Markdown of its own blocks.
///
/// The first line is the definition's text; the lines under it are indented to
/// show they belong to the note, so their common indentation is taken off before
/// they are read — otherwise every note holding a list would draw it as code.
pub(crate) fn note_text(lines: &[String]) -> String {
    let mut out = String::new();
    let mut rest = lines;
    if let Some((first, tail)) = lines.split_first() {
        out.push_str(first.trim_start());
        rest = tail;
    }
    let indent = rest
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    for line in rest {
        out.push('\n');
        if line.trim().is_empty() {
            continue;
        }
        out.push_str(line.get(indent..).unwrap_or(line));
    }
    out
}

/// Every reference definition in an answer, collected before its first line is
/// read so a link can point at a definition below it.
///
/// Code is not searched: a `[x]: url` inside a fence is part of the code.
pub(crate) fn reference_definitions(text: &str) -> Refs {
    let mut refs = Refs::new();
    let mut fence: Option<char> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        if let Some(marker) = fence {
            if fence_marker(line) == Some(marker) {
                fence = None;
            }
            continue;
        }
        if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
            continue;
        }
        if let Some((label, url)) = reference_definition(line) {
            // The first definition of a label wins, as in Markdown.
            refs.entry(label).or_insert(url);
        }
    }
    refs
}

/// The language tag after a fence, empty when the line has none.
/// The info string can carry more than a language (`rust title="x"`), and
/// `rust,no_run` is still Rust: the label is its first word.
pub(crate) fn language_of(line: &str) -> String {
    line.trim_start()
        .trim_start_matches(['`', '~'])
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_end_matches([',', ';'])
        .to_string()
}

/// `## Title` → `(2, "Title")`.
pub(crate) fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = line[level..].strip_prefix(' ')?;
    let rest = rest.trim_end();
    // A closing run of `#`s only ends the heading when a space comes before it,
    // so `# C#` keeps its sharp and `## 标题 ##` loses the decoration.
    let closed = rest.trim_end_matches('#');
    let text = if closed.is_empty() {
        closed
    } else if closed.ends_with([' ', '\t']) {
        closed.trim_end()
    } else {
        rest
    };
    Some((level, text))
}

/// `===` or `---` underlining the line above it → the heading's level.
///
/// Only the marker on its own, without gaps: `- - -` is a section rule, not an
/// underline.
pub(crate) fn setext_underline(line: &str) -> Option<usize> {
    let line = line.trim_end();
    if !line.is_empty() && line.chars().all(|character| character == '=') {
        return Some(1);
    }
    if !line.is_empty() && line.chars().all(|character| character == '-') {
        return Some(2);
    }
    None
}

/// `- item`, `* item`, `+ item` → `"item"`.
pub(crate) fn bullet(line: &str) -> Option<&str> {
    let marker = line.chars().next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    // A tab after the marker is a separator too: `-\t项` is an item.
    line[1..].strip_prefix([' ', '\t'])
}

/// `- [ ] item` / `- [x] item` → whether it is ticked and the item's text.
///
/// `X` counts as a tick too: models write both. An item with nothing after the
/// box is still a task, an empty one.
pub(crate) fn task_item(line: &str) -> Option<(bool, &str)> {
    let rest = bullet(line)?.strip_prefix('[')?;
    let (checked, rest) = match rest.chars().next()? {
        ' ' => (false, &rest[1..]),
        'x' | 'X' => (true, &rest[1..]),
        _ => return None,
    };
    let rest = rest.strip_prefix(']')?;
    // `- [x]item` is close enough; Markdown asks for a space, a model dropping
    // it should still read as a task.
    Some((checked, rest.trim_start()))
}

/// Deepest list nesting the transcript draws.
///
/// A deeper item is drawn at this level rather than pushing its text off the
/// column; six levels keep a deeply nested outline readable in the bubble.
pub(crate) const MAX_LIST_LEVEL: usize = 6;

/// How deeply a list item is indented: two spaces per level.
///
/// A tab counts as two spaces. This is only a hint at the nesting:
/// [`nested_level`] turns it into one level per step, so four spaces under an
/// item still nest once.
pub(crate) fn list_level(line: &str) -> usize {
    let mut spaces = 0;
    for character in line.chars() {
        match character {
            ' ' => spaces += 1,
            '\t' => spaces += 2,
            _ => break,
        }
    }
    (spaces / 2).min(MAX_LIST_LEVEL)
}

/// How deep an item really sits, given the block drawn before it.
///
/// Indentation only says "deeper than the item above": models indent a nested
/// item by two, three or four spaces and all of them mean one level, so a jump
/// of several indentation steps still nests once. An item with no item above it
/// is top level, however far it happens to be indented.
pub(crate) fn nested_level(blocks: &[Rich], level: usize) -> usize {
    let previous = match blocks.last().map(|rich| &rich.block) {
        Some(Block::Bullet(found, _) | Block::Task(found, _, _) | Block::Numbered(found, _, _)) => {
            *found
        }
        _ => return 0,
    };
    level.min(previous + 1).min(MAX_LIST_LEVEL)
}

/// `12. item` / `3) item` → `("12.", "item")` / `("3)", "item")`.
///
/// The marker has to end at a space or at the end of the line: `3.14 是圆周率`
/// is a sentence about a number, not the third item of a list. The delimiter is
/// kept as written because Markdown starts a new list when it changes.
pub(crate) fn numbered(line: &str) -> Option<(String, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let marker = rest.chars().next()?;
    if !matches!(marker, '.' | ')') {
        return None;
    }
    let after = &rest[marker.len_utf8()..];
    let body = if after.is_empty() {
        ""
    } else {
        after.strip_prefix([' ', '\t'])?
    };
    Some((format!("{}{}", &line[..digits], marker), body.trim_start()))
}

/// `---`, `***` or `___` on a line of its own → true.
///
/// Three of the same marker, spaces allowed between them. A table's rule has
/// pipes in it, so it is not one.
pub(crate) fn is_rule(line: &str) -> bool {
    let mut marker: Option<char> = None;
    let mut count = 0;
    for character in line.trim().chars() {
        if character == ' ' {
            continue;
        }
        if !matches!(character, '-' | '*' | '_') {
            return false;
        }
        match marker {
            Some(chosen) if chosen != character => return false,
            None => marker = Some(character),
            _ => {}
        }
        count += 1;
    }
    count >= 3
}

/// `|---|---|` or `| :-- | --: |` → true: the line between a table's header and
/// its rows.
///
/// A rule is any line built only from pipes, dashes, colons and spaces, with at
/// least one dash. Models sometimes drop the trailing pipe, so that is allowed.
pub(crate) fn is_table_rule(line: &str) -> bool {
    let line = line.trim();
    line.contains('|')
        && line.contains('-')
        && line
            .chars()
            .all(|character| matches!(character, '|' | '-' | ':' | ' '))
}

/// `| :-- | --: | :-: |` → how each column lines up.
///
/// The rule line is the only place Markdown states this, so its colons are read
/// just before the line itself is dropped.
pub(crate) fn aligns_of(line: &str) -> Vec<Align> {
    let line = line.trim();
    let inner = line.strip_prefix('|').unwrap_or(line);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner
        .split('|')
        .map(|cell| {
            let cell = cell.trim();
            match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            }
        })
        .collect()
}

/// `| a | b |` → `["a", "b"]`: the row's cells, with the pipe padding evened out.
///
/// The outer pipes are optional the way Markdown allows them to be, so `a | b`
/// counts as a row too; a line with a single cell is only a row when it is
/// wrapped in pipes (`| a |`), which keeps a stray `|` in prose from opening one.
pub(crate) fn cells_of(line: &str, refs: &Definitions) -> Option<Vec<Cell>> {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return None;
    }
    let mut inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    // A trailing pipe closes the row unless it was escaped.
    if inner.ends_with('|') && !inner.ends_with("\\|") {
        inner = &inner[..inner.len() - 1];
    }
    let cells: Vec<Cell> = split_cells(inner)
        .iter()
        .map(|cell| styled_cell_with(cell, refs))
        .collect();
    if cells.len() < 2 && !trimmed.starts_with('|') {
        return None;
    }
    Some(cells)
}

/// The block-level HTML elements the answer draws as blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HtmlBlock {
    /// `<pre>…</pre>`: a code block.
    Pre,
    /// `<table>…</table>`: a table.
    Table,
    /// `<blockquote>…</blockquote>`: a quote.
    Quote,
    /// `<ul>…</ul>` / `<ol>…</ol>`: a list.
    List(bool),
    /// `<h1>…</h1>` … `<h6>…</h6>`: a heading, with its level.
    Heading(usize),
    /// `<p>…</p>`: a paragraph.
    Paragraph,
    /// `<div>…</div>`: the blocks inside it.
    Div,
}

/// The element `line` opens, if it is one of the block-level ones.
pub(crate) fn html_block_start(line: &str) -> Option<HtmlBlock> {
    let lower = line.trim_start().to_ascii_lowercase();
    let opens = |tag: &str| {
        lower
            .strip_prefix(tag)
            .is_some_and(|rest| rest.starts_with(['>', ' ', '\t', '/']))
    };
    if opens("<pre") {
        Some(HtmlBlock::Pre)
    } else if opens("<table") {
        Some(HtmlBlock::Table)
    } else if opens("<blockquote") {
        Some(HtmlBlock::Quote)
    } else if opens("<ul") {
        Some(HtmlBlock::List(false))
    } else if opens("<ol") {
        Some(HtmlBlock::List(true))
    } else if opens("<p") {
        Some(HtmlBlock::Paragraph)
    } else if opens("<div") {
        Some(HtmlBlock::Div)
    } else if lower.starts_with("<h") {
        // `<h1>` … `<h6>`: the digit is the level.
        let level = lower
            .strip_prefix("<h")
            .and_then(|rest| rest.chars().next())
            .and_then(|digit| digit.to_digit(10))
            .filter(|level| (1..=6).contains(level))?;
        let after = 2 + 1;
        if lower
            .get(after..)
            .is_some_and(|rest| rest.starts_with(['>', ' ', '\t']))
        {
            Some(HtmlBlock::Heading(level as usize))
        } else {
            None
        }
    } else {
        None
    }
}

/// The tag that closes an HTML block.
pub(crate) fn html_block_close(block: HtmlBlock) -> &'static str {
    match block {
        HtmlBlock::Pre => "</pre>",
        HtmlBlock::Table => "</table>",
        HtmlBlock::Quote => "</blockquote>",
        HtmlBlock::List(true) => "</ol>",
        HtmlBlock::List(false) => "</ul>",
        HtmlBlock::Heading(1) => "</h1>",
        HtmlBlock::Heading(2) => "</h2>",
        HtmlBlock::Heading(3) => "</h3>",
        HtmlBlock::Heading(4) => "</h4>",
        HtmlBlock::Heading(5) => "</h5>",
        HtmlBlock::Heading(_) => "</h6>",
        HtmlBlock::Paragraph => "</p>",
        HtmlBlock::Div => "</div>",
    }
}

/// What is between an element's opening tag and its closing tag.
pub(crate) fn html_inner<'a>(text: &'a str, close: &str) -> &'a str {
    let lower = text.to_ascii_lowercase();
    let start = lower.find('>').map(|at| at + 1).unwrap_or(0);
    let end = lower.rfind(close).unwrap_or(text.len());
    text.get(start..end).unwrap_or("")
}

/// `<pre>…</pre>` → the language a `<code class="language-…">` names and the
/// code itself.
pub(crate) fn html_pre(text: &str) -> (String, String) {
    let mut body = html_inner(text, "</pre>");
    let mut language = String::new();
    let lower = body.to_ascii_lowercase();
    if let Some(at) = lower.find("<code") {
        if let Some(close) = lower[at..].find('>') {
            let tag = &body[at..at + close + 1];
            language = attribute_value(tag, "class")
                .unwrap_or_default()
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if let Some(name) = language.strip_prefix("language-") {
                language = name.to_string();
            }
            body = &body[at + close + 1..];
        }
        if let Some(end) = body.to_ascii_lowercase().rfind("</code>") {
            body = &body[..end];
        }
    }
    let code = decode_entities(body);
    // The newlines right inside the element are its own layout, not code; the
    // indentation of the first and last line is code and is kept.
    (
        language.trim().to_string(),
        code.trim_matches(['\n', '\r']).to_string(),
    )
}

/// `<table>…</table>` → its header row and the rows under it.
///
/// A row of `<th>` cells is the header; when no row says so, the first row is
/// used, because a table has to have one. Cell markup is stripped and what is
/// left is read as inline Markdown, so `**粗**` and `[链接](url)` still work.
pub(crate) fn html_table(text: &str, refs: &Definitions) -> Option<Table> {
    let body = html_inner(text, "</table>").to_string();
    let mut rows: Vec<(bool, Vec<Cell>)> = Vec::new();
    let mut rest = body.as_str();
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(at) = lower.find("<tr") else {
            break;
        };
        let after_open = lower[at..].find('>')?;
        let content = &rest[at + after_open + 1..];
        let lower_content = content.to_ascii_lowercase();
        let Some(end) = lower_content.find("</tr>") else {
            break;
        };
        let row = &content[..end];
        let (header, cells) = html_row(row, refs);
        if !cells.is_empty() {
            rows.push((header, cells));
        }
        rest = &content[end + "</tr>".len()..];
    }
    if rows.is_empty() {
        return None;
    }
    // The first row is the header, whether it used `<th>` or not: a table has to
    // have one.
    let (_, header) = rows.remove(0);
    let rows = rows;
    let columns = std::iter::once(header.len())
        .chain(rows.iter().map(|(_, cells)| cells.len()))
        .max()
        .unwrap_or(0);
    let mut header = header;
    header.resize(columns, Cell::default());
    let rows: Vec<Vec<Cell>> = rows
        .into_iter()
        .map(|(_, mut cells)| {
            cells.resize(columns, Cell::default());
            cells
        })
        .collect();
    Some(Table {
        header,
        rows,
        aligns: vec![Align::Left; columns],
    })
}

/// One `<tr>` → whether its cells are header cells, and the cells themselves.
pub(crate) fn html_row(row: &str, refs: &Definitions) -> (bool, Vec<Cell>) {
    let mut cells = Vec::new();
    let mut header = false;
    let mut rest = row;
    loop {
        let lower = rest.to_ascii_lowercase();
        let cell_at = lower.find("<td").into_iter().chain(lower.find("<th")).min();
        let Some(at) = cell_at else {
            break;
        };
        let is_header = lower[at..].starts_with("<th");
        header |= is_header;
        let Some(open_end) = lower[at..].find('>') else {
            break;
        };
        let content_start = at + open_end + 1;
        let close = if is_header { "</th>" } else { "</td>" };
        let Some(end) = lower[content_start..].find(close) else {
            break;
        };
        let content = &rest[content_start..content_start + end];
        let text = decode_entities(&strip_html_tags(content));
        cells.push(styled_cell_with(&text, refs));
        rest = &rest[content_start + end + close.len()..];
    }
    (header, cells)
}

/// `<ul>` / `<ol>` → its `<li>` items, one block each.
pub(crate) fn html_list(text: &str, ordered: bool, refs: &Definitions) -> Vec<Rich> {
    let close = if ordered { "</ol>" } else { "</ul>" };
    let body = html_inner(text, close).to_string();
    let mut items = Vec::new();
    let mut rest = body.as_str();
    let mut number = 0usize;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(at) = lower.find("<li") else {
            break;
        };
        let Some(open_end) = lower[at..].find('>') else {
            break;
        };
        let content_start = at + open_end + 1;
        let Some(end) = lower[content_start..].find("</li>") else {
            break;
        };
        let content = &rest[content_start..content_start + end];
        let text = decode_entities(&strip_html_tags(content));
        let cell = styled(&text, refs);
        number += 1;
        let block = if ordered {
            Block::Numbered(0, format!("{number}."), cell.text)
        } else {
            Block::Bullet(0, cell.text)
        };
        items.push(Rich {
            block,
            spans: cell.spans,
            links: cell.links,
        });
        rest = &rest[content_start + end + "</li>".len()..];
    }
    items
}

/// `<h1>`…`<h6>` / `<p>` → one block of styled text.
pub(crate) fn html_text_block(
    text: &str,
    close: &str,
    level: Option<usize>,
    refs: &Definitions,
) -> Option<Rich> {
    let inner = html_inner(text, close);
    let text = decode_entities(&strip_html_tags(inner));
    if text.trim().is_empty() {
        return None;
    }
    let cell = styled(&text, refs);
    let block = match level {
        Some(level) => Block::Heading(level, cell.text),
        None => Block::Paragraph(cell.text),
    };
    Some(Rich {
        block,
        spans: cell.spans,
        links: cell.links,
    })
}

/// `text` with its tags taken out; `<br>` becomes a space so words do not run
/// together.
pub(crate) fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut link: Option<String> = None;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find('>') else {
            // An unclosed `<` is text.
            out.push_str(&rest[at..]);
            return out.trim().to_string();
        };
        let tag = &rest[at..=at + end];
        let lower = tag.to_ascii_lowercase();
        match lower.as_str() {
            // The tags that only say how the text looks become the Markdown that
            // says the same, so a cell keeps its bold and code.
            "<b>" | "<strong>" | "</b>" | "</strong>" => out.push_str("**"),
            "<i>" | "<em>" | "</i>" | "</em>" => out.push('*'),
            "<code>" | "</code>" => out.push('`'),
            // `<br>`, `<br/>`: a line break keeps the words on either side apart.
            "<br>" | "<br/>" | "<br />" => out.push(' '),
            "</a>" => {
                if let Some(href) = link.take() {
                    out.push_str("](");
                    out.push_str(&href);
                    out.push(')');
                }
            }
            _ => {
                if lower.starts_with("<a ") || lower.starts_with("<a\t") {
                    if let Some(href) = attribute_value(tag, "href")
                        && is_openable_link(&href)
                    {
                        out.push('[');
                        link = Some(href);
                    }
                } else if lower.starts_with("<img") {
                    // A picture in a cell has no room; its alt text stands in.
                    let alt = attribute_value(tag, "alt").unwrap_or_default();
                    if !alt.trim().is_empty() {
                        out.push_str(alt.trim());
                    }
                }
            }
        }
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// `&amp;` and the like turned into the characters they stand for.
pub(crate) fn decode_entities(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        if character == '&'
            && let Some((decoded, after)) = entity_at(&characters, index)
        {
            out.push_str(&decoded);
            index = after;
            continue;
        }
        out.push(character);
        index += 1;
    }
    out
}

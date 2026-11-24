//! The inline loop itself: one line of text into a cell of runs, plus code spans, entities and run flushing.

use super::*;

/// Read one run of ordinary text: `**` opens and closes bold, backticks open and
/// close inline code, and the markers themselves never reach the screen.
pub(crate) fn styled(raw: &str, refs: &Definitions) -> Cell {
    let characters: Vec<char> = raw.chars().collect();
    inline_cell(&characters, true, refs)
}

/// The same, over characters already split, with links optional.
///
/// A link's label is read with `allow_links` off: a link inside a link is not a
/// thing, and it keeps the recursion at one level.
pub(crate) fn inline_cell(characters: &[char], allow_links: bool, refs: &Definitions) -> Cell {
    let mut cell = Cell::default();
    let mut run = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut code = false;
    let mut strike = false;
    let mut underline = false;
    let mut mono = false;
    let mut marked = false;
    // An `<a href>` whose label is still being read: the link's index in
    // `cell.links` and where it points.
    let mut anchor: Option<(usize, String)> = None;
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        match character {
            // `\*` and friends are the character itself: an answer that quotes
            // Markdown should not lose its asterisks.
            '\\' if !code => match characters.get(index + 1).copied() {
                // `\(…\)` is an inline formula, the same as `$…$`.
                Some('(') => match inline_paren_formula(characters, index) {
                    Some((text, after)) => {
                        flush_run(
                            &mut cell, &mut run, bold, italic, false, strike, underline, mono,
                            marked,
                        );
                        run.push_str(&text);
                        flush_run(
                            &mut cell, &mut run, bold, italic, true, strike, underline, mono,
                            marked,
                        );
                        index = after;
                    }
                    None => {
                        run.push('(');
                        index += 2;
                    }
                },
                Some(escaped) if is_escapable(escaped) => {
                    run.push(escaped);
                    index += 2;
                }
                _ => {
                    run.push('\\');
                    index += 1;
                }
            },
            '`' => {
                let ticks = marker_run(characters, index, '`');
                if code {
                    // A run opened by one backtick closes at the next one.
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    code = false;
                    index += 1;
                    continue;
                }
                match code_span(characters, index, ticks) {
                    // The span is taken whole, so nothing inside it is read as a
                    // marker: a backtick of its own stops being special.
                    Some((content, after)) => {
                        flush_run(
                            &mut cell, &mut run, bold, italic, false, strike, underline, mono,
                            marked,
                        );
                        run.push_str(&content);
                        flush_run(
                            &mut cell, &mut run, bold, italic, true, strike, underline, mono,
                            marked,
                        );
                        index = after;
                    }
                    // One backtick with nothing to close it opens a run the old
                    // way, so a half-written span still ends up as code.
                    None if ticks == 1 => {
                        flush_run(
                            &mut cell, &mut run, bold, italic, code, strike, underline, mono,
                            marked,
                        );
                        code = true;
                        index += 1;
                    }
                    // A longer run has no closing partner: it is just backticks.
                    None => {
                        for _ in 0..ticks {
                            run.push('`');
                        }
                        index += ticks;
                    }
                }
            }
            // `![说明](地址)` is a picture. Nothing here fetches one, so its alt
            // text is drawn in the sentence; a caption that points at an
            // openable address stays a link. A local path shows its alt text
            // too, instead of the raw `![]()` syntax. The picture is read even
            // inside a link's label, where only its text belongs (a link inside
            // a link is not a thing).
            '[' if !code && index > 0 && characters[index - 1] == '!' => {
                let Some((alt, url, after)) =
                    image_at(characters, index).or_else(|| reference_at(characters, index, refs))
                else {
                    run.push('[');
                    index += 1;
                    continue;
                };
                run.pop();
                // A picture with no alt text draws nothing, so it must not leave
                // the space on both sides of it behind either.
                if alt.trim().is_empty() {
                    if characters.get(after) == Some(&' ') && run.ends_with(' ') {
                        index = after + 1;
                    } else {
                        index = after;
                    }
                    continue;
                }
                flush_run(
                    &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                );
                let url = (allow_links && is_openable_link(&url)).then_some(url);
                push_label(&mut cell, &alt, url, bold, italic, strike, marked, refs);
                flush_run(
                    &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                );
                index = after;
            }
            // A link is drawn as its label; the address only shows when it is
            // clicked. Inside code the brackets are text. `[a](url)` comes
            // first, then the reference forms `[a][b]`, `[a][]` and `[a]`.
            '[' if !code && allow_links => {
                // `[^label]` is a footnote: it draws as its number, and the note
                // itself is at the end of the answer.
                if let Some((number, after)) = note_reference(characters, index, refs) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    run.push_str(&superscript(number));
                    index = after;
                    continue;
                }
                // `[](地址)` has no text to draw: its brackets are not worth
                // showing, so the whole run is skipped — together with one of the
                // spaces that would otherwise double up around it.
                if let Some((label, _, after)) = link_parts(characters, index)
                    && label.trim().is_empty()
                {
                    if characters.get(after) == Some(&' ') && run.ends_with(' ') {
                        index = after + 1;
                    } else {
                        index = after;
                    }
                    continue;
                }
                let found =
                    link_at(characters, index).or_else(|| reference_at(characters, index, refs));
                let Some((label, url, after)) = found else {
                    run.push('[');
                    index += 1;
                    continue;
                };
                flush_run(
                    &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                );
                push_label(
                    &mut cell,
                    &label,
                    Some(url),
                    bold,
                    italic,
                    strike,
                    marked,
                    refs,
                );
                index = after;
            }
            // A bare address is a link too: models often paste "https://…" or
            // "www.…" straight into the text, and a link that has to be copied
            // into a browser is not much of a link.
            'h' | 'w' if allow_links => {
                let Some((drawn, target, after)) = url_at(characters, index) else {
                    run.push(character);
                    index += 1;
                    continue;
                };
                flush_run(
                    &mut cell, &mut run, bold, italic, false, strike, underline, mono, marked,
                );
                let start = cell.text.len();
                cell.text.push_str(&drawn);
                cell.spans.push(Span {
                    len: drawn.len(),
                    bold,
                    italic,
                    code: false,
                    strike,
                    underline: false,
                    marked: false,
                    token: None,
                });
                cell.links.push(Link {
                    start,
                    end: start + drawn.len(),
                    url: target,
                });
                index = after;
            }
            // `&amp;` and the like: answers escape what they cannot write, and
            // `&amp;` on screen is a worse answer than `&`. Anything that is not
            // a known entity stays as written, so a bare `&` is never eaten.
            '&' if !code => match entity_at(characters, index) {
                Some((text, after)) => {
                    run.push_str(&text);
                    index = after;
                }
                None => {
                    run.push('&');
                    index += 1;
                }
            },
            // `$a^2$`: an inline formula, drawn in the mono font with its LaTeX
            // markers turned into symbols. The opening `$` cannot be followed by
            // a space and the closing one has to be on the same line, so
            // "$5 和 $10" stays prices.
            '$' if !code => match inline_formula(characters, index) {
                Some((text, after)) => {
                    flush_run(
                        &mut cell, &mut run, bold, italic, false, strike, underline, mono, marked,
                    );
                    run.push_str(&text);
                    flush_run(
                        &mut cell, &mut run, bold, italic, true, strike, underline, mono, marked,
                    );
                    index = after;
                }
                None => {
                    run.push('$');
                    index += 1;
                }
            },
            // `*` and `_` both emphasise; inside code they are characters.
            '*' | '_' if !code => {
                // `snake_case` keeps its underscores: see `opens_emphasis`.
                let marks = marker_run(characters, index, character);
                if marks == 1 && italic && closes_emphasis(characters, index, character) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    italic = false;
                    index += 1;
                    continue;
                }
                if marks == 1
                    && !italic
                    && opens_emphasis(characters, index, character)
                    && has_closer(characters, index, character)
                {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    italic = true;
                    index += 1;
                    continue;
                }
                if marks == 1 {
                    // A marker that cannot open or close anything is a
                    // character: a lone `*` must not eat the rest of the line.
                    run.push(character);
                    index += 1;
                    continue;
                }
                // `2**10` is arithmetic, not bold: a pair only opens when
                // another pair closes it. A pair that closes needs no search.
                if !bold && !has_bold_closer(characters, index, character) {
                    for _ in 0..marks {
                        run.push(character);
                    }
                    index += marks;
                    continue;
                }
                flush_run(
                    &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                );
                // `**` is bold; the third mark of `***` adds the italic it
                // stands for.
                bold = !bold;
                if marks >= 3 {
                    italic = !italic;
                }
                index += marks;
            }
            // `==高亮==`: a highlighted run, the same one `<mark>` draws. A lone
            // `=` is a character — `x = 1` and `if a == b` stay code-looking
            // text — and an unmatched pair stays as written.
            '=' if !code => {
                if marked && closes_mark(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    marked = false;
                    index += 2;
                    continue;
                }
                if !marked && opens_mark(characters, index) && has_mark_closer(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    marked = true;
                    index += 2;
                    continue;
                }
                // A longer run (`===`) is not a marker at all.
                let marks = marker_run(characters, index, '=');
                for _ in 0..marks {
                    run.push('=');
                }
                index += marks;
            }
            // `x^2^`: a superscript, drawn as the raised character itself.
            '^' if !code => match script_run(characters, index, '^', true) {
                Some((text, after)) => {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    run.push_str(&text);
                    index = after;
                }
                None => {
                    run.push('^');
                    index += 1;
                }
            },
            // `:smile:`: a short code. Only names on the list are replaced, so a
            // time (`12:30`), a path (`a::b`) and an address keep their colons.
            ':' if !code => match emoji_at(characters, index) {
                Some((emoji, after)) => {
                    run.push_str(emoji);
                    index = after;
                }
                None => {
                    run.push(':');
                    index += 1;
                }
            },
            // `~~struck~~`. A single `~` is a tilde — "5~10 个" is a range, not
            // a marker — and a pair with no closing pair stays as text.
            '~' if !code => {
                let tildes = tilde_run(characters, index);
                if tildes < 2 {
                    // `H~2~O`: a single tilde around text is a subscript, which
                    // is drawn as the lowered character itself.
                    if let Some((text, after)) = script_run(characters, index, '~', false) {
                        flush_run(
                            &mut cell, &mut run, bold, italic, code, strike, underline, mono,
                            marked,
                        );
                        run.push_str(&text);
                        index = after;
                        continue;
                    }
                    run.push('~');
                    index += 1;
                    continue;
                }
                if strike || has_strike_closer(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    strike = !strike;
                    index += 2;
                    continue;
                }
                run.push_str("~~");
                index += 2;
            }
            // `<https://…>` and `<user@example.com>`: autolinks, drawn without
            // their angle brackets. Inside code the brackets are text.
            '<' if !code => {
                // `<!-- … -->` is a note to whoever reads the source.
                if let Some(after) = html_comment_end(characters, index) {
                    index = after;
                    continue;
                }
                // `<img …>`: its alt text (or title) stands in for the picture,
                // and the address becomes a link when it can be opened.
                if let Some((caption, src, after)) = inline_html_image(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    let start = cell.text.len();
                    run.push_str(&caption);
                    if !caption.is_empty() && allow_links && is_openable_link(&src) {
                        // The link is pushed before the run is flushed, so the
                        // caption keeps a span of its own for `Cell::markdown`.
                        cell.links.push(Link {
                            start,
                            end: start + caption.len(),
                            url: src,
                        });
                    }
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    index = after;
                    continue;
                }
                if allow_links && let Some((drawn, target, after)) = angle_link(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    let start = cell.text.len();
                    cell.text.push_str(&drawn);
                    cell.spans.push(Span {
                        len: drawn.len(),
                        bold,
                        italic,
                        code: false,
                        strike,
                        underline: false,
                        marked: false,
                        token: None,
                    });
                    cell.links.push(Link {
                        start,
                        end: start + drawn.len(),
                        url: target,
                    });
                    index = after;
                    continue;
                }
                // `<sup>2</sup>` / `<sub>2</sub>`: a run cannot be raised or
                // lowered, so the characters themselves become script forms.
                if let Some((text, after)) = scripted_tag(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    run.push_str(&text);
                    index = after;
                    continue;
                }
                // `<a href="…">标签</a>` is a link, exactly like `[标签](…)`.
                // The link is pushed when the tag opens, so the label starts its
                // own run; its end is filled in at the closing tag.
                if allow_links && let Some((href, closing, after)) = anchor_tag(characters, index) {
                    flush_run(
                        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
                    );
                    if closing {
                        if let Some((at, url)) = anchor.take() {
                            let start = cell.links[at].start;
                            if cell.text.len() > start && is_openable_link(&url) {
                                cell.links[at].end = cell.text.len();
                            } else {
                                cell.links.remove(at);
                            }
                        }
                    } else if let Some(url) = href {
                        let start = cell.text.len();
                        cell.links.push(Link {
                            start,
                            end: start,
                            url: url.clone(),
                        });
                        anchor = Some((cell.links.len() - 1, url));
                    }
                    index = after;
                    continue;
                }
                // `<br>` and friends: the tag is Markdown's escape hatch for
                // things a line of text cannot say, so it is dropped and its
                // text kept. Anything else stays as it was written.
                match html_tag(characters, index) {
                    Some((name, closing, after)) => {
                        // The tag is syntax, not text: `<br>` is a break and
                        // a styling tag turns its run on — `<b>粗</b>` is bold
                        // because the answer says so, not because of Markdown.
                        if name == "br" && !closing {
                            run.push('\n');
                        } else if let Some(style) = html_style(&name) {
                            flush_run(
                                &mut cell, &mut run, bold, italic, code, strike, underline, mono,
                                marked,
                            );
                            match style {
                                HtmlStyle::Bold => bold = !closing,
                                HtmlStyle::Italic => italic = !closing,
                                HtmlStyle::Strike => strike = !closing,
                                HtmlStyle::Underline => underline = !closing,
                                HtmlStyle::Mono => mono = !closing,
                                HtmlStyle::Mark => marked = !closing,
                            }
                        }
                        index = after;
                    }
                    None => {
                        run.push('<');
                        index += 1;
                    }
                }
            }
            _ => {
                run.push(character);
                index += 1;
            }
        }
    }
    flush_run(
        &mut cell, &mut run, bold, italic, code, strike, underline, mono, marked,
    );
    // An `<a>` that never closed is no link at all.
    if let Some((at, _)) = anchor.take() {
        cell.links.remove(at);
    }
    cell
}

/// A code span starting at `index`, whose opening run is `ticks` backticks →
/// its content and where the text after it starts.
///
/// The closing run has to be the same length, which is what lets a span hold a
/// backtick of its own: double backticks around a word with a single one in it.
/// One space is dropped from each end when both are there, so a span can be
/// written with padding to keep it apart from the delimiters.
pub(crate) fn code_span(
    characters: &[char],
    index: usize,
    ticks: usize,
) -> Option<(String, usize)> {
    let mut cursor = index + ticks;
    while cursor < characters.len() {
        if characters[cursor] != '`' {
            cursor += 1;
            continue;
        }
        let run = marker_run(characters, cursor, '`');
        if run != ticks {
            cursor += run;
            continue;
        }
        let content: String = characters[index + ticks..cursor].iter().collect();
        let padded =
            content.starts_with(' ') && content.ends_with(' ') && !content.trim().is_empty();
        let content = if padded {
            content[1..content.len() - 1].to_string()
        } else {
            content
        };
        return Some((content, cursor + ticks));
    }
    None
}

/// The characters a backslash takes literally: the ASCII punctuation Markdown
/// allows it to escape, so `\^[x]` and `\==高亮==` are the characters they are.
pub(crate) fn is_escapable(character: char) -> bool {
    matches!(
        character,
        '!' | '"'
            | '#'
            | '$'
            | '%'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | '+'
            | ','
            | '-'
            | '.'
            | '/'
            | ':'
            | ';'
            | '<'
            | '='
            | '>'
            | '?'
            | '@'
            | '['
            | '\\'
            | ']'
            | '^'
            | '_'
            | '`'
            | '{'
            | '|'
            | '}'
            | '~'
    )
}

/// A `&name;`, `&#123;` or `&#x7B;` entity starting at `index` → what it stands
/// for and where the text after it starts.
///
/// The named table is short on purpose: these are the ones an answer uses, and
/// anything else is left as written rather than guessed at.
pub(crate) fn entity_at(characters: &[char], index: usize) -> Option<(String, usize)> {
    let mut cursor = index + 1;
    if characters.get(cursor) == Some(&'#') {
        cursor += 1;
        let hex = matches!(characters.get(cursor), Some('x' | 'X'));
        if hex {
            cursor += 1;
        }
        let start = cursor;
        while characters.get(cursor).is_some_and(|character| {
            if hex {
                character.is_ascii_hexdigit()
            } else {
                character.is_ascii_digit()
            }
        }) {
            cursor += 1;
        }
        if cursor == start || characters.get(cursor) != Some(&';') {
            return None;
        }
        let digits: String = characters[start..cursor].iter().collect();
        let code = u32::from_str_radix(&digits, if hex { 16 } else { 10 }).ok()?;
        return Some((char::from_u32(code)?.to_string(), cursor + 1));
    }
    let start = cursor;
    while characters
        .get(cursor)
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        cursor += 1;
    }
    if cursor == start || characters.get(cursor) != Some(&';') {
        return None;
    }
    let name: String = characters[start..cursor].iter().collect();
    let text = match name.to_ascii_lowercase().as_str() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "middot" => "·",
        "times" => "×",
        "divide" => "÷",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "laquo" => "«",
        "raquo" => "»",
        _ => return None,
    };
    Some((text.to_string(), cursor + 1))
}

/// Strip the inline markers from one table cell, keeping its runs.
/// A table cell read without any definitions, for the tests.
#[cfg(test)]
pub(crate) fn styled_cell(raw: &str) -> Cell {
    styled_cell_with(raw, &Definitions::default())
}

/// A table cell read with the answer's definitions to hand.
///
/// A cell is one line, so a `<br>` inside it is a space: a newline would be a
/// second line the grid has no room for.
pub(crate) fn styled_cell_with(raw: &str, refs: &Definitions) -> Cell {
    let mut cell = styled(raw.trim(), refs);
    if cell.text.contains('\n') {
        cell.text = cell.text.replace('\n', " ");
    }
    cell
}

/// Draw a link's (or a picture's) label into the cell.
///
/// The label is read as inline Markdown of its own, keeps the emphasis it sits
/// in, and stays a span of its own so [`Cell::markdown`] can put the brackets
/// back exactly around it. `url` is `None` for a picture whose address the app
/// will not open: its alt text is still drawn, just not as a link.
fn push_label(
    cell: &mut Cell,
    label: &str,
    url: Option<String>,
    bold: bool,
    italic: bool,
    strike: bool,
    marked: bool,
    refs: &Definitions,
) {
    let mut label = inline_cell(&label.chars().collect::<Vec<char>>(), false, refs);
    for span in &mut label.spans {
        span.bold |= bold;
        span.italic |= italic;
        span.strike |= strike;
        span.marked |= marked;
    }
    if label.text.is_empty() {
        return;
    }
    let start = cell.text.len();
    cell.text.push_str(&label.text);
    cell.spans.extend(label.spans);
    if let Some(url) = url {
        let end = start + label.text.len();
        cell.links.push(Link { start, end, url });
    }
}

/// A footnote marker: `[1]` is drawn as a superscript number (`¹`), not as
/// brackets in the middle of the sentence.
pub(crate) fn superscript(number: usize) -> String {
    number
        .to_string()
        .chars()
        .map(|digit| match digit {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            other => other,
        })
        .collect()
}

/// Append `run` to the cell, merging it into the previous span when the style
/// matches.
///
/// Text that follows a link is never merged into the label's run: the label has
/// to stay its own span so [`Cell::markdown`] can put the brackets back around
/// exactly it.
pub(crate) fn flush_run(
    cell: &mut Cell,
    run: &mut String,
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    underline: bool,
    mono: bool,
    marked: bool,
) {
    if run.is_empty() {
        return;
    }
    // `<code>` and `<kbd>` ask for the code font without opening a code span.
    let code = code || mono;
    let len = run.len();
    // A link that ends or starts here keeps its label a run of its own:
    // `Cell::markdown` puts the brackets on span boundaries.
    let after_link = cell
        .links
        .iter()
        .any(|link| link.end == cell.text.len() || link.start == cell.text.len());
    cell.text.push_str(run);
    run.clear();
    match cell.spans.last_mut() {
        Some(last)
            if !after_link
                && last.bold == bold
                && last.italic == italic
                && last.code == code
                && last.strike == strike
                && last.underline == underline
                && last.marked == marked =>
        {
            last.len += len;
        }
        _ => cell.spans.push(Span {
            len,
            bold,
            italic,
            code,
            strike,
            underline,
            marked,
            token: None,
        }),
    }
}

/// Split a row on `|`, keeping an escaped `\|` as a literal pipe inside its cell.
pub(crate) fn split_cells(inner: &str) -> Vec<String> {
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut characters = inner.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some('|') => cell.push('|'),
                Some(escaped) => {
                    cell.push('\\');
                    cell.push(escaped);
                }
                None => cell.push('\\'),
            },
            '|' => cells.push(std::mem::take(&mut cell)),
            _ => cell.push(character),
        }
    }
    cells.push(cell);
    cells
}

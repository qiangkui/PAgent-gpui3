//! Turning a reply into blocks: the block loop, fences, lists, quotes, tables, details, footnotes and display formulas.

use super::*;

/// Turn a reply into blocks, without the emphasis on their text.
///
/// Only the tests need this: the view draws `parse_rich`, which has the runs.
#[cfg(test)]
pub fn parse(text: &str) -> Vec<Block> {
    parse_rich(text)
        .into_iter()
        .map(|rich| rich.block)
        .collect()
}

/// Turn a reply into blocks, keeping the emphasis on each block's text.
pub fn parse_rich(text: &str) -> Vec<Rich> {
    parse_rich_with(text, &Definitions::default())
}

/// The same, with the definitions of an enclosing block to hand.
///
/// A quote is parsed on its own lines, but a `[x]: url` written outside it is
/// still a definition for the links inside: the outer tables are merged in, and
/// the quote's own definitions win over them.
pub(crate) fn parse_rich_with(text: &str, outer: &Definitions) -> Vec<Rich> {
    // Front matter is metadata for whatever writes a file, not part of the
    // answer: dropping it here keeps its `key: value` lines out of the text.
    let text = strip_front_matter(text);
    // Inline notes are turned into references first, so the numbering below can
    // count them together with `[^label]` definitions in the order they are
    // referred to.
    let (rewritten, inline) = rewrite_inline_notes(text);
    let text: &str = &rewritten;
    // Both kinds of definition are collected before the first line is read: a
    // link can point at a definition below it, and the notes are numbered in the
    // order they are defined.
    let (notes, taken) = footnote_definitions(text, inline);
    let mut refs = Definitions {
        links: reference_definitions(text),
        ..Definitions::default()
    };
    for (label, url) in &outer.links {
        refs.links
            .entry(label.clone())
            .or_insert_with(|| url.clone());
    }
    for (number, (label, _)) in notes.iter().enumerate() {
        refs.notes.insert(normalise_label(label), number + 1);
    }
    for (label, number) in &outer.notes {
        refs.notes.entry(label.clone()).or_insert(*number);
    }
    let mut blocks: Vec<Rich> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    // The opening fence's marker, its language, how far it is indented and the
    // lines inside it.
    let mut fence: Option<(char, String, usize, Vec<String>)> = None;
    // Lines of a four-space indented code block.
    let mut indented: Vec<String> = Vec::new();
    let mut table: Option<Table> = None;
    // The list item whose text is still being collected.
    let mut item: Option<(Item, Vec<String>)> = None;
    // True when a blank line follows an item: the next line decides whether it
    // is a second paragraph of that item or something else.
    let mut item_gap = false;
    // The item whose indented area the parser is still inside, and the blank
    // line that may introduce another block of that item.
    let mut open_item: Option<(usize, Marker)> = None;
    let mut after_item_gap: Option<(usize, Marker)> = None;
    // A block written under a list item, waiting to be wrapped once it is
    // complete: the item's level and marker, and the index in `blocks` where it
    // lands.
    let mut nest: Option<(usize, Marker, usize)> = None;
    // Lines already read as part of a multi-line HTML block.
    let mut skip_to = 0usize;
    // The definition being collected: its term (empty for a second description)
    // and the description's lines.
    let mut definition: Option<(String, Vec<String>)> = None;
    // Lines of a `<details>` element and of a display formula being collected,
    // with the marker that closes the formula.
    let mut details: Option<Vec<String>> = None;
    let mut formula: Option<(&'static str, Vec<String>)> = None;
    // True while inside a `<!-- … -->` comment.
    let mut in_comment = false;

    for (index, raw) in text.lines().enumerate() {
        // A block written under an item is wrapped once it is complete.
        wrap_nested(&mut blocks, &mut nest);
        // A multi-line HTML block already read every line it needed.
        if index < skip_to {
            continue;
        }
        // The paragraph keeps the line as it was written: its trailing spaces are
        // what makes a hard line break. Everything else reads the trimmed line.
        let line = raw.trim_end();
        let trimmed = line.trim_start();

        // Leaving the item's indented area: a line that starts at the margin and
        // has text closes it.
        if open_item.is_some() && !trimmed.is_empty() && fence_indent(raw) == 0 {
            open_item = None;
            after_item_gap = None;
        }
        // A blank line after an item's blocks may introduce another block of the
        // same item.
        if trimmed.is_empty() {
            if open_item.is_some() {
                after_item_gap = open_item;
            }
        } else if let Some((level, marker)) = after_item_gap.take() {
            let columns = fence_indent(raw);
            let next = text.lines().nth(index + 1);
            if columns == 0 {
                open_item = None;
            } else if columns >= 4 || starts_block(trimmed, next) {
                open_item = Some((level, marker));
                nest = Some((level, marker, blocks.len()));
            } else {
                open_item = None;
            }
        }

        // A footnote definition is drawn at the end of the answer, not here.
        // Lines inside a fence are code, and never counted as definitions.
        if taken.get(index).copied().unwrap_or(false) && fence.is_none() {
            flush(&mut paragraph, &mut blocks, &refs);
            continue;
        }

        // A list item's text may run on to the next line: answers wrap a long
        // item and indent the rest of the sentence. Only an indented line
        // continues it, so a paragraph written right under a list (with the
        // blank line forgotten) stays a paragraph of its own.
        if item.is_some() && !item_gap && continues_item(raw) {
            if let Some((_, lines)) = item.as_mut() {
                lines.push(line.trim_start().to_string());
            }
            continue;
        }
        // A blank line after an item may be the gap before a second paragraph of
        // that item: the next line decides. A list item is one block of text, so
        // such a paragraph joins it with a blank line between.
        if item.is_some() && raw.trim().is_empty() {
            item_gap = true;
            continue;
        }
        let mut nested: Option<(usize, Marker)> = None;
        if item_gap {
            item_gap = false;
            let trimmed = raw.trim_start();
            let columns = fence_indent(raw);
            // A term followed by `: 说明` is a definition list, and a table's
            // rule or a setext underline turns the line above it into a block of
            // its own: none of them is a paragraph of the item, even when they
            // are indented under the list.
            let next = text.lines().nth(index + 1);
            let next_is_definition =
                next.is_some_and(|next| definition_item(next.trim_start()).is_some());
            let next_opens_block = next.is_some_and(|next| {
                let next = next.trim_start();
                is_table_rule(next) || setext_underline(next).is_some()
            });
            if item.is_some()
                && columns > 0
                && columns < 4
                && !looks_like_marker(trimmed)
                && !next_is_definition
                && !next_opens_block
            {
                if let Some((_, lines)) = item.as_mut() {
                    lines.push(String::new());
                    lines.push(trimmed.to_string());
                }
                continue;
            }
            // An indented fence, quote, table, picture or a four-space block
            // under the item is a block of its own: keep it under the item
            // instead of at the margin.
            if let Some((level, marker)) = item.as_ref().and_then(item_level)
                && columns > 0
                && !is_list_marker(trimmed)
                && (columns >= 4 || starts_block(trimmed, next))
            {
                nested = Some((level, marker));
            }
        }
        // The same block may follow the item's text directly, without a blank
        // line: `- 项` then an indented fence is still that item's fence.
        if nested.is_none()
            && let Some((level, marker)) = item.as_ref().and_then(item_level)
        {
            let next = text.lines().nth(index + 1);
            let columns = fence_indent(raw);
            if columns > 0
                && !is_list_marker(line.trim_start())
                && (columns >= 4 || starts_block(line.trim_start(), next))
            {
                nested = Some((level, marker));
            }
        }
        flush_item(&mut item, &mut blocks, &refs);
        if let Some((level, marker)) = nested {
            open_item = Some((level, marker));
            nest = Some((level, marker, blocks.len()));
        }

        // A definition's description runs on to an indented line, the way a
        // list item's text does.
        if definition.is_some() && continues_item(raw) {
            if let Some((_, lines)) = definition.as_mut() {
                lines.push(line.trim_start().to_string());
            }
            continue;
        }
        // Another `: 说明` line belongs to the same entry and is read where the
        // first one is, so it must not close the definition in progress.
        if !(definition.is_some() && definition_item(line.trim_start()).is_some()) {
            flush_definition(&mut definition, &mut blocks, &refs);
        }

        // A `<details>` element is collected whole: its summary is the clickable
        // line and its body is Markdown of its own, drawn when it is opened. A
        // fenced or four-space code block owns its lines, so what looks like a
        // tag inside one stays code.
        let details_start =
            fence.is_none() && !is_indented(raw) && starts_with_tag(line, "<details");
        if details.is_some() || details_start {
            if details.is_none() {
                flush(&mut paragraph, &mut blocks, &refs);
                flush_quote(&mut quote, &mut blocks, &refs);
                close_table(&mut table, &mut blocks);
                flush_indented(&mut indented, &mut blocks);
                details = Some(Vec::new());
            }
            if let Some(lines) = details.as_mut() {
                lines.push(raw.to_string());
            }
            // Nested `<details>` elements close one at a time: the outer one ends
            // only when every open tag has its own closing tag.
            let closed = details
                .as_ref()
                .is_some_and(|lines| open_details(lines) == 0);
            if closed {
                finish_details(&mut details, &mut blocks, &refs);
            }
            continue;
        }

        // `$$…$$` or `\[…\]` is a formula on a line of its own, however many
        // lines it spans.
        let display = line.trim_start();
        let opens_formula = fence.is_none()
            && !is_indented(raw)
            && (display.starts_with("$$") || display.starts_with("\\["));
        if formula.is_some() || opens_formula {
            if formula.is_none() {
                flush(&mut paragraph, &mut blocks, &refs);
                flush_quote(&mut quote, &mut blocks, &refs);
                close_table(&mut table, &mut blocks);
                flush_indented(&mut indented, &mut blocks);
                let close = if display.starts_with("$$") {
                    "$$"
                } else {
                    "\\]"
                };
                formula = Some((close, Vec::new()));
            }
            if let Some((_, lines)) = formula.as_mut() {
                lines.push(line.trim().to_string());
            }
            let (close, collected) = formula
                .as_ref()
                .map(|(close, lines)| (*close, lines.join("\n")))
                .unwrap_or(("$$", String::new()));
            // Both markers are in: a one-line formula arrives that way too.
            let complete = if close == "$$" {
                collected.matches("$$").count() >= 2
            } else {
                collected.contains("\\]")
            };
            if complete {
                formula = None;
                let source = formula_source(&collected, close);
                let text = latex(&source);
                blocks.push(Rich {
                    block: Block::Math(Formula::new(&source)),
                    spans: math_run(&text),
                    links: Vec::new(),
                });
            }
            continue;
        }

        // Block-level HTML the answer draws as a block of its own: a `<pre>` is
        // a code block, a `<table>` a table and a `<blockquote>` a quote. The
        // element is collected whole, closing tag and all. A code block owns its
        // lines, so a tag-looking line inside one is code.
        let html_block = if fence.is_none() && !is_indented(raw) {
            html_block_start(trimmed).filter(|block| html_block_complete(text, index, *block))
        } else {
            None
        };
        if let Some(block) = html_block {
            let close = html_block_close(block);
            let mut end = index;
            let mut collected: Vec<&str> = Vec::new();
            for (position, raw_line) in text.lines().enumerate().skip(index) {
                collected.push(raw_line);
                end = position;
                if raw_line.to_ascii_lowercase().contains(close) {
                    break;
                }
            }
            let collected = collected.join("\n");
            flush(&mut paragraph, &mut blocks, &refs);
            flush_quote(&mut quote, &mut blocks, &refs);
            flush_item(&mut item, &mut blocks, &refs);
            close_table(&mut table, &mut blocks);
            flush_indented(&mut indented, &mut blocks);
            match block {
                HtmlBlock::Pre => {
                    let (language, body) = html_pre(&collected);
                    blocks.push(Rich {
                        block: Block::Code(language.clone(), body.clone()),
                        spans: highlight(&language, &body),
                        links: Vec::new(),
                    });
                }
                HtmlBlock::Table => {
                    if let Some(table) = html_table(&collected, &refs) {
                        blocks.push(Rich {
                            block: Block::Table(table),
                            spans: Vec::new(),
                            links: Vec::new(),
                        });
                    }
                }
                HtmlBlock::Quote => {
                    let inner = html_inner(&collected, close);
                    let inner = parse_rich_with(inner, &refs);
                    blocks.push(Rich {
                        block: Block::Quote(inner),
                        spans: Vec::new(),
                        links: Vec::new(),
                    });
                }
                HtmlBlock::List(ordered) => {
                    let at = blocks.len();
                    blocks.extend(html_list(&collected, ordered, &refs));
                    // The element made several blocks: all of them belong under
                    // the item when one is waiting for them.
                    if let Some((level, marker, from)) = nest.take() {
                        if from == at {
                            for slot in &mut blocks[at..] {
                                indent_block(level, marker, slot);
                            }
                        } else {
                            nest = Some((level, marker, from));
                        }
                    }
                }
                HtmlBlock::Heading(level) => {
                    if let Some(rich) = html_text_block(&collected, close, Some(level), &refs) {
                        blocks.push(rich);
                    }
                }
                HtmlBlock::Paragraph => {
                    if let Some(rich) = html_text_block(&collected, close, None, &refs) {
                        blocks.push(rich);
                    }
                }
                HtmlBlock::Div => {
                    // A `<div>` has no look of its own: what it holds is drawn
                    // as the blocks it is.
                    let inner = html_inner(&collected, close);
                    let at = blocks.len();
                    blocks.extend(parse_rich_with(inner, &refs));
                    if let Some((level, marker, from)) = nest.take() {
                        if from == at {
                            for slot in &mut blocks[at..] {
                                indent_block(level, marker, slot);
                            }
                        } else {
                            nest = Some((level, marker, from));
                        }
                    }
                }
            }
            skip_to = end + 1;
            continue;
        }

        // An indented code block runs while the indentation lasts; any other
        // line closes it before being read as whatever it is.
        if !(is_indented(raw) && !indented.is_empty()) {
            flush_indented(&mut indented, &mut blocks);
        }
        // Inside such a block every indented line is code, even one that looks
        // like a marker: the block was opened by the line above.
        if is_indented(raw) && !indented.is_empty() {
            indented.push(indented_text(raw).to_string());
            continue;
        }

        if let Some((marker, language, indent, body)) = fence.as_mut() {
            // Only the marker that opened the block closes it, so a `~~~` block
            // may contain a line of backticks.
            if fence_marker(line) == Some(*marker) {
                let code = body.join("\n");
                push_fence_block(&mut blocks, language, &code);
                fence = None;
            } else {
                // Code keeps its own indentation and markers; the indentation of
                // the fence itself is layout, so a block written under a list
                // item does not draw shifted to the right.
                body.push(without_indent(line, *indent).to_string());
            }
            continue;
        }

        if let Some(marker) = fence_marker(line) {
            close_table(&mut table, &mut blocks);
            flush(&mut paragraph, &mut blocks, &refs);
            flush_quote(&mut quote, &mut blocks, &refs);
            fence = Some((marker, language_of(line), fence_indent(line), Vec::new()));
            continue;
        }

        // Every `>` line belongs to the same quote, the way consecutive lines
        // belong to one paragraph: one block per line would draw a stack of
        // separate boxes with a gap between them.
        if let Some(rest) = quote_line(raw) {
            close_table(&mut table, &mut blocks);
            flush(&mut paragraph, &mut blocks, &refs);
            quote.push(rest.to_string());
            continue;
        }
        flush_quote(&mut quote, &mut blocks, &refs);

        // Only `|` rows continue a table. Anything else ends it and is then read
        // as ordinary text below.
        if table.is_some() {
            match cells_of(line, &refs) {
                // A lone pipe (or a row of nothing) is what a half-streamed row
                // looks like: keep the table open, but do not add a blank row.
                Some(cells) if cells.iter().any(|cell| !cell.text.is_empty()) => {
                    if let Some(table) = table.as_mut() {
                        table.rows.push(cells);
                    }
                    continue;
                }
                Some(_) => continue,
                None => close_table(&mut table, &mut blocks),
            }
        }

        // `[b]: https://…` was collected up front and draws nothing itself.
        if reference_definition(line).is_some() {
            flush(&mut paragraph, &mut blocks, &refs);
            continue;
        }

        if line.trim().is_empty() {
            flush(&mut paragraph, &mut blocks, &refs);
            continue;
        }

        // Four spaces (or a tab) with nothing above it in the same block start a
        // code block. A line that still reads as a marker is left to the rules
        // below, so an indented list item stays a list item.
        if is_indented(raw) && paragraph.is_empty() && quote.is_empty() && table.is_none() {
            let text = indented_text(raw);
            if !looks_like_marker(text) {
                indented.push(text.to_string());
                continue;
            }
        }

        // `<!-- … -->` is a note to whoever reads the source: it draws nothing,
        // even when it spans several lines.
        if in_comment || trimmed.starts_with("<!--") {
            if !in_comment {
                flush(&mut paragraph, &mut blocks, &refs);
                in_comment = true;
            }
            if let Some(at) = line.find("-->") {
                in_comment = false;
                let rest = line[at + 3..].trim_start();
                if !rest.is_empty() {
                    paragraph.push(rest.to_string());
                }
            }
            continue;
        }
        // `<hr>` is the HTML spelling of a section rule.
        if is_html_rule(trimmed) {
            close_table(&mut table, &mut blocks);
            flush(&mut paragraph, &mut blocks, &refs);
            blocks.push(Rich {
                block: Block::Rule,
                spans: Vec::new(),
                links: Vec::new(),
            });
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            let cell = styled(rest, &refs);
            blocks.push(Rich {
                block: Block::Heading(level, cell.text),
                spans: cell.spans,
                links: cell.links,
            });
            continue;
        }
        // A line of `=` or `-` right under a text line underlines it into a
        // heading (setext). Only the last line of the paragraph becomes the
        // heading; anything above it stays a paragraph, as Markdown says.
        if let Some(level) = setext_underline(trimmed)
            && let Some(text) = paragraph.pop()
        {
            flush(&mut paragraph, &mut blocks, &refs);
            // The underlined line may be indented (it can sit under a list
            // item); the heading's text is what it says, not its layout.
            let cell = styled(text.trim(), &refs);
            blocks.push(Rich {
                block: Block::Heading(level, cell.text),
                spans: cell.spans,
                links: cell.links,
            });
            continue;
        }
        // `---` on its own line separates sections. It is checked before the
        // table rule because a line of dashes has no pipes, and the paragraph
        // above it ends here rather than swallowing the dashes as text.
        if is_rule(trimmed) {
            close_table(&mut table, &mut blocks);
            flush(&mut paragraph, &mut blocks, &refs);
            blocks.push(Rich {
                block: Block::Rule,
                spans: Vec::new(),
                links: Vec::new(),
            });
            continue;
        }
        // A table's rule (`|---|---|`) belongs to the line above it: that line is
        // the header, and the `|` lines after the rule are its body.
        if is_table_rule(trimmed) {
            let header = paragraph.pop();
            match header.as_deref().and_then(|line| cells_of(line, &refs)) {
                Some(cells) => {
                    flush(&mut paragraph, &mut blocks, &refs);
                    table = Some(Table {
                        header: cells,
                        rows: Vec::new(),
                        aligns: aligns_of(trimmed),
                    });
                }
                None => {
                    // A rule with no header above it is punctuation; keep the line
                    // that came before it, if any.
                    if let Some(header) = header {
                        paragraph.push(header);
                    }
                }
            }
            continue;
        }
        // A task list is a list whose marker is a box: `- [ ] todo`,
        // `- [x] done`. It is checked before the plain bullet, which would
        // otherwise read the box as part of the item's text.
        if let Some((checked, rest)) = task_item(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            item = Some((
                Item::Task(nested_level(&blocks, list_level(line)), checked),
                vec![rest.to_string()],
            ));
            continue;
        }
        if let Some(rest) = bullet(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            item = Some((
                Item::Bullet(nested_level(&blocks, list_level(line))),
                vec![rest.to_string()],
            ));
            continue;
        }
        if let Some((marker, rest)) = numbered(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            item = Some((
                Item::Numbered(nested_level(&blocks, list_level(line)), marker),
                vec![rest.to_string()],
            ));
            continue;
        }
        // `术语` + `: 说明` is a definition list. The term is the line above, so
        // the line has to be there: a `: 说明` with nothing above it is text.
        // A second `: 说明` under the same term is another description.
        if let Some(rest) = definition_item(trimmed) {
            // The terms are the paragraph above — every line of it — without the
            // indentation that made them text rather than nested markers.
            // Several terms may share one description.
            let terms: Vec<String> = paragraph
                .drain(..)
                .map(|term| term.trim().to_string())
                .collect();
            if !terms.is_empty()
                || definition.is_some()
                || matches!(
                    blocks.last().map(|rich| &rich.block),
                    Some(Block::Definition(_))
                )
            {
                flush_definition(&mut definition, &mut blocks, &refs);
                definition = Some((terms.join("\n"), vec![rest.to_string()]));
                continue;
            }
        }
        // `<img src="…">` alone on its line is the picture, like `![alt](src)`.
        if let Some((alt, url)) = html_image(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            let mut cell = styled(&alt, &refs);
            if cell.links.is_empty() && is_openable_link(&url) && !cell.text.is_empty() {
                cell.links.push(Link {
                    start: 0,
                    end: cell.text.len(),
                    url: url.clone(),
                });
            }
            blocks.push(Rich {
                block: Block::Image(cell.text, url),
                spans: cell.spans,
                links: cell.links,
            });
            continue;
        }
        // `![说明](地址)` alone on its line is the picture itself, not a link in a
        // sentence, so it draws as a placeholder card.
        if let Some((alt, url, title)) = image_line(trimmed) {
            flush(&mut paragraph, &mut blocks, &refs);
            let caption = if alt.trim().is_empty() { title } else { alt };
            let mut cell = styled(&caption, &refs);
            if cell.links.is_empty() && is_openable_link(&url) && !cell.text.is_empty() {
                cell.links.push(Link {
                    start: 0,
                    end: cell.text.len(),
                    url: url.clone(),
                });
            }
            blocks.push(Rich {
                block: Block::Image(cell.text, url),
                spans: cell.spans,
                links: cell.links,
            });
            continue;
        }
        paragraph.push(raw.to_string());
    }

    flush_item(&mut item, &mut blocks, &refs);
    flush_definition(&mut definition, &mut blocks, &refs);
    flush_indented(&mut indented, &mut blocks);
    close_table(&mut table, &mut blocks);
    if let Some((_, language, _, body)) = fence {
        let code = body.join("\n");
        push_fence_block(&mut blocks, &language, &code);
    }
    flush(&mut paragraph, &mut blocks, &refs);
    flush_quote(&mut quote, &mut blocks, &refs);
    // A stream that stops mid-element still has to draw: the collected lines are
    // all there is.
    finish_details(&mut details, &mut blocks, &refs);
    if let Some((close, lines)) = formula.take() {
        let source = formula_source(&lines.join("\n"), close);
        let text = latex(&source);
        blocks.push(Rich {
            block: Block::Math(Formula::new(&source)),
            spans: math_run(&text),
            links: Vec::new(),
        });
    }
    // The notes the answer defines come last, in the order they were written.
    // A note is read as Markdown of its own, so a list or a fence inside one
    // keeps its shape.
    for (index, (_, text)) in notes.iter().enumerate() {
        let inner = parse_rich_with(text, &refs);
        blocks.push(Rich {
            block: Block::Footnote(index + 1, inner),
            spans: Vec::new(),
            links: Vec::new(),
        });
    }
    // A block written under the last item may still be waiting for its end.
    wrap_nested(&mut blocks, &mut nest);
    blocks
}

/// The level of the list item whose text is being collected, and the column its
/// text starts after.
pub(crate) fn item_level(item: &(Item, Vec<String>)) -> Option<(usize, Marker)> {
    match &item.0 {
        Item::Bullet(level) | Item::Task(level, _) => Some((*level, Marker::Bullet)),
        Item::Numbered(level, _) => Some((*level, Marker::Number)),
    }
}

/// Whether `line` opens a list item (bullet, task or number).
pub(crate) fn is_list_marker(line: &str) -> bool {
    bullet(line).is_some() || numbered(line).is_some()
}

/// Whether an indented line opens a block of its own rather than a paragraph.
///
/// The last two need the next line: a table starts at its rule, and a setext
/// heading at its underline.
pub(crate) fn starts_block(line: &str, next: Option<&str>) -> bool {
    quote_line(line).is_some()
        || fence_marker(line).is_some()
        || heading(line).is_some()
        || starts_with_tag(line, "<details")
        || starts_with_tag(line, "<img")
        || starts_with_tag(line, "<pre")
        || starts_with_tag(line, "<table")
        || starts_with_tag(line, "<blockquote")
        || starts_with_tag(line, "<ul")
        || starts_with_tag(line, "<ol")
        || starts_with_tag(line, "<h1")
        || starts_with_tag(line, "<h2")
        || starts_with_tag(line, "<h3")
        || starts_with_tag(line, "<h4")
        || starts_with_tag(line, "<h5")
        || starts_with_tag(line, "<h6")
        || starts_with_tag(line, "<p>")
        || starts_with_tag(line, "<p ")
        || starts_with_tag(line, "<div")
        || is_html_rule(line)
        || image_line(line).is_some()
        || line.starts_with("$$")
        || line.starts_with("\\[")
        || next.is_some_and(|next| {
            let next = next.trim();
            is_table_rule(next) || setext_underline(next).is_some()
        })
}

/// Whether the HTML element opening on line `index` is a block this app draws.
///
/// A heading, a paragraph, a list and a `<div>` only count when their closing tag
/// is really there (and a list also needs an item): a sentence that mentions
/// `<h1>` or `<ul>` is text, not a heading or an empty list. A table, a quote and
/// a `<pre>` are drawn as they stream in, so they need no closing tag.
pub(crate) fn html_block_complete(text: &str, index: usize, block: HtmlBlock) -> bool {
    let close = html_block_close(block);
    let mut closed = false;
    let mut has_item = false;
    for line in text.lines().skip(index) {
        let lower = line.to_ascii_lowercase();
        has_item |= lower.contains("<li");
        if lower.contains(close) {
            closed = true;
            break;
        }
    }
    match block {
        HtmlBlock::Heading(_) | HtmlBlock::Paragraph | HtmlBlock::List(_) | HtmlBlock::Div => {
            closed && (!matches!(block, HtmlBlock::List(_)) || has_item)
        }
        HtmlBlock::Pre | HtmlBlock::Table | HtmlBlock::Quote => true,
    }
}

/// Whether a fence's language says the body is a formula.
pub(crate) fn is_math_language(language: &str) -> bool {
    matches!(
        language.trim().to_ascii_lowercase().as_str(),
        "math" | "latex" | "tex" | "katex"
    )
}

/// Draw a finished fence: a formula when its language says so, code otherwise.
///
/// Models put display maths in ```math fences as often as in `$$…$$`; the body
/// is then laid out like any other display formula instead of shown as source.
pub(crate) fn push_fence_block(blocks: &mut Vec<Rich>, language: &str, body: &str) {
    if is_math_language(language) && !body.trim().is_empty() {
        let source = body.trim().to_string();
        let text = latex(&source);
        blocks.push(Rich {
            block: Block::Math(Formula::new(&source)),
            spans: math_run(&text),
            links: Vec::new(),
        });
        return;
    }
    blocks.push(Rich {
        block: Block::Code(language.to_string(), body.to_string()),
        spans: highlight(language, body),
        links: Vec::new(),
    });
}

/// Wrap the block that was written under a list item once it is complete.
///
/// The block is only known to be finished when its own rule says so (a fence's
/// closing marker, a quote's last line…), so it is pushed as usual and wrapped
/// here on the next pass. Only the first block after the item is taken: the
/// block's own lines never push another one.
pub(crate) fn wrap_nested(blocks: &mut [Rich], nest: &mut Option<(usize, Marker, usize)>) {
    let Some((level, marker, from)) = *nest else {
        return;
    };
    if blocks.len() <= from {
        return;
    }
    *nest = None;
    if let Some(slot) = blocks.get_mut(from) {
        indent_block(level, marker, slot);
    }
}

/// Put one block under a list item's text column.
pub(crate) fn indent_block(level: usize, marker: Marker, rich: &mut Rich) {
    let inner = std::mem::replace(
        rich,
        Rich {
            block: Block::Paragraph(String::new()),
            spans: Vec::new(),
            links: Vec::new(),
        },
    );
    *rich = Rich {
        block: Block::Indented(level, marker, Box::new(inner)),
        spans: Vec::new(),
        links: Vec::new(),
    };
}

/// A `$…$` formula starting at `index` → the text to draw and where the text
/// after it starts.
///
/// Markdown's inline math cannot start or end with a space, and only a `$` closes
/// it: that is what keeps "$5 和 $10" reading as prices.
pub(crate) fn inline_formula(characters: &[char], index: usize) -> Option<(String, usize)> {
    if characters.get(index) != Some(&'$') {
        return None;
    }
    // `$$…$$` written inside a sentence is one formula too: a display marker on
    // a line of its own is read as a block before this runs, so only the inline
    // spelling reaches here.
    if characters.get(index + 1) == Some(&'$') {
        let mut end = index + 2;
        while let Some(character) = characters.get(end).copied() {
            match character {
                '$' if characters.get(end + 1) == Some(&'$') => {
                    let source: String = characters[index + 2..end].iter().collect();
                    if source.trim().is_empty() {
                        return None;
                    }
                    return Some((latex(&source), end + 2));
                }
                '\\' => end += 2,
                '`' | '\n' => return None,
                _ => end += 1,
            }
        }
        return None;
    }
    match characters.get(index + 1) {
        Some(next) if !next.is_whitespace() => {}
        _ => return None,
    }
    let mut end = index + 1;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '$' => {
                let source: String = characters[index + 1..end].iter().collect();
                if source.is_empty() || source.ends_with(' ') {
                    return None;
                }
                return Some((latex(&source), end + 1));
            }
            '\\' => end += 2,
            // A formula does not run past a code span or the end of a line.
            '`' | '\n' => return None,
            _ => end += 1,
        }
    }
    None
}

/// A `\(…\)` formula starting at `index` (the backslash) → the text to draw and
/// where the text after it starts.
///
/// Models write inline maths both ways; `\(…\)` is the form LaTeX itself uses to
/// stay clear of the `$`s that prices are written with.
pub(crate) fn inline_paren_formula(characters: &[char], index: usize) -> Option<(String, usize)> {
    if characters.get(index) != Some(&'\\') || characters.get(index + 1) != Some(&'(') {
        return None;
    }
    let mut end = index + 2;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '\\' if characters.get(end + 1) == Some(&')') => {
                let source: String = characters[index + 2..end].iter().collect();
                if source.trim().is_empty() {
                    return None;
                }
                return Some((latex(&source), end + 2));
            }
            // A formula does not run past a code span or the end of a line.
            '`' | '\n' => return None,
            _ => end += 1,
        }
    }
    None
}

/// The end of a `<!-- … -->` comment starting at `index`, when it closes on the
/// same line.
pub(crate) fn html_comment_end(characters: &[char], index: usize) -> Option<usize> {
    if characters.get(index) != Some(&'<')
        || characters.get(index + 1) != Some(&'!')
        || characters.get(index + 2) != Some(&'-')
        || characters.get(index + 3) != Some(&'-')
    {
        return None;
    }
    let mut end = index + 4;
    while end + 2 <= characters.len() {
        match characters.get(end).copied() {
            Some('-')
                if characters.get(end + 1) == Some(&'-')
                    && characters.get(end + 2) == Some(&'>') =>
            {
                return Some(end + 3);
            }
            Some('\n') | None => return None,
            _ => end += 1,
        }
    }
    None
}

/// `<img …>` starting at `index` → its caption, address and the text after it.
pub(crate) fn inline_html_image(
    characters: &[char],
    index: usize,
) -> Option<(String, String, usize)> {
    if characters.get(index) != Some(&'<') {
        return None;
    }
    let head: String = characters[index..characters.len().min(index + 4)]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    if !head.starts_with("<img") {
        return None;
    }
    let mut end = index;
    loop {
        match characters.get(end).copied() {
            Some('>') => break,
            Some('\n') | None => return None,
            _ => {
                if end - index > 300 {
                    return None;
                }
                end += 1;
            }
        }
    }
    let attributes: String = characters[index..end].iter().collect();
    let src = attribute_value(&attributes, "src")?;
    let alt = attribute_value(&attributes, "alt").unwrap_or_default();
    let title = attribute_value(&attributes, "title").unwrap_or_default();
    let caption = if alt.trim().is_empty() { title } else { alt };
    Some((caption, src, end + 1))
}

/// Whether `line` starts with `tag`, ignoring case and leading spaces.
pub(crate) fn starts_with_tag(line: &str, tag: &str) -> bool {
    line.trim_start().to_ascii_lowercase().starts_with(tag)
}

/// How many `<details>` elements the collected lines leave open.
///
/// A nested element has to close before the one around it, so the count is what
/// decides where the outer one ends; a line holding both tags (a whole element
/// in one line) brings the count back down.
pub(crate) fn open_details(lines: &[String]) -> usize {
    let mut depth = 0usize;
    for line in lines {
        let lower = line.to_ascii_lowercase();
        depth += lower.matches("<details").count();
        depth = depth.saturating_sub(lower.matches("</details>").count());
    }
    depth
}

/// The summary, whether the element says it starts open, and the body of a
/// `<details>` element.
///
/// The tags are cut out by hand: the summary may share its line with `<details>`,
/// and the body is everything else, which is then read as Markdown.
pub(crate) fn details_parts(text: &str) -> (String, bool, String) {
    let mut rest = text.to_string();
    let mut summary = String::new();
    let mut open = false;
    if let Some(at) = rest.to_ascii_lowercase().find("<details")
        && let Some(end) = rest[at..].find('>')
    {
        let tag = &rest[at..at + end + 1];
        // `<details open>` shows its contents straight away.
        open = tag
            .to_ascii_lowercase()
            .split_whitespace()
            .any(|part| part.trim_end_matches('>') == "open" || part.starts_with("open="));
    }
    if let Some(at) = rest.to_ascii_lowercase().find("<summary")
        && let Some(open_tag) = rest[at..].find('>')
    {
        let content = at + open_tag + 1;
        if let Some(close) = rest[content..].to_ascii_lowercase().find("</summary>") {
            summary = rest[content..content + close].trim().to_string();
            rest.replace_range(at..content + close + "</summary>".len(), "");
        }
    }
    if let Some(at) = rest.to_ascii_lowercase().find("<details")
        && let Some(end) = rest[at..].find('>')
    {
        rest.replace_range(at..at + end + 1, "");
    }
    // The last closing tag is this element's own: an inner element keeps its
    // closing tag, so the body parses as the nested element it is.
    if let Some(at) = rest.to_ascii_lowercase().rfind("</details>") {
        rest.truncate(at);
    }
    (summary, open, rest.trim().to_string())
}

/// Finish the `<details>` being collected, if there is one.
pub(crate) fn finish_details(
    details: &mut Option<Vec<String>>,
    blocks: &mut Vec<Rich>,
    refs: &Definitions,
) {
    let Some(lines) = details.take() else {
        return;
    };
    let (summary, open, body) = details_parts(&lines.join("\n"));
    let summary = if summary.is_empty() {
        "详情".to_string()
    } else {
        summary
    };
    let summary = styled(&summary, refs);
    let inner = parse_rich_with(&body, refs);
    blocks.push(Rich {
        block: Block::Details(summary, open, inner),
        spans: Vec::new(),
        links: Vec::new(),
    });
}

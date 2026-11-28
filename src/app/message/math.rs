//! Math layout: turning parsed math into laid-out pieces.
//!
//! Kept apart from the rest of the transcript renderer because it is a small
//! layout engine of its own — sequences, script sizes, limits around large
//! operators — and a change to it should never touch how a message looks.

use super::*;

/// A formula's pieces side by side, centred on the line.
///
/// A display formula may write its rows on separate lines with `\\`: each row is
/// laid out on its own, one under the other, instead of running together.
pub(super) fn math_sequence(
    nodes: &[markdown::MathNode],
    palette: Palette,
    tone: u32,
    size: f32,
) -> AnyElement {
    let mut rows: Vec<Vec<&markdown::MathNode>> = vec![Vec::new()];
    for node in nodes {
        if matches!(node, markdown::MathNode::Break) {
            rows.push(Vec::new());
        } else if let Some(row) = rows.last_mut() {
            row.push(node);
        }
    }
    let row = |pieces: &[&markdown::MathNode]| {
        // An operator with its limits stands taller than the rest of the line;
        // aligning baselines would hang everything from its upper limit, so a
        // row holding one is centred instead.
        let limits = pieces.iter().any(|node| big_operator(node).is_some());
        let laid_out = div().flex().flex_row().children(
            pieces
                .iter()
                .map(|node| math_piece(node, palette, tone, size)),
        );
        if limits {
            laid_out.items_center().into_any_element()
        } else {
            laid_out.items_baseline().into_any_element()
        }
    };
    if rows.len() == 1 {
        return row(&rows[0]);
    }
    div()
        .flex()
        .flex_col()
        .items_center()
        .children(rows.iter().map(|pieces| row(pieces)))
        .into_any_element()
}

/// One piece of a formula: text, a stack of two, a raised or lowered script, or a
/// root with its bar.
pub(super) fn math_piece(
    node: &markdown::MathNode,
    palette: Palette,
    tone: u32,
    size: f32,
) -> AnyElement {
    let script = size * MATH_SCRIPT;
    match node {
        markdown::MathNode::Text(text) => div()
            .font_family(MONO_FONT)
            .text_size(px(size))
            .text_color(rgb(tone))
            .child(text.clone())
            .into_any_element(),
        markdown::MathNode::Row(pieces) => {
            let limits = pieces.iter().any(|node| big_operator(node).is_some());
            let laid_out = div().flex().flex_row().children(
                pieces
                    .iter()
                    .map(|piece| math_piece(piece, palette, tone, size)),
            );
            if limits {
                laid_out.items_center().into_any_element()
            } else {
                laid_out.items_baseline().into_any_element()
            }
        }
        markdown::MathNode::Frac(first, second) => {
            // The bar needs a definite width: a percentage of an auto-width column
            // resolves to nothing, which leaves fractions with no line at all.
            let width = math_width(first, size).max(math_width(second, size)) + size * 0.4;
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(width))
                .child(math_piece(first, palette, tone, size))
                .child(div().w_full().h(px(1.)).my(px(1.)).bg(rgb(palette.muted)))
                .child(math_piece(second, palette, tone, size))
                .into_any_element()
        }
        // A script sits beside its base and is nudged up or down: `relative()`
        // moves it without asking the line to grow. A sum's limits go above and
        // below it instead, the way a display formula writes them.
        markdown::MathNode::Sup(base, power) | markdown::MathNode::Sub(base, power) => {
            if let Some((operator, lower, upper)) = big_operator(node) {
                return math_limits(operator, lower, upper, palette, tone, size);
            }
            let offset = if matches!(node, markdown::MathNode::Sup(..)) {
                -size * 0.35
            } else {
                size * 0.25
            };
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .child(math_piece(base, palette, tone, size))
                .child(
                    div()
                        .relative()
                        .top(px(offset))
                        .child(math_piece(power, palette, tone, script)),
                )
                .into_any_element()
        }
        markdown::MathNode::Sqrt(index, body) => {
            let body_width = math_width(body, size) + size * 0.2;
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .children(
                    index
                        .as_ref()
                        .map(|index| math_piece(index, palette, tone, script)),
                )
                .child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(size * 1.15))
                        .text_color(rgb(tone))
                        .child("√"),
                )
                // The bar over the radicand is a hairline above it, as wide as the
                // radicand in the mono font's advance.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(px(body_width))
                        .child(div().w_full().h(px(1.)).bg(rgb(palette.muted)))
                        .child(math_piece(body, palette, tone, size)),
                )
                .into_any_element()
        }
        // `\binom{n}{k}`: the two parts stacked inside brackets, with no bar.
        markdown::MathNode::Binom(top, bottom) => {
            let glyph = |text: &'static str| {
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(size * 1.7))
                    .text_color(rgb(tone))
                    .child(text)
            };
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(glyph("("))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .px(px(size * 0.15))
                        .child(math_piece(top, palette, tone, size))
                        .child(math_piece(bottom, palette, tone, size)),
                )
                .child(glyph(")"))
                .into_any_element()
        }
        // `\overline{x}` / `\underline{x}`: a hairline above or below the piece.
        markdown::MathNode::Overline(body, above) => {
            let rule = || div().w_full().h(px(1.)).bg(rgb(palette.muted));
            let width = math_width(body, size) + size * 0.2;
            let piece = math_piece(body, palette, tone, size);
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(width))
                .when(*above, |this| this.child(rule()))
                .child(piece)
                .when(!*above, |this| this.child(rule()))
                .into_any_element()
        }
        // A matrix: rows of cells in a grid, inside the bracket its environment
        // names. Every column is as wide as its widest cell, so `aligned` lines
        // up its `&` marks instead of each row drifting on its own.
        markdown::MathNode::Matrix(rows, bracket) => {
            let (left, right) = match bracket {
                Some(markdown::Bracket::Paren) => ("(", ")"),
                Some(markdown::Bracket::Square) => ("[", "]"),
                Some(markdown::Bracket::Bar) => ("|", "|"),
                Some(markdown::Bracket::Brace) => ("{", "}"),
                None => ("", ""),
            };
            let glyph = |text: &'static str| {
                div()
                    .font_family(MONO_FONT)
                    .text_size(px(size * 1.7))
                    .text_color(rgb(tone))
                    .child(text)
            };
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            let widths: Vec<f32> = (0..columns)
                .map(|column| {
                    rows.iter()
                        .filter_map(|row| row.get(column))
                        .map(|cell| math_nodes_width(cell, size))
                        .fold(0., f32::max)
                })
                .collect();
            div()
                .flex()
                .flex_row()
                .items_center()
                .when(!left.is_empty(), |this| this.child(glyph(left)))
                .child(div().flex().flex_col().gap_1().px(px(size * 0.3)).children(
                    rows.iter().map(|row| {
                        div().flex().flex_row().items_center().gap_2().children(
                            row.iter().enumerate().map(|(column, cell)| {
                                div()
                                    .w(px(widths.get(column).copied().unwrap_or(0.)))
                                    .child(math_sequence(cell, palette, tone, size))
                            }),
                        )
                    }),
                ))
                .when(!right.is_empty(), |this| this.child(glyph(right)))
                .into_any_element()
        }
        // `\boxed{x}`: a frame drawn round the piece.
        markdown::MathNode::Framed(body) => div()
            .border_1()
            .border_color(rgb(tone))
            .px(px(size * 0.25))
            .py(px(1.))
            .child(math_piece(body, palette, tone, size))
            .into_any_element(),
        // Rows are split before a piece is drawn; a stray break draws nothing.
        markdown::MathNode::Break => div().into_any_element(),
    }
}

/// The drawn width of a piece, in pixels.
///
/// Pieces are drawn in the mono font, whose advance is about 0.62 em, and a stack
/// is as wide as its wider side. Rules are given this definite width because a
/// percentage inside a column that is sized by its content comes out as nothing.
pub(super) fn math_width(node: &markdown::MathNode, size: f32) -> f32 {
    match node {
        markdown::MathNode::Text(text) => {
            text.chars()
                .map(|character| {
                    // A CJK glyph is about twice as wide as a mono ASCII one.
                    if character.is_ascii() { 0.62 } else { 1.0 }
                })
                .sum::<f32>()
                * size
        }
        markdown::MathNode::Break => 0.,
        markdown::MathNode::Row(pieces) => math_nodes_width(pieces, size),
        markdown::MathNode::Frac(first, second) => {
            math_width(first, size).max(math_width(second, size)) + size * 0.4
        }
        markdown::MathNode::Sup(base, power) | markdown::MathNode::Sub(base, power) => {
            math_width(base, size) + math_width(power, size * MATH_SCRIPT)
        }
        markdown::MathNode::Sqrt(index, body) => {
            index
                .as_ref()
                .map(|index| math_width(index, size * MATH_SCRIPT))
                .unwrap_or(0.)
                + size * 1.0
                + math_width(body, size)
        }
        markdown::MathNode::Binom(top, bottom) => {
            math_width(top, size).max(math_width(bottom, size)) + size * 1.4
        }
        markdown::MathNode::Overline(body, _) => math_width(body, size) + size * 0.2,
        markdown::MathNode::Framed(body) => math_width(body, size) + size * 0.8,
        markdown::MathNode::Matrix(rows, bracket) => {
            let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
            let mut total = 0.;
            for column in 0..columns {
                total += rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| math_nodes_width(cell, size))
                    .fold(0., f32::max);
            }
            total
                + columns.saturating_sub(1) as f32 * 8.
                + if bracket.is_some() { size * 2.4 } else { 0. }
        }
    }
}

/// The drawn width of a run of pieces.
pub(super) fn math_nodes_width(nodes: &[markdown::MathNode], size: f32) -> f32 {
    nodes.iter().map(|node| math_width(node, size)).sum()
}

/// A big operator with its limits, when `node` reads as one.
///
/// `\sum_{i=1}^{n}` parses as `Sup(Sub(Text("∑"), i=1), n)`, and a sum with only a
/// subscript as `Sub(Text("∑"), i=1)`.
pub(super) fn big_operator(
    node: &markdown::MathNode,
) -> Option<(
    &'static str,
    Option<&markdown::MathNode>,
    Option<&markdown::MathNode>,
)> {
    match node {
        markdown::MathNode::Sup(base, power) => {
            if let Some(operator) = operator_of(base) {
                return Some((operator, None, Some(power)));
            }
            if let markdown::MathNode::Sub(inner, lower) = base.as_ref()
                && let Some(operator) = operator_of(inner)
            {
                return Some((operator, Some(lower), Some(power)));
            }
            None
        }
        markdown::MathNode::Sub(base, power) => {
            operator_of(base).map(|operator| (operator, Some(power.as_ref()), None))
        }
        _ => None,
    }
}

/// The big operator a node stands for, if it is one.
pub(super) fn operator_of(node: &markdown::MathNode) -> Option<&'static str> {
    let markdown::MathNode::Text(text) = node else {
        return None;
    };
    Some(match text.as_str() {
        "∑" => "∑",
        "∏" => "∏",
        "∫" => "∫",
        "∮" => "∮",
        "⋃" => "⋃",
        "⋂" => "⋂",
        // Names whose limit sits under the word itself.
        "lim" => "lim",
        "limsup" => "lim sup",
        "liminf" => "lim inf",
        "argmax" => "argmax",
        "argmin" => "argmin",
        "max" => "max",
        "min" => "min",
        "sup" => "sup",
        "inf" => "inf",
        "det" => "det",
        "gcd" => "gcd",
        "Pr" => "Pr",
        _ => return None,
    })
}

/// An operator with its limits above and below, one size up.
pub(super) fn math_limits(
    operator: &str,
    lower: Option<&markdown::MathNode>,
    upper: Option<&markdown::MathNode>,
    palette: Palette,
    tone: u32,
    size: f32,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .items_center()
        .children(upper.map(|node| math_piece(node, palette, tone, size * MATH_SCRIPT)))
        .child(
            div()
                .font_family(MONO_FONT)
                .text_size(px(size * 1.2))
                .text_color(rgb(tone))
                .child(operator.to_string()),
        )
        .children(lower.map(|node| math_piece(node, palette, tone, size * MATH_SCRIPT)))
        .into_any_element()
}

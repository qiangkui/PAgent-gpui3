//! LaTeX turned into one line of text, and into nodes the view lays out.

use super::*;

/// The runs of a formula's text: one code run, so it is drawn in the mono font
/// and a selection still copies it.
pub(crate) fn math_run(text: &str) -> Vec<Span> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![Span {
        len: text.len(),
        bold: false,
        italic: false,
        code: true,
        strike: false,
        underline: false,
        marked: false,
        token: None,
    }]
}

/// Split a formula into the pieces the view lays out.
pub(crate) fn math_nodes(source: &str) -> Vec<MathNode> {
    math_nodes_of(&source.chars().collect::<Vec<char>>())
}

/// The same, over characters already split.
pub(crate) fn math_nodes_of(characters: &[char]) -> Vec<MathNode> {
    let mut nodes: Vec<MathNode> = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        match character {
            '^' | '_' => match math_argument(characters, index + 1) {
                Some((argument, next)) => {
                    let base = take_base(&mut nodes, &mut text);
                    let argument = Box::new(argument);
                    nodes.push(if character == '^' {
                        MathNode::Sup(Box::new(base), argument)
                    } else {
                        MathNode::Sub(Box::new(base), argument)
                    });
                    index = next;
                }
                None => {
                    text.push(character);
                    index += 1;
                }
            },
            '\\' => {
                let start = index + 1;
                let mut end = start;
                while characters
                    .get(end)
                    .is_some_and(|found| found.is_ascii_alphabetic())
                {
                    end += 1;
                }
                let name: String = characters[start..end].iter().collect();
                index = math_command(&name, characters, end, &mut nodes, &mut text);
            }
            '{' => match group(characters, index) {
                Some((inner, next)) => {
                    flush_text(&mut nodes, &mut text);
                    nodes.extend(math_nodes_of(&inner));
                    index = next;
                }
                None => {
                    text.push('{');
                    index += 1;
                }
            },
            _ => {
                text.push(character);
                index += 1;
            }
        }
    }
    flush_text(&mut nodes, &mut text);
    nodes
}

/// One LaTeX command in a formula → where the text after it starts.
///
/// The pieces that need laying out become nodes; everything else is appended to
/// the pending run of text.
pub(crate) fn math_command(
    name: &str,
    characters: &[char],
    after: usize,
    nodes: &mut Vec<MathNode>,
    text: &mut String,
) -> usize {
    match name {
        "frac" | "dfrac" | "tfrac" => match math_argument(characters, after) {
            Some((first, after_first)) => match math_argument(characters, after_first) {
                Some((second, after_second)) => {
                    flush_text(nodes, text);
                    nodes.push(MathNode::Frac(Box::new(first), Box::new(second)));
                    after_second
                }
                None => {
                    text.push('\\');
                    text.push_str(name);
                    after
                }
            },
            None => {
                text.push('\\');
                text.push_str(name);
                after
            }
        },
        "sqrt" => {
            // `\sqrt[3]{x}` carries its index in brackets.
            let mut from = after;
            let mut degree: Option<MathNode> = None;
            if characters.get(from) == Some(&'[')
                && let Some(close) = characters[from + 1..]
                    .iter()
                    .position(|found| *found == ']')
            {
                let index_text: String = characters[from + 1..from + 1 + close].iter().collect();
                degree = Some(MathNode::Text(latex(&index_text)));
                from += close + 2;
            }
            match math_argument(characters, from) {
                Some((body, next)) => {
                    flush_text(nodes, text);
                    nodes.push(MathNode::Sqrt(degree.map(Box::new), Box::new(body)));
                    next
                }
                None => {
                    text.push('√');
                    after
                }
            }
        }
        "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" | "operatorname" | "mbox" => {
            match group(characters, after) {
                Some((inner, next)) => {
                    let literal: String = inner.iter().collect();
                    text.push_str(&literal);
                    next
                }
                None => after,
            }
        }
        "begin" => match group(characters, after) {
            Some((name, next)) => {
                let environment: String = name.iter().collect();
                let closing: Vec<char> = format!("\\end{{{environment}}}").chars().collect();
                let mut end = next;
                while end + closing.len() <= characters.len()
                    && characters[end..end + closing.len()] != closing[..]
                {
                    end += 1;
                }
                let body: String = characters[next..end].iter().collect();
                let after_end = (end + closing.len()).min(characters.len());
                // A matrix draws as a grid; anything else stays a line of text.
                match matrix_cells(&environment, &body) {
                    Some(cells) => {
                        flush_text(nodes, text);
                        nodes.push(MathNode::Matrix(cells, bracket_of(&environment)));
                    }
                    None => text.push_str(&environment_text(&environment, &body)),
                }
                after_end
            }
            None => after,
        },
        "binom" | "dbinom" | "tbinom" => match math_argument(characters, after) {
            Some((top, after_top)) => match math_argument(characters, after_top) {
                Some((bottom, next)) => {
                    flush_text(nodes, text);
                    nodes.push(MathNode::Binom(Box::new(top), Box::new(bottom)));
                    next
                }
                None => after,
            },
            None => after,
        },
        "overline" | "underline" => match math_argument(characters, after) {
            Some((body, next)) => {
                flush_text(nodes, text);
                nodes.push(MathNode::Overline(Box::new(body), name == "overline"));
                next
            }
            None => after,
        },
        // `\overbrace{x}` / `\underbrace{x}` draw a brace that text has no
        // character for; a rule above (or below) the piece stands in for it.
        "overbrace" | "underbrace" => match math_argument(characters, after) {
            Some((body, next)) => {
                flush_text(nodes, text);
                nodes.push(MathNode::Overline(Box::new(body), name == "overbrace"));
                next
            }
            None => after,
        },
        // `\boxed{x}` / `\fbox{x}`: the piece inside a drawn frame.
        "boxed" | "fbox" => match math_argument(characters, after) {
            Some((body, next)) => {
                flush_text(nodes, text);
                nodes.push(MathNode::Framed(Box::new(body)));
                next
            }
            None => after,
        },
        // `\boldsymbol{\alpha}` / `\bm{x}` only change the weight; the piece
        // itself is drawn as it is.
        "boldsymbol" | "bm" | "pmb" => match group(characters, after) {
            Some((inner, next)) => {
                flush_text(nodes, text);
                nodes.extend(math_nodes_of(&inner));
                next
            }
            None => after,
        },
        // `\color{red}` / `\textcolor{red}{x}`: the colour is dropped and the
        // text it would have coloured stays.
        "color" | "textcolor" | "colorbox" => match group(characters, after) {
            Some((_, next)) => next,
            None => after,
        },
        // `\tag{1}` is a number for a typesetter, not part of the formula.
        "tag" | "notag" | "nonumber" => match group(characters, after) {
            Some((_, next)) => next,
            None => after,
        },
        // `\cancel{x}` / `\substack{…}` only group their content.
        "cancel" | "bcancel" | "xcancel" | "sout" | "substack" => match group(characters, after) {
            Some((inner, next)) => {
                flush_text(nodes, text);
                nodes.extend(math_nodes_of(&inner));
                next
            }
            None => after,
        },
        // Spacing words are a space each; `\!` is handled with the punctuation
        // commands below.
        "quad" | "qquad" | "enspace" | "thinspace" | "medspace" | "thickspace" | "space" => {
            text.push(' ');
            after
        }
        // Layout and style commands draw nothing: `\left(` is the bracket
        // itself, `\displaystyle` only tells a typesetter how big to set the
        // rest.
        "left" | "right" | "middle" | "displaystyle" | "textstyle" | "scriptstyle"
        | "scriptscriptstyle" | "limits" | "nolimits" | "big" | "Big" | "bigg" | "Bigg"
        | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" => {
            let mut next = after;
            if matches!(name, "left" | "right" | "middle") {
                // `\left.` is an invisible delimiter.
                if characters.get(next) == Some(&'.') {
                    next += 1;
                }
            } else if characters.get(next) == Some(&' ') {
                next += 1;
            }
            next
        }
        // `\mathbb{R}`: the blackboard letters Unicode has.
        "mathbb" => match group(characters, after) {
            Some((inner, next)) => {
                let literal: String = inner.iter().collect();
                text.push_str(&blackboard(&literal));
                next
            }
            None => after,
        },
        // A space command (`\,`, `\quad`), a line break or an escaped
        // character. The character after the backslash belongs to the command,
        // so it is consumed here; leaving it behind turned `\,dx` into `,dx`.
        // A line break (`\\`) keeps the rows apart.
        "" => match characters.get(after).copied() {
            Some('\\') => {
                flush_text(nodes, text);
                nodes.push(MathNode::Break);
                after + 1
            }
            Some(',') | Some(';') | Some(':') | Some(' ') => {
                text.push(' ');
                after + 1
            }
            // `\!` takes a thin space away.
            Some('!') => after + 1,
            Some(literal) => {
                text.push(literal);
                after + 1
            }
            None => after,
        },
        _ => {
            if let Some(symbol) = latex_symbol(name) {
                text.push(symbol);
                return after;
            }
            if let Some(word) = function_name(name) {
                // The whole word is one piece, so `\sin^2 x` raises the name
                // rather than its last letter, and `\lim_{x \to 0}` can put its
                // limit under `lim`.
                flush_text(nodes, text);
                nodes.push(MathNode::Text(word.to_string()));
                return after;
            }
            // An accent keeps its argument with its combining mark.
            if let Some(mark) = accent_mark(name)
                && let Some((argument, next)) = math_argument(characters, after)
                && let MathNode::Text(literal) = argument
            {
                text.push_str(&literal);
                text.push(mark);
                return next;
            }
            // Unknown: keep the source, backslash and all.
            text.push('\\');
            text.push_str(name);
            after
        }
    }
}

/// The argument of a script or a command: a braced group, or one character.
pub(crate) fn math_argument(characters: &[char], from: usize) -> Option<(MathNode, usize)> {
    if characters.get(from) == Some(&'{') {
        let (inner, next) = group(characters, from)?;
        let nodes = math_nodes_of(&inner);
        let node = match nodes.len() {
            0 => MathNode::Text(String::new()),
            1 => nodes
                .into_iter()
                .next()
                .unwrap_or(MathNode::Text(String::new())),
            // Several pieces stay pieces: flattening them would lose the
            // exponents and fractions inside.
            _ => MathNode::Row(nodes),
        };
        return Some((node, next));
    }
    let character = characters.get(from).copied()?;
    Some((MathNode::Text(character.to_string()), from + 1))
}

/// A `{…}` group starting at `from` → its characters and where the text after it
/// starts.
pub(crate) fn group(characters: &[char], from: usize) -> Option<(Vec<char>, usize)> {
    if characters.get(from) != Some(&'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut end = from;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((characters[from + 1..end].to_vec(), end + 1));
                }
            }
            _ => {}
        }
        end += 1;
    }
    None
}

/// The base a script attaches to: the last character of the pending text, or the
/// node before it.
pub(crate) fn take_base(nodes: &mut Vec<MathNode>, text: &mut String) -> MathNode {
    if let Some(last) = text.pop() {
        if !text.is_empty() {
            nodes.push(MathNode::Text(std::mem::take(text)));
        }
        return MathNode::Text(last.to_string());
    }
    nodes.pop().unwrap_or_else(|| MathNode::Text(String::new()))
}

/// Append the pending text as a node, if there is any.
pub(crate) fn flush_text(nodes: &mut Vec<MathNode>, text: &mut String) {
    if !text.is_empty() {
        nodes.push(MathNode::Text(std::mem::take(text)));
    }
}

/// The words `\name` writes as they are (`\log`, `\lim`).
pub(crate) fn function_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "exp" => "exp",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "cot" => "cot",
        "sec" => "sec",
        "csc" => "csc",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "max" => "max",
        "min" => "min",
        "lim" => "lim",
        "limsup" => "limsup",
        "liminf" => "liminf",
        "argmax" => "argmax",
        "argmin" => "argmin",
        "det" => "det",
        "gcd" => "gcd",
        "Pr" => "Pr",
        "mod" => "mod",
        _ => return None,
    })
}

/// The combining mark an accent command puts after its argument.
pub(crate) fn accent_mark(name: &str) -> Option<char> {
    Some(match name {
        "vec" => '\u{20d7}',
        "bar" => '\u{0304}',
        "hat" | "widehat" => '\u{0302}',
        "dot" => '\u{0307}',
        "ddot" => '\u{0308}',
        "tilde" | "widetilde" => '\u{0303}',
        _ => return None,
    })
}

/// What is between the markers of a collected display formula.
pub(crate) fn formula_source(collected: &str, close: &str) -> String {
    let inner = collected.trim();
    let inner = if close == "$$" {
        inner.strip_prefix("$$").unwrap_or(inner)
    } else {
        inner.strip_prefix("\\[").unwrap_or(inner)
    };
    let inner = inner.strip_suffix(close).unwrap_or(inner);
    inner.trim().to_string()
}

/// Turn a formula's LaTeX into text the app can draw.
///
/// There is no typesetter here, but the text is made to read like maths: the
/// symbols of common LaTeX become Unicode (`\alpha`, `\times`, `\le`, `\sum`),
/// `\frac{a}{b}` becomes `a/b` with brackets where an operator needs them,
/// `\sqrt[3]{x}` becomes `∛x`, scripts become real superscripts and subscripts
/// (`x^2` → `x²`, `\sum_{i=1}^{n}` → `∑ᵢ₌₁ⁿ`), `\vec{v}` gets its combining
/// mark, `\mathbb{R}` becomes `ℝ`, and matrix-like environments are laid out a
/// row at a time. Anything unrecognised keeps its backslash so nothing is
/// silently lost.
pub(crate) fn latex(source: &str) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        if character != '\\' {
            // `^` and `_` take a braced group or one character. When every
            // character of the argument has a Unicode script form, that is what
            // is drawn (`x^2` → `x²`, `\sum_{i=1}^{n}` → `∑ᵢ₌₁ⁿ`); otherwise
            // the marker stays and the argument is bracketed.
            if character == '^' || character == '_' {
                let raised = character == '^';
                let (argument, next) = match characters.get(index + 1) {
                    Some('{') => braced(&characters, index + 1),
                    Some(other) => (Some(other.to_string()), index + 2),
                    None => (None, index + 1),
                };
                if let Some(argument) = argument {
                    match script(&argument, raised) {
                        Some(scripted) => out.push_str(&scripted),
                        None => {
                            out.push(character);
                            // The argument may hold commands of its own, so it is
                            // converted before it is bracketed.
                            let converted = latex(&argument);
                            if converted.chars().count() == 1 {
                                out.push_str(&converted);
                            } else {
                                out.push('(');
                                out.push_str(&converted);
                                out.push(')');
                            }
                        }
                    }
                    index = next;
                    continue;
                }
            }
            out.push(character);
            index += 1;
            continue;
        }
        // A command: `\name`, plus the argument `\frac` and `\sqrt` take.
        let start = index + 1;
        let mut end = start;
        while characters
            .get(end)
            .is_some_and(|found| found.is_ascii_alphabetic())
        {
            end += 1;
        }
        let name: String = characters[start..end].iter().collect();
        let after = end;
        match name.as_str() {
            // A space command (`\,`, `\quad`), a line break or an escaped
            // character. The character after the backslash belongs to the
            // command, so it is consumed here; leaving it behind turned `\,dx`
            // into `,dx`.
            "" => match characters.get(after).copied() {
                // `\\` starts a new line of the formula.
                Some('\\') => {
                    out.push('\n');
                    index = after + 1;
                }
                Some(',') | Some(';') | Some(':') | Some(' ') => {
                    out.push(' ');
                    index = after + 1;
                }
                // `\!` takes a thin space away.
                Some('!') => index = after + 1,
                Some(literal) => {
                    out.push(literal);
                    index = after + 1;
                }
                None => index = after,
            },
            "quad" | "qquad" | "enspace" | "thinspace" | "medspace" | "thickspace" | "space" => {
                out.push(' ');
                index = after;
            }
            // Style switches and `\left`/`\right` add nothing to the text: they
            // only tell a typesetter how to lay it out.
            "left" | "right" | "middle" | "displaystyle" | "textstyle" | "scriptstyle"
            | "scriptscriptstyle" | "limits" | "nolimits" | "big" | "Big" | "bigg" | "Bigg"
            | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" => {
                let mut next = after;
                if matches!(name.as_str(), "left" | "right" | "middle") {
                    // `\left.` is an invisible delimiter.
                    if characters.get(next) == Some(&'.') {
                        next += 1;
                    }
                } else if characters.get(next) == Some(&' ') {
                    // A style switch takes the space that separates it from the
                    // formula it applies to, so `\displaystyle \sum` does not
                    // leave a stray leading space.
                    next += 1;
                }
                index = next;
            }
            // `\overbrace{x}` / `\underbrace{x}` / `\boxed{x}` draw a brace or a
            // box that text has no character for: the piece inside stays.
            "overbrace" | "underbrace" | "boxed" | "fbox" => {
                let (argument, next) = braced(&characters, after);
                if let Some(argument) = argument {
                    out.push_str(&latex(&argument));
                    index = next;
                } else {
                    index = after;
                }
            }
            // `\color{red}{x}` / `\textcolor{red}{x}`: the colour is dropped and
            // the text it would have coloured stays.
            "color" | "textcolor" | "colorbox" => {
                let (_, next) = braced(&characters, after);
                let (argument, after_argument) = braced(&characters, next);
                match argument {
                    Some(argument) => {
                        out.push_str(&latex(&argument));
                        index = after_argument;
                    }
                    None => index = next,
                }
            }
            // `\tag{1}` numbers the equation for a typesetter; the number is not
            // part of the formula here.
            "tag" | "notag" | "nonumber" => {
                let (_, next) = braced(&characters, after);
                index = next;
            }
            // `\cancel{x}` / `\substack{…}` only group their content.
            "cancel" | "bcancel" | "xcancel" | "sout" | "substack" => {
                let (argument, next) = braced(&characters, after);
                if let Some(argument) = argument {
                    out.push_str(&latex(&argument));
                    index = next;
                } else {
                    index = after;
                }
            }
            "frac" | "dfrac" | "tfrac" => {
                let (first, after_first) = braced(&characters, after);
                let (second, after_second) = braced(&characters, after_first);
                match (first, second) {
                    (Some(first), Some(second)) => {
                        // The sides may hold commands of their own
                        // (`\frac{\partial f}{\partial x}`), so they are
                        // converted before they are read.
                        let first = latex(&first);
                        let second = latex(&second);
                        // `\frac{1}{2}` reads as `1/2`; a side with an operator
                        // in it gets brackets so the order survives.
                        let side = |argument: String| {
                            if simple(&argument) {
                                argument
                            } else {
                                format!("({argument})")
                            }
                        };
                        out.push_str(&format!("{}/{}", side(first), side(second)));
                        index = after_second;
                    }
                    _ => {
                        out.push('\\');
                        out.push_str(&name);
                        index = after;
                    }
                }
            }
            // `\binom{n}{k}` reads as `C(n, k)` in one line of text.
            "binom" | "dbinom" | "tbinom" => {
                let (first, after_first) = braced(&characters, after);
                let (second, after_second) = braced(&characters, after_first);
                match (first, second) {
                    (Some(first), Some(second)) => {
                        out.push_str(&format!("C({}, {})", latex(&first), latex(&second)));
                        index = after_second;
                    }
                    _ => {
                        out.push('\\');
                        out.push_str(&name);
                        index = after;
                    }
                }
            }
            "sqrt" => {
                // `\sqrt[3]{x}` carries its index in brackets.
                let mut from = after;
                let mut degree: Option<String> = None;
                if characters.get(from) == Some(&'[')
                    && let Some(close) = characters[from + 1..]
                        .iter()
                        .position(|found| *found == ']')
                {
                    degree = Some(characters[from + 1..from + 1 + close].iter().collect());
                    from += close + 2;
                }
                if let (Some(argument), next) = braced(&characters, from) {
                    let argument = latex(&argument);
                    match degree.as_deref() {
                        Some("3") => out.push('∛'),
                        Some("4") => out.push('∜'),
                        Some(degree) => {
                            out.push_str(
                                &script(degree, true).unwrap_or_else(|| format!("^{degree}")),
                            );
                            out.push('√');
                        }
                        None => out.push('√'),
                    }
                    if simple(&argument) {
                        out.push_str(&argument);
                    } else {
                        out.push('(');
                        out.push_str(&argument);
                        out.push(')');
                    }
                    index = next;
                } else {
                    out.push('√');
                    index = after;
                }
            }
            // Literal text (`\text{速度}`) is drawn as written; the alphabets
            // (`\mathbf`, `\boldsymbol`) still hold formulas, so their commands
            // are converted.
            "text" | "mbox" | "operatorname" => {
                if let (Some(argument), next) = braced(&characters, after) {
                    out.push_str(&argument);
                    index = next;
                } else {
                    index = after;
                }
            }
            "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" | "boldsymbol" | "bm" | "pmb" => {
                if let (Some(argument), next) = braced(&characters, after) {
                    out.push_str(&latex(&argument));
                    index = next;
                } else {
                    index = after;
                }
            }
            // `\vec{v}` and friends: the argument with a combining mark after
            // it, which is as close as text gets to an accent or a rule.
            "vec" | "bar" | "hat" | "dot" | "ddot" | "tilde" | "widehat" | "widetilde"
            | "overline" | "underline" => {
                if let (Some(argument), next) = braced(&characters, after) {
                    out.push_str(&latex(&argument));
                    out.push(match name.as_str() {
                        "vec" => '\u{20d7}',
                        "bar" | "overline" => '\u{0304}',
                        "hat" | "widehat" => '\u{0302}',
                        "dot" => '\u{0307}',
                        "ddot" => '\u{0308}',
                        "underline" => '\u{0332}',
                        _ => '\u{0303}',
                    });
                    index = next;
                } else {
                    index = after;
                }
            }
            // `\mathbb{R}`: the blackboard letters Unicode has.
            "mathbb" => {
                if let (Some(argument), next) = braced(&characters, after) {
                    out.push_str(&blackboard(&argument));
                    index = next;
                } else {
                    index = after;
                }
            }
            // `\begin{pmatrix} a & b \\ c & d \end{pmatrix}`: the rows are laid
            // out one after another, inside the bracket the environment names.
            "begin" => {
                if let (Some(environment), next) = braced(&characters, after) {
                    let closing: Vec<char> = format!("\\end{{{environment}}}").chars().collect();
                    let mut end = next;
                    let mut found = None;
                    while end + closing.len() <= characters.len() {
                        if characters[end..end + closing.len()] == closing[..] {
                            found = Some(end);
                            break;
                        }
                        end += 1;
                    }
                    match found {
                        Some(end) => {
                            let body: String = characters[next..end].iter().collect();
                            out.push_str(&environment_text(&environment, &body));
                            index = end + closing.len();
                        }
                        None => index = next,
                    }
                } else {
                    index = after;
                }
            }
            // `\log`, `\lim` and the rest are words, not symbols.
            "log" | "ln" | "lg" | "exp" | "sin" | "cos" | "tan" | "cot" | "sec" | "csc"
            | "arcsin" | "arccos" | "arctan" | "sinh" | "cosh" | "tanh" | "max" | "min" | "lim"
            | "det" | "gcd" | "mod" | "sup" | "inf" | "argmax" | "argmin" | "Pr" => {
                out.push_str(&name);
                index = after;
            }
            // `\limsup` and `\liminf` read as two words.
            "limsup" => {
                out.push_str("lim sup");
                index = after;
            }
            "liminf" => {
                out.push_str("lim inf");
                index = after;
            }
            _ => match latex_symbol(&name) {
                Some(symbol) => {
                    out.push(symbol);
                    index = after;
                }
                // Unknown: keep the source, backslash and all.
                None => {
                    out.push('\\');
                    out.push_str(&name);
                    index = after;
                }
            },
        }
    }
    out
}

/// The `{…}` argument starting at `from` → its text and where the text after it
/// starts.
pub(crate) fn braced(characters: &[char], from: usize) -> (Option<String>, usize) {
    if characters.get(from) != Some(&'{') {
        return (None, from);
    }
    let Some(close) = characters[from + 1..]
        .iter()
        .position(|found| *found == '}')
    else {
        return (None, from);
    };
    let text: String = characters[from + 1..from + 1 + close].iter().collect();
    (Some(text), from + close + 2)
}

/// The rows of a matrix-like environment → its cells, each split into nodes, or
/// `None` when the name is not one this app draws as a grid.
pub(crate) fn matrix_cells(name: &str, body: &str) -> Option<Vec<Vec<Vec<MathNode>>>> {
    let drawn = matches!(
        name,
        "matrix"
            | "array"
            | "aligned"
            | "align"
            | "align*"
            | "gathered"
            | "gather"
            | "split"
            | "subarray"
            | "alignedat"
            | "cases"
    ) || bracket_of(name).is_some();
    if !drawn {
        return None;
    }
    // `\begin{array}{cc}` and `\begin{alignedat}{2}` carry a column spec first.
    let stripped;
    let body = if matches!(name, "array" | "alignedat" | "subarray") {
        let characters: Vec<char> = body.chars().collect();
        stripped = match group(&characters, 0) {
            Some((_, next)) => characters[next..].iter().collect::<String>(),
            None => body.to_string(),
        };
        stripped.as_str()
    } else {
        body
    };
    let rows: Vec<Vec<Vec<MathNode>>> = body
        .split("\\\\")
        .map(|row| {
            row.split('&')
                .map(|cell| math_nodes(cell.trim()))
                .collect::<Vec<_>>()
        })
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .collect();
    if rows.is_empty() {
        return None;
    }
    Some(rows)
}

/// The bracket an environment draws round its grid, if any.
pub(crate) fn bracket_of(name: &str) -> Option<Bracket> {
    Some(match name {
        "pmatrix" => Bracket::Paren,
        "bmatrix" => Bracket::Square,
        "Bmatrix" | "cases" => Bracket::Brace,
        "vmatrix" | "Vmatrix" => Bracket::Bar,
        _ => return None,
    })
}

/// A matrix-like environment as text: rows one after another, cells separated by
/// a space, the whole inside whatever bracket the name asks for.
pub(crate) fn environment_text(name: &str, body: &str) -> String {
    let rows: Vec<String> = body
        .split("\\\\")
        .map(|row| {
            row.split('&')
                .map(|cell| latex(cell.trim()))
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|row| !row.is_empty())
        .collect();
    let joined = rows.join("; ");
    match name {
        "pmatrix" => format!("({joined})"),
        "bmatrix" => format!("[{joined}]"),
        "vmatrix" => format!("|{joined}|"),
        "cases" => format!("{{{joined}}}"),
        _ => joined,
    }
}

/// The blackboard-bold letters `\mathbb` has a Unicode form for.
pub(crate) fn blackboard(argument: &str) -> String {
    argument
        .chars()
        .map(|character| match character {
            'R' => 'ℝ',
            'N' => 'ℕ',
            'Z' => 'ℤ',
            'Q' => 'ℚ',
            'C' => 'ℂ',
            'P' => 'ℙ',
            'H' => 'ℍ',
            other => other,
        })
        .collect()
}

/// Whether an argument can sit in `a/b` without brackets.
pub(crate) fn simple(argument: &str) -> bool {
    !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character.is_whitespace() || "+-*/=<>".contains(character))
}

/// The Unicode a `\name` stands for, `None` when the app does not know it.
pub(crate) fn latex_symbol(name: &str) -> Option<char> {
    Some(match name {
        "alpha" => 'α',
        "beta" => 'β',
        "gamma" => 'γ',
        "delta" => 'δ',
        "epsilon" => 'ε',
        "zeta" => 'ζ',
        "eta" => 'η',
        "theta" => 'θ',
        "iota" => 'ι',
        "kappa" => 'κ',
        "lambda" => 'λ',
        "mu" => 'μ',
        "nu" => 'ν',
        "xi" => 'ξ',
        "pi" => 'π',
        "rho" => 'ρ',
        "sigma" => 'σ',
        "tau" => 'τ',
        "upsilon" => 'υ',
        "phi" => 'φ',
        "chi" => 'χ',
        "psi" => 'ψ',
        "omega" => 'ω',
        "Gamma" => 'Γ',
        "Delta" => 'Δ',
        "Theta" => 'Θ',
        "Lambda" => 'Λ',
        "Xi" => 'Ξ',
        "Pi" => 'Π',
        "Sigma" => 'Σ',
        "Phi" => 'Φ',
        "Psi" => 'Ψ',
        "Omega" => 'Ω',
        "times" => '×',
        "cdot" => '·',
        "div" => '÷',
        "pm" => '±',
        "mp" => '∓',
        "le" | "leq" => '≤',
        "ge" | "geq" => '≥',
        "ne" | "neq" => '≠',
        "approx" => '≈',
        "equiv" => '≡',
        "infty" => '∞',
        "to" | "rightarrow" => '→',
        "leftarrow" => '←',
        "Rightarrow" => '⇒',
        "Leftarrow" => '⇐',
        "leftrightarrow" => '↔',
        "sum" => '∑',
        "prod" => '∏',
        "int" => '∫',
        "partial" => '∂',
        "nabla" => '∇',
        "in" => '∈',
        "notin" => '∉',
        "subset" => '⊂',
        "subseteq" => '⊆',
        "supset" => '⊃',
        "cup" => '∪',
        "cap" => '∩',
        "forall" => '∀',
        "exists" => '∃',
        "emptyset" => '∅',
        "angle" => '∠',
        "perp" => '⊥',
        "ldots" | "dots" => '…',
        "cdots" => '⋯',
        "degree" => '°',
        "circ" => '∘',
        "prime" => '′',
        "omicron" => 'ο',
        "varepsilon" => 'ε',
        "vartheta" => 'ϑ',
        "varphi" => 'ϕ',
        "varrho" => 'ϱ',
        "varsigma" => 'ς',
        "varpi" => 'ϖ',
        "langle" => '⟨',
        "rangle" => '⟩',
        "lvert" | "rvert" | "vert" => '|',
        "lVert" | "rVert" | "Vert" => '‖',
        "lfloor" => '⌊',
        "rfloor" => '⌋',
        "lceil" => '⌈',
        "rceil" => '⌉',
        "ll" => '≪',
        "gg" => '≫',
        "sim" => '∼',
        "simeq" => '≃',
        "cong" => '≅',
        "propto" => '∝',
        "asymp" => '≍',
        "doteq" => '≐',
        "leqslant" => '⩽',
        "geqslant" => '⩾',
        "mid" => '∣',
        "parallel" => '∥',
        "nmid" => '∤',
        "oplus" => '⊕',
        "ominus" => '⊖',
        "otimes" => '⊗',
        "oslash" => '⊘',
        "odot" => '⊙',
        "uplus" => '⊎',
        "sqcup" => '⊔',
        "sqcap" => '⊓',
        "ast" => '∗',
        "star" => '⋆',
        "bullet" => '∙',
        "dagger" => '†',
        "ddagger" => '‡',
        "therefore" => '∴',
        "because" => '∵',
        "aleph" => 'ℵ',
        "hbar" => 'ℏ',
        "ell" => 'ℓ',
        "Re" => 'ℜ',
        "Im" => 'ℑ',
        "wp" => '℘',
        "wedge" | "land" => '∧',
        "vee" | "lor" => '∨',
        "neg" | "lnot" => '¬',
        "vdash" => '⊢',
        "dashv" => '⊣',
        "models" => '⊨',
        "ni" => '∋',
        "supseteq" => '⊇',
        "subsetneq" => '⊊',
        "supsetneq" => '⊋',
        "setminus" => '∖',
        "triangle" => '△',
        "square" => '□',
        "diamond" => '⋄',
        "top" => '⊤',
        "bot" => '⊥',
        "uparrow" => '↑',
        "downarrow" => '↓',
        "Uparrow" => '⇑',
        "Downarrow" => '⇓',
        "Leftrightarrow" => '⇔',
        "mapsto" => '↦',
        "hookrightarrow" => '↪',
        "implies" => '⟹',
        "iff" => '⟺',
        "vdots" => '⋮',
        "ddots" => '⋱',
        "colon" => ':',
        "lt" => '<',
        "gt" => '>',
        "varnothing" => '∅',
        _ => return None,
    })
}

/// `text` in Unicode superscripts (`raised`) or subscripts, `None` when any
/// character has no such form.
///
/// Whole groups are converted so a formula reads like maths instead of source:
/// `\sum_{i=1}^{n}` becomes `∑ᵢ₌₁ⁿ` rather than `∑_(i=1)^(n)`.
pub(crate) fn script(text: &str, raised: bool) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        let mapped = if raised {
            super_script(character)
        } else {
            sub_script(character)
        };
        out.push(mapped?);
    }
    Some(out)
}

/// The Unicode superscript of a character, `None` when there is none.
pub(crate) fn super_script(character: char) -> Option<char> {
    Some(match character {
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
        '+' => '⁺',
        '-' => '⁻',
        '=' => '⁼',
        '(' => '⁽',
        ')' => '⁾',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        _ => return None,
    })
}

/// The Unicode subscript of a character, `None` when there is none.
pub(crate) fn sub_script(character: char) -> Option<char> {
    Some(match character {
        '0' => '₀',
        '1' => '₁',
        '2' => '₂',
        '3' => '₃',
        '4' => '₄',
        '5' => '₅',
        '6' => '₆',
        '7' => '₇',
        '8' => '₈',
        '9' => '₉',
        '+' => '₊',
        '-' => '₋',
        '=' => '₌',
        '(' => '₍',
        ')' => '₎',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        _ => return None,
    })
}

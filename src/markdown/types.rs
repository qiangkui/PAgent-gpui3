//! The data model of a drawn answer: blocks, spans, links, tables and formulas.

use super::*;

/// One piece of an answer, in display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Plain lines, joined with `\n`.
    Paragraph(String),
    /// `#`…`######` heading: level and text.
    Heading(usize, String),
    /// `- ` / `* ` / `+ ` item: how deeply it is nested and its text.
    Bullet(usize, String),
    /// `- [ ] ` / `- [x] ` item: how deeply it is nested, whether it is ticked,
    /// and its text. The box is drawn in the marker column, so the text does not
    /// carry it.
    Task(usize, bool, String),
    /// `1. ` item: how deeply it is nested, the marker (`1.`) and the text.
    Numbered(usize, String, String),
    /// `> ` quote: the blocks its lines parse into, drawn inside one box.
    Quote(Vec<Rich>),
    /// `术语` + `: 说明` (definition list): the whole entry as drawn, with the
    /// term on its own (bold) line and the description under it.
    ///
    /// It is one block of text rather than a term and a description apart, so a
    /// selection copies 「术语 + 换行 + 说明」 in one drag.
    Definition(String),
    /// A `---` line: a separator between sections.
    Rule,
    /// A header row and the rows under it.
    Table(Table),
    /// `[^1]: …`: the number drawn in the marker column and the note's own
    /// blocks.
    ///
    /// The definitions are collected before parsing and appended at the end of
    /// the answer, so the text reads in order but the notes stay together.
    /// A note may hold a list, a fence or a table of its own.
    Footnote(usize, Vec<Rich>),
    /// A block written under a list item — a fence, quote, table, image or a
    /// deeper block — inset by the item's level so it reads as part of that
    /// item. The marker says which column the item's text starts at, so a child
    /// lines up under the words whether the item is a bullet or a number.
    Indented(usize, Marker, Box<Rich>),
    /// `![说明](地址)` on a line of its own: the alt text and the address.
    ///
    /// Nothing here fetches the network: a local file is drawn by the view, and
    /// any other address draws a placeholder that names the image instead of a
    /// link in the middle of a sentence.
    Image(String, String),
    /// `$$…$$` or `\[…\]`: a formula on a line of its own, laid out by the view.
    Math(Formula),
    /// `<details><summary>…</summary>…</details>`: the summary line, whether it
    /// starts open, and the blocks inside.
    ///
    /// The summary is a parsed cell, so `**粗**` and `` `代码` `` in it keep
    /// their emphasis; its links are not clickable (the line toggles instead).
    Details(Cell, bool, Vec<Rich>),
    /// A ``` fence: language tag (empty when none) and the body.
    Code(String, String),
}

/// Which marker column a list item's text starts after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// A bullet or a task's box.
    Bullet,
    /// A number (`1.`, `1)`), which is wider than a bullet.
    Number,
}

/// A Markdown table, one cell per entry.
///
/// Every row carries as many cells as the widest row, so the chat can lay the
/// table out as a grid; short rows are padded with empty cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<Cell>,
    pub rows: Vec<Vec<Cell>>,
    /// How each column lines up, from the `:--` marks in the rule.
    pub aligns: Vec<Align>,
}

/// One cell: its text, plus how that text is emphasised.
///
/// The text has its markers removed already, and `spans` walks it from the first
/// byte to the last, so a byte offset in the table's text (which is what a
/// selection and the clipboard use) always lands on a painted character.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub spans: Vec<Span>,
    /// The `[label](url)` links of this text, by byte range.
    pub links: Vec<Link>,
}

/// One `[label](url)` link: where its label sits in the drawn text, and where
/// it points.
///
/// The label is drawn (underlined) instead of the Markdown around it, so a click
/// can open the address without the raw syntax showing in the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// First byte of the label.
    pub start: usize,
    /// One past the label's last byte.
    pub end: usize,
    /// Where a click goes.
    pub url: String,
}

impl Link {
    /// Whether `index` falls on the label, which is what a click tests.
    pub fn contains(&self, index: usize) -> bool {
        self.start <= index && index < self.end
    }
}

/// Whether a URL is one the app will open.
///
/// Answers are untrusted text: only `http`, `https` and `mailto` are ever handed
/// to the system opener, so a `file:` or `javascript:` "link" cannot run
/// anything. `mailto:` is here because an address in angle brackets is an
/// autolink in Markdown, and the system's handler only opens a mail window.
pub fn is_openable_link(url: &str) -> bool {
    let url = url.trim();
    (url.starts_with("http://") && url.len() > "http://".len())
        || (url.starts_with("https://") && url.len() > "https://".len())
        || url
            .strip_prefix("mailto:")
            .is_some_and(|rest| !rest.is_empty() && !rest.chars().any(char::is_whitespace))
}

/// Whether `text` is one mail address and nothing else: `user@example.com`.
pub(crate) fn is_email(text: &str) -> bool {
    let Some((user, host)) = text.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !user.contains('@')
        && !user.chars().any(char::is_whitespace)
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
}

/// A stretch of text drawn in one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Length in bytes of the text this run covers.
    pub len: usize,
    /// `**bold**`.
    pub bold: bool,
    /// `*italic*` (and `***bold italic***`).
    pub italic: bool,
    /// `backticks`: drawn in the code font, on the same background as the text.
    pub code: bool,
    /// `~~struck~~`: drawn with a line through it.
    pub strike: bool,
    /// `<u>` / `<ins>`: drawn with a line under it.
    pub underline: bool,
    /// `<mark>`: drawn on a highlighted background.
    pub marked: bool,
    /// The kind of code token this run is, for the colours of a code block.
    pub token: Option<Token>,
}

/// What a run of code is, so the view can colour it.
///
/// The parser keeps the *kind*, not the colour: which colour a keyword gets is
/// the theme's business, and a tokenised block parses the same in both themes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Comment,
    String,
    Number,
    Keyword,
    /// A name that is called: `foo(`.
    Function,
    /// A name that reads as a type: `String`, `Vec`.
    Type,
    /// A line a `diff` adds: `+新增`.
    Added,
    /// A line a `diff` removes: `-删除`.
    Removed,
}

impl From<&str> for Cell {
    fn from(text: &str) -> Self {
        let mut cell = Cell {
            text: text.to_string(),
            spans: Vec::new(),
            links: Vec::new(),
        };
        if !cell.text.is_empty() {
            cell.spans.push(Span {
                len: cell.text.len(),
                bold: false,
                italic: false,
                code: false,
                strike: false,
                underline: false,
                marked: false,
                token: None,
            });
        }
        cell
    }
}

impl Cell {
    /// The cell as Markdown: its bold, italic and code runs get their markers
    /// back, so copying a table keeps the emphasis it was drawn with.
    ///
    /// A link's brackets are placed where its label starts and ends, which lines
    /// up with the span boundaries because [`styled`] keeps the label's runs
    /// separate from the text around it.
    pub fn markdown(&self) -> String {
        let mut out = String::with_capacity(self.text.len());
        let mut start = 0;
        for span in &self.spans {
            let end = start + span.len;
            let raw = self.text.get(start..end).unwrap_or_default();
            let mut piece = if span.code {
                // As many backticks as it takes to hold the run already in the
                // text, and padding when the content starts or ends with one.
                let longest = raw
                    .split(|found| found != '`')
                    .map(str::len)
                    .max()
                    .unwrap_or(0);
                let ticks = "`".repeat(longest + 1);
                let pad = if raw.starts_with(['`', ' ']) || raw.ends_with(['`', ' ']) {
                    " "
                } else {
                    ""
                };
                format!("{ticks}{pad}{raw}{pad}{ticks}")
            } else {
                raw.to_string()
            };
            if span.strike {
                piece = format!("~~{piece}~~");
            }
            // `==高亮==` is the Markdown form of a highlighted run; `<mark>` was
            // the only way to get one before, and there is no Markdown spelling
            // for an underline to copy back to.
            if span.marked {
                piece = format!("=={piece}==");
            }
            if span.italic {
                piece = format!("*{piece}*");
            }
            if span.bold {
                piece = format!("**{piece}**");
            }
            // A bare address is its own label: it was written as the address, so
            // copying it back must not turn it into `[address](address)`. A
            // `www.…` address is drawn without the scheme it opens with.
            let explicit =
                |link: &&Link| link.url != raw && link.url.strip_prefix("https://") != Some(raw);
            if self
                .links
                .iter()
                .any(|link| link.start == start && explicit(&link))
            {
                piece = format!("[{piece}");
            }
            out.push_str(&piece);
            if let Some(link) = self
                .links
                .iter()
                .find(|link| link.end == end && explicit(link))
            {
                out.push_str(&format!("]({})", link.url));
            }
            start = end;
        }
        out
    }
}

/// Where a table column's text sits. Markdown says it on the rule line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// `|---|` or `|:--|`: the default.
    #[default]
    Left,
    /// `|:--:|`.
    Center,
    /// `|--:|`, the usual choice for numbers.
    Right,
}

impl Table {
    /// The table as plain text: the form selection and the clipboard work in.
    ///
    /// Cells are joined with `" | "` and rows with `\n`, and the drawing code
    /// ([`crate::select_text`]) walks the same cells to map a drag back to a byte
    /// index, so this spelling has to stay in step with that layout.
    pub fn text(&self) -> String {
        let mut out = String::new();
        let rows = std::iter::once(&self.header).chain(&self.rows);
        for (index, row) in rows.enumerate() {
            if index > 0 {
                out.push('\n');
            }
            let cells: Vec<&str> = row.iter().map(|cell| cell.text.as_str()).collect();
            out.push_str(&cells.join(" | "));
        }
        out
    }

    /// The table as a Markdown snippet: header, rule with the alignment marks,
    /// then the rows.
    ///
    /// This is what a table's copy button puts on the clipboard, so it can be
    /// pasted into anything that understands Markdown and still line up.
    pub fn markdown(&self) -> String {
        let mut out = String::new();
        write_row(&mut out, &self.header);
        out.push('\n');
        out.push('|');
        for align in &self.aligns {
            out.push_str(match align {
                Align::Left => " --- |",
                Align::Center => " :--: |",
                Align::Right => " ---: |",
            });
        }
        for row in &self.rows {
            out.push('\n');
            write_row(&mut out, row);
        }
        out
    }
}

/// `| a | b |`, with any pipe inside a cell escaped again.
pub(crate) fn write_row(out: &mut String, cells: &[Cell]) {
    out.push('|');
    for cell in cells {
        out.push(' ');
        out.push_str(&cell.markdown().replace('|', "\\|"));
        out.push_str(" |");
    }
}

/// One block of an answer, with the emphasis covering its text.
///
/// The block's own text has its markers removed; `spans` walks that text from
/// the first byte to the last, so a renderer can draw `**bold**` bold and
/// `` `code` `` in the code font while a selection still copies plain words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rich {
    pub block: Block,
    pub spans: Vec<Span>,
    /// The `[label](url)` links of the block's text, by byte range.
    pub links: Vec<Link>,
}

/// The reference links an answer defines: a normalised label → its address.
pub(crate) type Refs = std::collections::HashMap<String, String>;

/// What an answer defines for the inline text to point at: `[label]` links and
/// `[^note]` footnotes (a normalised label → the number it is drawn with).
#[derive(Default)]
pub(crate) struct Definitions {
    pub(crate) links: Refs,
    pub(crate) notes: std::collections::HashMap<String, usize>,
}

/// One piece of a display formula, laid out by the view.
///
/// The parts text cannot show — a stacked fraction, a raised script, a root with
/// its bar — are nodes of their own; everything else is a run of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MathNode {
    /// Ordinary text, drawn as it is.
    Text(String),
    /// Several pieces side by side, kept apart so their own layout survives
    /// (`\frac{x^2 + y^2}{2}` keeps the exponents).
    Row(Vec<MathNode>),
    /// `\\`: the formula starts a new line here.
    Break,
    /// `\frac{a}{b}`: numerator over denominator.
    Frac(Box<MathNode>, Box<MathNode>),
    /// `x^2`: the exponent, raised and smaller.
    Sup(Box<MathNode>, Box<MathNode>),
    /// `x_i`: the index, lowered and smaller.
    Sub(Box<MathNode>, Box<MathNode>),
    /// `\sqrt{x}`: a root, with its index when one was written.
    Sqrt(Option<Box<MathNode>>, Box<MathNode>),
    /// `\binom{n}{k}`: n over k inside brackets, with no bar between them.
    Binom(Box<MathNode>, Box<MathNode>),
    /// `\overline{x}` / `\underline{x}`: a rule above (or below) the piece.
    Overline(Box<MathNode>, bool),
    /// `\boxed{x}`: the piece inside a drawn frame.
    Framed(Box<MathNode>),
    /// `\begin{pmatrix}…\end{pmatrix}` and the like: rows of cells (each cell a
    /// row of nodes), with the bracket the environment draws round them, if any.
    Matrix(Vec<Vec<Vec<MathNode>>>, Option<Bracket>),
}

/// The bracket a matrix-like environment draws round its grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bracket {
    /// `pmatrix`: round brackets.
    Paren,
    /// `bmatrix`: square brackets.
    Square,
    /// `vmatrix`: vertical bars.
    Bar,
    /// `Bmatrix` and `cases`: braces.
    Brace,
}

impl MathNode {
    /// Whether the node has to be drawn rather than written as text.
    fn is_compound(&self) -> bool {
        match self {
            Self::Text(_) | Self::Break => false,
            Self::Row(..)
            | Self::Frac(..)
            | Self::Sup(..)
            | Self::Sub(..)
            | Self::Sqrt(..)
            | Self::Binom(..)
            | Self::Overline(..)
            | Self::Framed(..)
            | Self::Matrix(..) => true,
        }
    }
}

/// A display formula: what it was written as, its flat text, and the nodes the
/// view lays out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formula {
    /// The LaTeX, for the copy button.
    pub source: String,
    /// The formula as one line of text, for a selection and the clipboard.
    pub text: String,
    /// What to draw; empty when the formula is drawn from `text`.
    pub nodes: Vec<MathNode>,
}

impl Formula {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.trim().to_string(),
            text: latex(source),
            nodes: math_nodes(source),
        }
    }

    /// Whether the view has to lay this formula out instead of drawing its text.
    ///
    /// A formula with nothing to stack stays one line of text, which keeps it
    /// selectable; stacking is only worth losing that for when it says something
    /// the line cannot.
    pub fn stacked(&self) -> bool {
        self.nodes.iter().any(MathNode::is_compound)
    }
}

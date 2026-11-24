//! Unit tests for the Markdown parser, grouped by the part they cover.

use super::*;

macro_rules! row {
    ($($cell:expr),* $(,)?) => { vec![$(Cell::from($cell)),*] };
}

mod blocks;
mod highlight;
mod inline;
mod math;
mod misc;

/// A run of `len` bytes with the emphasis a test cares about.
fn span(len: usize, bold: bool, italic: bool, code: bool) -> Span {
    Span {
        len,
        bold,
        italic,
        code,
        strike: false,
        underline: false,
        marked: false,
        token: None,
    }
}

/// A quote that holds one plain paragraph, for the assertions that only care
/// about the quoted words.
fn quoted(text: &str) -> Block {
    Block::Quote(vec![para(text)])
}

/// One plain paragraph, with the single run the parser would give it.
fn para(text: &str) -> Rich {
    Rich {
        block: Block::Paragraph(text.to_string()),
        spans: vec![span(text.len(), false, false, false)],
        links: Vec::new(),
    }
}

/// The text a block draws, `None` for the blocks that draw none.
///
/// Mirrors `app::message::block_body`, which is what a selection and the
/// clipboard index into. A quote draws no text of its own: its lines are
/// blocks inside it, and they are checked on their own.
fn body_of(block: &Block) -> Option<&str> {
    match block {
        Block::Paragraph(text) => Some(text),
        Block::Heading(_, text) | Block::Bullet(_, text) => Some(text),
        Block::Task(_, _, text) => Some(text),
        Block::Numbered(_, _, text) => Some(text),
        Block::Definition(text) => Some(text),
        Block::Image(text, _) => Some(text),
        Block::Math(formula) => Some(&formula.text),
        Block::Code(_, _) | Block::Table(_) | Block::Rule | Block::Quote(_) => None,
        Block::Details(_, _, _) | Block::Footnote(_, _) | Block::Indented(_, _, _) => None,
    }
}

/// Every run of every block covers its text exactly, with no link pointing
/// outside it.
fn check_runs(rich: &[Rich]) {
    for block in rich {
        match &block.block {
            Block::Quote(inner) | Block::Details(_, _, inner) | Block::Footnote(_, inner) => {
                check_runs(inner);
                continue;
            }
            Block::Indented(_, _, inner) => {
                check_runs(std::slice::from_ref(inner.as_ref()));
                continue;
            }
            _ => {}
        }
        let Some(body) = body_of(&block.block) else {
            continue;
        };
        let covered: usize = block.spans.iter().map(|span| span.len).sum();
        assert_eq!(covered, body.len(), "{:?}", block.block);
        assert_eq!(covered == 0, body.is_empty(), "{:?}", block.block);
        for link in &block.links {
            assert!(link.start < link.end, "{:?}", block.block);
            assert!(link.end <= body.len(), "{:?}", block.block);
            assert!(is_openable_link(&link.url), "{}", link.url);
        }
    }
}

//! Just enough Markdown to draw an assistant answer.
//!
//! The model answers in Markdown. Showing the raw text puts fence markers and
//! `**` noise in the transcript, so a reply is split into blocks the chat view
//! can lay out — code blocks, headings, list items, quotes, tables — and the
//! inline markers become runs: `**bold**` is drawn bold, `*italic*` slanted,
//! `backticks` in the code font. Markers the app has no style for are stripped
//! instead of printed.

mod blocks;
mod highlight;
mod inline;
mod math;
mod parse;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use blocks::*;
pub(crate) use highlight::*;
pub(crate) use inline::*;
pub(crate) use math::*;
pub(crate) use parse::*;
pub(crate) use types::*;

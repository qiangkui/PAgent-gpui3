//! The 上下文 page: what each request was made of, and what it cost.
//!
//! A request is a run of model round trips, and every round sends the whole
//! prompt again with the tool results produced so far. The request's figures
//! are therefore the sum of its rounds, while each round can be opened on its
//! own to read the pieces that round actually sent.
//!
//! Everything here is measured from the same pieces the window itself sends
//! (`prompt_blocks`, the history it replays, the tool schemas the engine
//! reports), so a category's number moves when the thing it names changes
//! instead of drifting from a copy of the prompt kept for display.
//!
//! The page is filed by what each part answers: `model` holds the shapes it
//! reads, `report` derives the figures, `ring` and `timeline` build the two
//! charts, `actions` reads the tool calls and their clocks, `cards` draws the
//! panels under the list, `view` is the page itself, and `tests` pins the
//! arithmetic the rest reads.

use super::*;

mod actions;
mod cards;
mod model;
mod report;
mod ring;
mod timeline;
mod view;

#[cfg(test)]
mod tests;

pub(crate) use self::actions::*;
pub(crate) use self::cards::*;
pub(crate) use self::model::*;
pub(crate) use self::report::*;
pub(crate) use self::ring::*;
pub(crate) use self::timeline::*;

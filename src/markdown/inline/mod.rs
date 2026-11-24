//! One line of text into runs: emphasis, code spans, links, HTML tags, entities and strike-through.

use super::*;

mod core;
mod emoji;
mod emphasis;
mod links;
mod tags;

pub(crate) use core::*;
pub(crate) use emoji::*;
pub(crate) use emphasis::*;
pub(crate) use links::*;
pub(crate) use tags::*;

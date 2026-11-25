//! One bar per request along the window, with the compactions marked.

use super::*;

/// How tall the timeline's tallest bar is, in pixels.
pub(crate) const TIMELINE_BAR_HEIGHT: f32 = 92.;

/// Most bars the timeline draws even when the list shows the whole
/// conversation: several hundred two-pixel columns would run past the card.
pub(crate) const TIMELINE_BARS: usize = 120;

/// One notable thing that happened at a request, as the timeline flags it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TimelineFlag {
    /// The engine compacted the transcript before this request went out.
    Compacted,
    /// Skill prompt text entered the window here.
    Skills,
    /// The model id changed from the previous request.
    ModelChange,
    /// The biggest context jump the chart plots.
    BiggestJump,
    /// The slowest request the chart plots.
    Longest,
}

impl TimelineFlag {
    /// Every flag, in the order the legend lists them.
    pub(crate) const ALL: [Self; 5] = [
        Self::Compacted,
        Self::Skills,
        Self::ModelChange,
        Self::BiggestJump,
        Self::Longest,
    ];

    /// The name the legend shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Compacted => "压缩",
            Self::Skills => "技能",
            Self::ModelChange => "换模型",
            Self::BiggestJump => "最大跳变",
            Self::Longest => "最长",
        }
    }

    /// The flag's colour. Chart colours name an event rather than a surface, so
    /// they stay the same in both themes, like the category colours.
    pub(crate) fn color(self) -> u32 {
        match self {
            Self::Compacted => 0x00c0_392b,
            Self::Skills => 0x00a8_55f7,
            Self::ModelChange => 0x002f_6fd0,
            Self::BiggestJump => 0x0014_a3a3,
            Self::Longest => 0x00a9_6612,
        }
    }
}

/// One request as the timeline draws it.
#[derive(Clone)]
pub(crate) struct TimelinePoint {
    /// Which request it is, counting from one.
    pub(crate) turn: u64,
    /// Prompt + answer of its last round: the size the bar shows.
    pub(crate) context: u64,
    /// The bar's make-up as `(colour, share of this bar)` pairs.
    pub(crate) segments: Vec<(u32, f32)>,
    /// True when no make-up was recorded, so the bar is one neutral block.
    pub(crate) flat: bool,
    /// Flags that belong to it, in [`TimelineFlag::ALL`] order.
    pub(crate) flags: Vec<TimelineFlag>,
}

/// Turns whose answer followed a compaction note in a save that predates the
/// structured field.
///
/// The engine writes one note right before the answer it compressed for, so an
/// old transcript names the request that way. New saves carry
/// [`RequestRow::compaction_layer`], and the note is only a fallback: matching
/// prose would lose the event the moment the wording changes.
pub(crate) fn noted_compactions(messages: &[ChatMessage]) -> std::collections::HashSet<u64> {
    let mut turns = std::collections::HashSet::new();
    let mut answered = 0u64;
    let mut pending = false;
    for message in messages {
        match message.role {
            Role::Note if message.text.contains("上下文接近窗口上限") => pending = true,
            Role::Assistant => {
                answered += 1;
                if pending {
                    // Clear it either way: the note belongs to *this* answer, and
                    // a structured layer already says so. Leaving it set would
                    // flag the next answer as well.
                    if message.compaction_layer.is_empty() {
                        turns.insert(answered);
                    }
                    pending = false;
                }
            }
            _ => {}
        }
    }
    turns
}

/// The timeline: every plotted request, oldest first, with its make-up and the
/// notable things that happened at it.
///
/// `noted` carries the compaction turns read with the report, so drawing the
/// chart does not rescan the transcript on every paint. The two extremes
/// ("最大跳变", "最近") are scoped to the requests the chart actually plots, so a
/// dot never points at a bar that is off screen.
pub(crate) fn timeline_points(
    rows: &[RequestRow],
    noted: &std::collections::HashSet<u64>,
) -> Vec<TimelinePoint> {
    let ordered: Vec<&RequestRow> = rows.iter().rev().collect();
    let skills = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Skills);
    let longest = ordered
        .iter()
        .enumerate()
        .filter(|(_, row)| row.elapsed_ms > 0)
        .max_by_key(|(_, row)| row.elapsed_ms)
        .map(|(index, _)| index);
    let jump = ordered
        .iter()
        .enumerate()
        .skip(1)
        .map(|(index, row)| {
            (
                index,
                row.context.saturating_sub(ordered[index - 1].context),
            )
        })
        .max_by_key(|(_, size)| *size)
        .filter(|(_, size)| *size > 0)
        .map(|(index, _)| index);
    let mut points = Vec::with_capacity(ordered.len());
    for (index, row) in ordered.iter().enumerate() {
        // The bar shows the same closed split the request card and the totals
        // table read; a server search's pages land in 工具结果 here too.
        let total: usize = row.closed_composition.iter().sum();
        let flat = total == 0;
        let segments: Vec<(u32, f32)> = if flat {
            Vec::new()
        } else {
            ContextKind::ALL
                .iter()
                .zip(&row.closed_composition)
                .filter(|(_, tokens)| **tokens > 0)
                .map(|(kind, tokens)| (kind.color(), *tokens as f32 / total as f32))
                .collect()
        };
        let mut flags = Vec::new();
        if !row.compaction_layer.is_empty() || noted.contains(&row.turn) {
            flags.push(TimelineFlag::Compacted);
        }
        if skills.is_some_and(|slot| {
            row.closed_composition
                .get(slot)
                .is_some_and(|tokens| *tokens > 0)
        }) {
            flags.push(TimelineFlag::Skills);
        }
        if index > 0 {
            let before = ordered[index - 1];
            if !row.model.is_empty() && !before.model.is_empty() && row.model != before.model {
                flags.push(TimelineFlag::ModelChange);
            }
        }
        if jump == Some(index) {
            flags.push(TimelineFlag::BiggestJump);
        }
        if longest == Some(index) {
            flags.push(TimelineFlag::Longest);
        }
        points.push(TimelinePoint {
            turn: row.turn,
            context: row.context,
            segments,
            flat,
            flags,
        });
    }
    points
}

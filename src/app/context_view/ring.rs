//! The consumption ring: one arc per category, as a path the window fills.

use super::*;

/// Diameter of the ring that shows the whole conversation's consumption.
pub(crate) const RING_SIZE: f32 = 148.0;

/// Width of the band the ring paints.
pub(crate) const RING_THICKNESS: f32 = 18.0;

/// One arc of the ring: the outside edge out, the inside edge back, so a
/// filled path paints a slice of an annulus.
pub(crate) fn ring_path(
    center: Point<Pixels>,
    outer: f32,
    inner: f32,
    start: f32,
    sweep: f32,
) -> Option<gpui::Path> {
    let steps = ((sweep.abs() / 0.05).ceil() as usize).clamp(2, 512);
    let at = |angle: f32, radius: f32| {
        point(
            px(f32::from(center.x) + radius * angle.cos()),
            px(f32::from(center.y) + radius * angle.sin()),
        )
    };
    let mut builder = gpui::PathBuilder::fill();
    builder.move_to(at(start, outer));
    for step in 1..=steps {
        builder.line_to(at(start + sweep * step as f32 / steps as f32, outer));
    }
    for step in (0..=steps).rev() {
        builder.line_to(at(start + sweep * step as f32 / steps as f32, inner));
    }
    builder.close();
    builder.build().ok()
}

/// The whole conversation as a ring: one arc per category, sized by its share
/// of everything the endpoints billed.
///
/// `segments` are `(colour, share of the circle)`; the shares are expected to
/// add up to one. A share of zero draws nothing, so a conversation with no
/// figures is just the quiet track.
pub(crate) fn consumption_ring(
    segments: Vec<(u32, f32)>,
    track: u32,
    size: f32,
    thickness: f32,
) -> AnyElement {
    let element = gpui::canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let center = bounds.center();
            let outer =
                (f32::from(bounds.size.width).min(f32::from(bounds.size.height)) / 2.0) - 1.0;
            let inner = (outer - thickness).max(0.0);
            if let Some(path) = ring_path(center, outer, inner, 0.0, std::f32::consts::TAU) {
                window.paint_path(path, rgb(track));
            }
            // A hair of space between arcs keeps the seams readable when two
            // neighbouring categories hold similar shares.
            let gap = 0.014;
            let mut start = -std::f32::consts::FRAC_PI_2;
            for (colour, share) in &segments {
                let sweep = share * std::f32::consts::TAU - gap;
                let path = (sweep > 0.0)
                    .then(|| ring_path(center, outer, inner, start + gap / 2.0, sweep))
                    .flatten();
                if let Some(path) = path {
                    window.paint_path(path, rgb(*colour));
                }
                start += share * std::f32::consts::TAU;
            }
        },
    );
    element.w(px(size)).h(px(size)).into_any_element()
}

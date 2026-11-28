//! Small shared widgets: buttons, frames, chips, and detail rows.

use super::*;

/// A small on/off pill: the track and its knob, no label and no handler.
///
/// The row that holds it is the hit target, so this stays a drawing: the
/// composer's per-conversation menu uses it to show a setting's state, and
/// [`menu_switch_row`] wraps it with the name and the click.
pub(crate) fn switch_pill(on: bool, palette: Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .w(px(28.))
        .h(px(16.))
        .px(px(2.))
        .rounded_full()
        .bg(rgb(if on {
            palette.accent
        } else {
            palette.panel_soft
        }))
        .border_1()
        .border_color(rgb(if on { palette.accent } else { palette.border }))
        .when(on, |this| this.justify_end())
        .when(!on, |this| this.justify_start())
        .child(div().size(px(10.)).rounded_full().bg(rgb(if on {
            palette.accent_text
        } else {
            palette.muted
        })))
}

/// One per-conversation setting: its name, a line about what it does, and a
/// switch that turns it on or off.
///
/// The whole row is the hit target, so a click anywhere on it flips the switch
/// rather than only on the 28-by-16 pill.
pub(crate) fn menu_switch_row(
    id: impl Into<ElementId>,
    label: &'static str,
    note: impl Into<SharedString>,
    on: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    action: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    let note: SharedString = note.into();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |view, _, _, cx| action(view, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .gap_0p5()
                .child(div().text_xs().text_color(rgb(palette.text)).child(label))
                .when(!note.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.faint))
                            .whitespace_normal()
                            .child(note.clone()),
                    )
                }),
        )
        .child(switch_pill(on, palette))
        .into_any_element()
}

/// One choice of a per-conversation figure, laid out in a row of small chips.
pub(crate) fn menu_choice(
    id: impl Into<ElementId>,
    label: impl Into<String>,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    action: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .px_2()
        .py_1()
        .rounded_md()
        .bg(rgb(if selected {
            palette.panel_soft
        } else {
            palette.canvas
        }))
        .text_xs()
        .text_color(rgb(if selected {
            palette.accent
        } else {
            palette.muted
        }))
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_, _, _, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |view, _, _, cx| action(view, cx)))
        .child(label.into())
        .into_any_element()
}

/// A slider over a fixed list of values: a rail, a ring per notch, and one hit
/// area per notch.
///
/// Painting is one track, one fill, a ring per notch and one knob, all placed
/// from the same fraction (`notch / (notches - 1)`): the fill ends exactly where
/// the knob's centre is, and every ring sits on a notch the pointer can reach,
/// so the count of stops is readable without dragging. The knob is always drawn
/// — an unset setting parks it on the value it stands for.
///
/// Rings past the knob use the rail's own colour, so the filled half reads as
/// one piece. A disabled slider (`enabled == false`) paints everything in the
/// rail colour and takes no input: the row's value still shows, it just cannot
/// be moved (temperature while thinking is on).
///
/// Hit testing is still one area per notch rather than a bounds check, so a
/// click lands on the notch under the pointer and a drag across the rail keeps
/// setting values as it passes (each area reports its own mouse moves while the
/// button is down). The rail is inset by half a knob at each end so the knob can
/// reach 0 % and 100 % without hanging outside the control.
pub(crate) fn settings_slider(
    id: &'static str,
    notches: usize,
    active: usize,
    enabled: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    on_pick: impl Fn(&mut DshFlowApp, usize, &mut Context<DshFlowApp>) + Clone + 'static,
) -> Div {
    /// Height of the whole control, which is also the knob's room to sit in.
    const RAIL_HEIGHT: f32 = 18.;
    /// Thickness of the rail itself.
    const BAR: f32 = 4.;
    /// Diameter of the knob, gap ring included.
    const KNOB: f32 = 16.;
    /// Diameter of the coloured dot inside that ring: one pixel of background
    /// all round, which keeps the dot from blurring into the rail it sits on
    /// without the ring reading as a white wheel of its own.
    const DOT: f32 = 14.;
    /// Diameter of a notch ring: twice the rail, so the hole still shows.
    const TICK: f32 = 8.;
    /// Thickness of that ring.
    const RING: f32 = 2.;
    /// Visible length of the rail, from notch 0 to the last notch.
    const RAIL: f32 = 180.;
    /// Room for half a knob beyond each end of the rail.
    const END_PAD: f32 = KNOB / 2.;

    let last = notches.saturating_sub(1);
    let active = active.min(last);
    let frac_of = move |notch: usize| {
        if last == 0 {
            0.
        } else {
            notch as f32 / last as f32
        }
    };
    let frac = frac_of(active);
    let ink = if enabled {
        palette.accent
    } else {
        palette.border
    };
    let dot_ink = if enabled {
        palette.accent
    } else {
        palette.muted
    };

    let mut rings: Vec<AnyElement> = Vec::with_capacity(notches);
    for notch in 0..notches {
        let ring = if enabled && notch <= active {
            palette.accent
        } else {
            palette.border
        };
        rings.push(
            div()
                .absolute()
                .left(relative(frac_of(notch)))
                .top(px((BAR - TICK) / 2.))
                .ml(px(-TICK / 2.))
                .size(px(TICK))
                .rounded_full()
                .border(px(RING))
                .border_color(rgb(ring))
                .bg(rgb(palette.canvas))
                .into_any_element(),
        );
    }

    let mut hits: Vec<AnyElement> = Vec::with_capacity(notches);
    for notch in 0..notches {
        let on_click = on_pick.clone();
        let on_drag = on_pick.clone();
        let area = div().id((id, notch)).flex_1().h_full();
        let area = if enabled {
            area.cursor_pointer()
                .on_click(cx.listener(move |view, _, _, cx| on_click(view, notch, cx)))
                .on_mouse_move(cx.listener(move |view, event: &MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Left) {
                        on_drag(view, notch, cx);
                    }
                }))
        } else {
            area
        };
        hits.push(area.into_any_element());
    }

    div()
        .relative()
        .flex()
        .items_center()
        .flex_shrink_0()
        .w(px(RAIL + END_PAD * 2.))
        .h(px(RAIL_HEIGHT))
        .px(px(END_PAD))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(BAR))
                .rounded_full()
                .bg(rgb(palette.border))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .h_full()
                        .w(relative(frac))
                        .rounded_full()
                        .bg(rgb(ink)),
                )
                // Rings sit above the fill, exactly like the mock: their canvas
                // hole punches through both the rail and the filled part, which
                // is what makes the notch readable at every stop.
                .children(rings)
                .child(
                    // Centred on the fraction, then back by half the knob so the
                    // dot sits over the end of the fill rather than after it.
                    div()
                        .absolute()
                        .left(relative(frac))
                        .top(px((BAR - KNOB) / 2.))
                        .ml(px(-KNOB / 2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(KNOB))
                        .rounded_full()
                        .bg(rgb(palette.canvas))
                        .child(div().size(px(DOT)).rounded_full().bg(rgb(dot_ink))),
                )
                .child(
                    // The hit band is as tall as the whole control, not just the
                    // 4px rail, so a click is easy to land; it is still cut into
                    // one area per notch.
                    div()
                        .absolute()
                        .left_0()
                        .top(px(-(RAIL_HEIGHT - BAR) / 2.))
                        .w_full()
                        .h(px(RAIL_HEIGHT))
                        .flex()
                        .flex_row()
                        .children(hits),
                ),
        )
}

/// The "恢复默认" link beside a slider: accent and clickable while the setting
/// is its own, faint and inert once it is back at the default.
pub(crate) fn settings_default_button(
    id: impl Into<ElementId>,
    at_default: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    action: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    let link = div()
        .id(id)
        .flex_shrink_0()
        .px_2()
        .py_1()
        .rounded_md()
        .text_xs()
        .text_color(rgb(if at_default {
            palette.faint
        } else {
            palette.accent
        }))
        .child("恢复默认");
    if at_default {
        // Nothing left to restore, so no hover highlight and no click.
        return link.into_any_element();
    }
    link.cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| action(view, cx)))
        .into_any_element()
}

/// The drag handle between the sidebar and the pane.
///
/// Painted in the sidebar's own colour: the two read as one surface, and the
/// drag cursor is the only hint that it can be moved.
pub(crate) fn splitter(palette: Palette, cx: &mut Context<DshFlowApp>) -> impl IntoElement {
    div()
        .id("splitter")
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .w(px(SPLITTER_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(palette.chrome))
        .cursor(CursorStyle::ResizeLeftRight)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|view, _, _, cx| {
                view.resizing_sidebar = true;
                cx.stop_propagation();
                cx.notify();
            }),
        )
}

/// The error/warning strip under the title bar, if one is set.
///
/// Chat sets it for things like a file that could not be attached; the workflow
/// page has always shown it. Dropping it in the chat view left those failures
/// invisible, which is why this is shared rather than drawn twice.
impl DshFlowApp {
    pub(crate) fn render_banner(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let banner = self.banner.as_ref()?;
        let color = if banner.error {
            palette.failed
        } else {
            palette.warn
        };
        Some(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap_3()
                .mx_5()
                .mt_3()
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(color))
                .bg(rgb(palette.panel))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_xs()
                        .text_color(rgb(color))
                        .whitespace_normal()
                        .child(banner.text.clone()),
                )
                .child(
                    div()
                        .id("dismiss-banner")
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.banner = None;
                            cx.notify();
                        }))
                        .child("关闭"),
                )
                .into_any_element(),
        )
    }
}
///
/// Windows sizes its own caption buttons at 46x32; matching that keeps the hit
/// area where the pointer expects it.
pub(crate) fn window_button(
    area: WindowControlArea,
    glyph: &'static str,
    palette: Palette,
    hovered: bool,
) -> Div {
    let hover_background = if matches!(area, WindowControlArea::Close) {
        palette.failed
    } else {
        palette.panel_soft
    };

    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(WINDOW_BUTTON_WIDTH))
        .h_full()
        .window_control_area(area)
        // The highlight is painted from the window's own pointer state, not
        // gpui's hover: these cells are non-client for the platform, and its
        // mouse moves do not always reach the hover hit-test.
        .when(hovered, |this| this.bg(rgb(hover_background)))
        .text_color(rgb(palette.text))
        .child(div().text_size(px(19.)).child(glyph))
}

/// A 20px square icon button, used for the page actions beside a heading.
/// One small square action in a page header: a folder, a rescan, a "＋".
///
/// The button is a little larger than the glyph it carries so the target stays
/// comfortable next to a page title, and the header rows bottom-align it.
pub(crate) fn icon_button(
    id: &'static str,
    icon: &'static str,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
    run: impl Fn(&mut DshFlowApp, &mut Context<DshFlowApp>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(24.))
        .rounded_sm()
        .cursor_pointer()
        .hover(move |style| style.bg(rgb(palette.panel_soft)))
        .on_click(cx.listener(move |view, _, _, cx| run(view, cx)))
        .child(
            svg()
                .path(icon)
                .w(px(16.))
                .h(px(16.))
                .text_color(rgb(palette.muted)),
        )
        .into_any_element()
}

/// A titled block on the settings page.
///
/// Every settings page is a column of these: a bordered card on the panel
/// colour, one bold heading, an optional line of explanation, then the controls.
/// The nav on the left and the cards on the right share one rhythm, so a page of
/// two blocks and a page of five read the same way.
pub(crate) fn settings_section(title: &'static str, palette: Palette) -> Div {
    settings_card(title, palette)
}

/// A section whose explanation sits behind a "?" beside the heading.
pub(crate) fn settings_section_help(
    title: &'static str,
    explanation: impl Into<String>,
    palette: Palette,
) -> Div {
    settings_card_help(title, explanation, palette)
}

/// A card whose heading carries its explanation behind a "?".
///
/// The pages used to spell the explanation out under the heading, which pushed
/// the controls down for text read once. The icon keeps the card tight; the
/// tooltip has the words when someone wants them.
pub(crate) fn settings_card_help(
    title: &str,
    explanation: impl Into<String>,
    palette: Palette,
) -> Div {
    settings_shell(
        palette,
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(settings_heading(title.to_string(), palette))
            .child(help_icon(
                SharedString::from(format!("help-card-{title}")),
                explanation,
                palette,
            )),
    )
}

/// The card itself: the one surface every settings block sits on.
pub(crate) fn settings_card(title: &str, palette: Palette) -> Div {
    settings_shell(palette, settings_heading(title.to_string(), palette))
}

/// The card around a heading.
fn settings_shell(palette: Palette, heading: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.canvas))
        .shadow_sm()
        .p_5()
        .child(heading)
}

/// A card's bold heading.
fn settings_heading(title: String, palette: Palette) -> Div {
    div()
        .text_sm()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(palette.text))
        .child(title)
}

/// The heading of a whole page: a title, then one line about the page.
pub(crate) fn settings_page_header(title: &str, note: &str, palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(rgb(palette.text))
                .child(title.to_string()),
        )
        .child(settings_note(note, palette))
}

/// One line of explanation: the muted voice under a heading.
pub(crate) fn settings_note(note: &str, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .line_height(px(17.))
        .text_color(rgb(palette.muted))
        .whitespace_normal()
        .child(note.to_string())
        .into_any_element()
}

/// The "i" ring that holds a setting's explanation.
///
/// Settings pages no longer print the explanation beside every control; the
/// icon marks where to look and the words arrive on hover. gpui draws the
/// bubble itself as a root element of the same window, so the page's scroll
/// area cannot clip it and it flips to stay inside the window.
///
/// The glyph carries the ring, so the button draws no border and no surface of
/// its own: only the ink changes, from muted to the accent, while it is hovered.
pub(crate) fn help_icon(
    id: impl Into<ElementId>,
    explanation: impl Into<String>,
    palette: Palette,
) -> impl IntoElement {
    let explanation = explanation.into();
    div()
        .id(id)
        .group("help-chip")
        .flex()
        .items_center()
        .justify_center()
        .size(px(14.))
        .flex_shrink_0()
        .cursor_pointer()
        // The platform default is half a second; a settings hint that appears
        // faster feels like part of the row instead of a delayed surprise.
        .tooltip_show_delay(Duration::from_millis(250))
        .child(
            // A vector glyph rather than a font character: layout centres this
            // box exactly, so the ring lands in the middle of the chip whatever
            // font the system picks.
            svg()
                .path("icons/help.svg")
                .w(px(14.))
                .h(px(14.))
                .text_color(rgb(palette.faint))
                .group_hover("help-chip", move |style| {
                    style.text_color(rgb(palette.accent))
                }),
        )
        .tooltip(move |_window, cx| {
            let bubble = cx.new(|_| HelpBubble {
                explanation: explanation.clone(),
                palette,
            });
            bubble.into()
        })
}

/// The bubble a [`help_icon`] shows on hover.
///
/// It is drawn over whatever the page has underneath, so it brings its own
/// surface: the panel colour, a hairline border and a shadow, at a width that
/// keeps a paragraph readable without covering the page.
struct HelpBubble {
    explanation: String,
    palette: Palette,
}

impl Render for HelpBubble {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(280.))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgb(self.palette.border))
            .bg(rgb(self.palette.canvas))
            .shadow_sm()
            .text_xs()
            .line_height(px(18.))
            .text_color(rgb(self.palette.text))
            .whitespace_normal()
            .child(self.explanation.clone())
    }
}

/// The offset a wheel delta asks for, kept inside what is left to scroll.
///
/// Offsets run from `0` at the top to `-max` at the end, the convention the
/// scroll handles use. A delta past either end pins the offset there instead of
/// being ignored, which is what keeps a table that cannot move any further from
/// handing the wheel to the page behind it.
pub(crate) fn wheel_offset(offset: Pixels, delta: Pixels, max: Pixels) -> Pixels {
    (offset + delta).clamp(-max, px(0.))
}

/// Scroll `handle` by a wheel event that landed where nothing scrolls by itself.
///
/// A table's thin bar is a real column beside its rows, so the wheel over the
/// bar belongs to the table; without this the event would fall through to the
/// page behind the card and the panel would jump instead.
/// @returns true when the offset moved, so the caller can repaint.
pub(crate) fn wheel_scroll(handle: &ScrollHandle, event: &ScrollEvent, window: &Window) -> bool {
    let delta = event.delta.pixel_delta(window.line_height()).y;
    if delta == px(0.) {
        return false;
    }
    let current = handle.offset();
    let next = wheel_offset(current.y, delta, handle.max_offset().y);
    if next == current.y {
        return false;
    }
    handle.set_offset(point(current.x, next));
    true
}

/// Keeps a nested scroll area's wheel to itself.
///
/// gpui has already scrolled the element by the time this runs, so stopping the
/// event here only means the surface behind it — the transcript, a panel, a
/// settings page — no longer picks the same wheel up once this list reaches its
/// ends. Every scrollable piece that lives inside another scrollable surface
/// ends with this.
pub(crate) trait ContainedWheel: Sized {
    /// Stop scroll events at this element.
    fn contained_wheel(self) -> Self;
}

impl ContainedWheel for gpui::Stateful<Div> {
    fn contained_wheel(self) -> Self {
        self.on_scroll(|_, _, cx| cx.stop_propagation())
    }
}

/// Fade a surface in as it appears, lifting it by `rise` pixels.
///
/// The animation is keyed by the element's id, so a menu that is rebuilt every
/// frame still plays once when it opens, and a surface that stays open keeps its
/// last frame. gpui's `reduce_motion` setting is honoured by the element itself.
pub(crate) fn appear(element: Div, key: impl Into<ElementId>, rise: f32) -> impl IntoElement {
    element.with_animation(
        key,
        Animation::new(std::time::Duration::from_millis(140)).with_easing(gpui::ease_out_quint()),
        move |element, delta| element.opacity(delta).mt(px(rise * (1.0 - delta))),
    )
}

/// A path, a command or a file name on a soft chip, so it reads as a value.
pub(crate) fn settings_code(text: String, palette: Palette) -> AnyElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(rgb(palette.panel_soft))
        .text_xs()
        .text_color(rgb(palette.text))
        .child(text)
        .into_any_element()
}

/// How a value on a row should read: ordinary, good, a caution, or broken.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTone {
    /// Ordinary text.
    Normal,
    /// Something that works.
    Good,
    /// Something to watch.
    Warn,
    /// Something that does not work.
    Bad,
}

impl SettingsTone {
    /// The colour this tone paints.
    fn color(self, palette: Palette) -> u32 {
        match self {
            Self::Normal => palette.text,
            Self::Good => palette.ok,
            Self::Warn => palette.warn,
            Self::Bad => palette.failed,
        }
    }
}

/// One label/value row: a fixed label column, then the value.
pub(crate) fn settings_row(label: &str, value: AnyElement, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(124.))
                .flex_shrink_0()
                .text_xs()
                .line_height(px(17.))
                .text_color(rgb(palette.muted))
                .child(label.to_string()),
        )
        .child(div().flex_1().min_w(px(0.)).child(value))
        .into_any_element()
}

/// A row whose value is plain text in the given tone.
pub(crate) fn settings_text_row(
    label: &str,
    value: String,
    tone: SettingsTone,
    palette: Palette,
) -> AnyElement {
    let text = div()
        .text_xs()
        .line_height(px(17.))
        .whitespace_normal()
        .text_color(rgb(tone.color(palette)))
        .child(value)
        .into_any_element();
    settings_row(label, text, palette)
}

/// The look of a settings button; the caller adds the action.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsButton {
    /// The one action a card is about.
    Primary,
    /// Anything else on the card.
    Secondary,
    /// Removing or discarding something.
    Danger,
}

/// A settings button, sized and coloured like every other one.
pub(crate) fn settings_button(
    id: impl Into<gpui::ElementId>,
    label: &str,
    kind: SettingsButton,
    palette: Palette,
) -> gpui::Stateful<Div> {
    let (background, border, ink) = match kind {
        SettingsButton::Primary => (palette.accent, palette.accent, palette.accent_text),
        SettingsButton::Secondary => (palette.panel_soft, palette.border, palette.text),
        SettingsButton::Danger => (palette.panel_soft, palette.failed, palette.failed),
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(background))
        .text_xs()
        .text_color(rgb(ink))
        .cursor_pointer()
        .hover(move |style| style.opacity(0.92))
        .child(label.to_string())
}

/// A small pill saying whether something is working.
pub(crate) fn settings_status_pill(ok: bool, label: &str, palette: Palette) -> AnyElement {
    let colour = if ok { palette.ok } else { palette.failed };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(rgb(palette.panel_soft))
        .child(status_dot(colour))
        .child(
            div()
                .text_xs()
                .text_color(rgb(colour))
                .child(label.to_string()),
        )
        .into_any_element()
}

/// A short result line under an action: what just happened.
pub(crate) fn settings_notice(text: &str, failed: bool, palette: Palette) -> AnyElement {
    div()
        .text_xs()
        .line_height(px(17.))
        .whitespace_normal()
        .text_color(rgb(if failed { palette.failed } else { palette.ok }))
        .child(text.to_string())
        .into_any_element()
}

/// A page title with its controls just to the right of it.
///
/// The plugins, MCP and Skill pages are read as flat lists: the title is the
/// only heading, its actions follow it after a gap (not pinned to the far
/// edge), and there is no line of explanation beneath it.
/// A page's name, with its action buttons pinned to the title's bottom edge.
///
/// Bottom alignment rather than centring: the icon buttons are shorter than a
/// `text_xl` line, and sitting them on the baseline band keeps them from
/// floating above the title's left edge. The wide gap after the title keeps the
/// action cluster from crowding it; the buttons themselves stay close together.
pub(crate) fn page_title(title: &'static str, actions: Vec<AnyElement>, palette: Palette) -> Div {
    let has_actions = !actions.is_empty();
    let mut row = div()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(32.))
        .flex_shrink_0()
        .px_6()
        .py_4()
        .child(div().text_xl().text_color(rgb(palette.text)).child(title));
    if has_actions {
        row = row.child(
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap_1()
                .children(actions),
        );
    }
    row
}

/// Merge the declared plan with the engine's results, keeping plan order.
pub(crate) fn build_rows(spec: &ManifestSpec, results: &[StepView]) -> Vec<StepRow> {
    let mut rows: Vec<StepRow> = spec
        .steps
        .iter()
        .map(|step| {
            let result = results.iter().find(|view| view.id == step.id);
            StepRow {
                id: step.id.clone(),
                kind: step.kind.label().to_string(),
                description: step.description.clone(),
                // A run's own one-liner wins over the plan's command summary:
                // "共 51 条" is what a reader wants on the card.
                summary: result
                    .and_then(|view| view.summary.clone())
                    .unwrap_or_else(|| step.summary.clone()),
                plan_summary: step.summary.clone(),
                start_line: step.start_line,
                end_line: step.end_line,
                program: step.program.clone(),
                child_manifest: step.child_manifest.clone(),
                when: step.when.clone(),
                save: step.save.clone(),
                notes: step.notes.clone(),
                status: result.map_or_else(|| "pending".to_string(), |view| view.status.clone()),
                duration_ms: result.map_or(0, |view| view.duration_ms),
                stderr: result.and_then(|view| view.stderr.clone()),
                stdout: result.and_then(|view| view.stdout.clone()),
                text: result.and_then(|view| view.text.clone()),
                artifacts: result
                    .map(|view| view.artifacts.clone())
                    .unwrap_or_default(),
                error: result.and_then(|view| view.error.clone()),
            }
        })
        .collect();

    for result in results {
        if spec.steps.iter().any(|step| step.id == result.id) {
            continue;
        }
        rows.push(StepRow {
            id: result.id.clone(),
            kind: result.kind.clone(),
            summary: result.summary.clone().unwrap_or_default(),
            status: result.status.clone(),
            duration_ms: result.duration_ms,
            stderr: result.stderr.clone(),
            stdout: result.stdout.clone(),
            text: result.text.clone(),
            artifacts: result.artifacts.clone(),
            error: result.error.clone(),
            ..Default::default()
        });
    }
    rows
}

/// Label and colour for the pane's status chip.
pub(crate) fn pane_status(pane: &Pane<'_>, palette: Palette) -> (String, u32) {
    if pane.running {
        return (
            format!("运行中 {}", format_duration(pane.elapsed_ms)),
            palette.running,
        );
    }
    if pane.cancelled {
        return ("已取消".to_string(), palette.warn);
    }
    match pane.outcome {
        Some(outcome) if outcome.ok => ("成功".to_string(), palette.ok),
        Some(_) => ("失败".to_string(), palette.failed),
        None => ("计划".to_string(), palette.faint),
    }
}

/// Colour of a step status.
pub(crate) fn status_color(status: &str, palette: Palette) -> u32 {
    match status {
        "ok" => palette.ok,
        "failed" => palette.failed,
        "skipped" => palette.skipped,
        "running" => palette.running,
        "pending" | "unknown" => palette.faint,
        _ => palette.muted,
    }
}

/// A small filled circle.
pub(crate) fn status_dot(color: u32) -> Div {
    div().w(px(7.)).h(px(7.)).rounded_full().bg(rgb(color))
}

/// A pill with a dot and a label.
pub(crate) fn status_chip(label: &str, color: u32, palette: Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .rounded_full()
        .bg(rgb(palette.panel_soft))
        .child(status_dot(color))
        .child(
            div()
                .text_xs()
                .text_color(rgb(color))
                .child(label.to_string()),
        )
}

/// The colour grid under one role: six rows of hues, a row of greys, and the
/// theme's own colour.
///
/// Picking a tile applies it and closes the grid; the last tile puts the role
/// back on the theme's colour, which is the same as never having changed it.
pub(crate) fn color_grid(
    key: &'static str,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    for (row_index, row) in crate::theme::swatch_grid().into_iter().enumerate() {
        let mut tiles: Vec<AnyElement> = Vec::new();
        for (column, value) in row.into_iter().enumerate() {
            let id = row_index * 16 + column;
            tiles.push(
                div()
                    .id(("swatch", id))
                    .w(px(18.))
                    .h(px(18.))
                    .flex_shrink_0()
                    .rounded_sm()
                    .border_1()
                    .border_color(rgb(palette.border))
                    .bg(rgb(value))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.apply_color(key, value);
                        view.sync_color_fields(cx);
                        view.color_menu = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        rows.push(
            div()
                .flex()
                .flex_row()
                .gap_1()
                .children(tiles)
                .into_any_element(),
        );
    }
    rows.push(
        div()
            .id("swatch-theme")
            .mt_1()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(palette.panel))
            .text_xs()
            .text_color(rgb(palette.muted))
            .cursor_pointer()
            .child("用主题原色")
            .on_click(cx.listener(move |view, _, _, cx| {
                view.reset_color(key);
                view.sync_color_fields(cx);
                view.color_menu = None;
                cx.notify();
            }))
            .into_any_element(),
    );

    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.panel_soft))
        .children(rows)
        .into_any_element()
}

/// One palette in the theme picker: a colour preview, a name, and a badge when
/// it is the one in use.
///
/// The preview is painted with the palette the card ships, so the cards show the
/// choice rather than the current state; the card chrome follows the live theme.
pub(crate) fn theme_card(
    kind: ThemeKind,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> impl IntoElement {
    let shown = kind.palette();

    div()
        .id(kind.label())
        .flex()
        .flex_col()
        .gap_2()
        .w(px(150.))
        .p_2()
        .rounded_lg()
        .bg(rgb(palette.panel))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .cursor_pointer()
        .on_click(cx.listener(move |view, _, _, cx| {
            view.set_theme(kind, cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .h(px(56.))
                .px_3()
                .rounded_md()
                .bg(rgb(shown.background))
                .border_1()
                .border_color(rgb(shown.border))
                .child(
                    div()
                        .w(px(26.))
                        .h(px(26.))
                        .rounded_md()
                        .bg(rgb(shown.panel)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .w(px(56.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.text)),
                        )
                        .child(
                            div()
                                .w(px(38.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.muted)),
                        ),
                )
                .child(
                    div()
                        .w(px(10.))
                        .h(px(10.))
                        .rounded_full()
                        .bg(rgb(shown.accent)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.text))
                        .child(kind.label()),
                )
                .when(selected, |this| {
                    this.child(status_chip("当前", palette.accent, palette))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(kind.description()),
        )
}

/// The picker's third card: the reader's own colours.
///
/// It is a state, not a choice: the window becomes custom the moment one colour
/// differs from the theme, so the card only reports that. Its preview is the
/// live palette, which is the only thing it could honestly show.
pub(crate) fn custom_theme_card(
    shown: &Palette,
    selected: bool,
    palette: Palette,
    cx: &mut Context<DshFlowApp>,
) -> impl IntoElement {
    div()
        .id("theme-custom")
        .flex()
        .flex_col()
        .gap_2()
        .w(px(150.))
        .p_2()
        .rounded_lg()
        .bg(rgb(palette.panel))
        .border_1()
        .border_color(rgb(if selected {
            palette.accent
        } else {
            palette.border
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .h(px(56.))
                .px_3()
                .rounded_md()
                .bg(rgb(shown.background))
                .border_1()
                .border_color(rgb(shown.border))
                .child(
                    div()
                        .w(px(26.))
                        .h(px(26.))
                        .rounded_md()
                        .bg(rgb(shown.panel)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .w(px(56.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.text)),
                        )
                        .child(
                            div()
                                .w(px(38.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(shown.muted)),
                        ),
                )
                .child(
                    div()
                        .w(px(10.))
                        .h(px(10.))
                        .rounded_full()
                        .bg(rgb(shown.accent)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(if selected {
                            palette.text
                        } else {
                            palette.muted
                        }))
                        .child(crate::theme::CUSTOM_LABEL),
                )
                .when(selected, |this| {
                    this.child(status_chip("当前", palette.accent, palette))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.faint))
                .whitespace_normal()
                .child(crate::theme::CUSTOM_DESCRIPTION),
        )
        // The card reports the mode; the switches that change it are the theme
        // cards above and the colour rows below.
        .when(!selected, |this| this.cursor(CursorStyle::Arrow))
        .when(!selected, |this| this.opacity(0.75))
        .on_click(cx.listener(|_, _, _, cx| {
            // Clicking it is harmless: it only means "stay as I am".
            cx.stop_propagation();
        }))
}

/// A label/value line inside a step detail.
pub(crate) fn detail_line(label: &str, value: &str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap_2()
        .child(
            div()
                .w(px(64.))
                .flex_shrink_0()
                .text_xs()
                .text_color(rgb(palette.faint))
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_xs()
                .text_color(rgb(palette.muted))
                .whitespace_normal()
                .child(value.to_string()),
        )
        .into_any_element()
}

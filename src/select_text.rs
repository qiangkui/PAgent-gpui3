//! Text the pointer can select inside the transcript.
//!
//! gpui-pre 0.3.14 paints text through `StyledText`, which has no selection: the only
//! way to select a range is to shape the text ourselves and hit-test the pointer
//! against the shaped lines. This element does exactly that — it shapes one block
//! of answer text with the enclosing `div`'s text style, paints it, draws a
//! highlight for the selected range, and reports drags to the view, which owns the
//! selection (so `Ctrl+C` can copy it and the next frame repaints it).
//!
//! Selection is per block: a drag picks within the block it started in, which
//! keeps the model small and matches how the answers are laid out.

use std::{cell::RefCell, ops::Range, rc::Rc};

use gpui::{
    App, Background, BorderStyle, Bounds, Corners, CursorStyle, DispatchPhase, Edges, Element,
    FontStyle, FontWeight, Hitbox, HitboxBehavior, Hsla, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Size,
    StrikethroughStyle, Style, TextAlign, TextRun, TextStyle, UnderlineStyle, Window, WrappedLine,
    hsla, point, px, size,
};

use crate::markdown::{Align, Cell, Link, Span};

/// Padding either side of a table cell's text, in pixels.
const CELL_PAD: f32 = 8.;
/// Padding above and below a cell's text, so rows are not pinched between the
/// dividers.
const CELL_PAD_Y: f32 = 5.;
/// Corner radius of the table's frame, to sit with the bubble's own corners.
const FRAME_RADIUS: f32 = 6.;

/// What a drag over one block of text reports to the view.
pub type OnDrag = Rc<dyn Fn(DragEvent, &mut Window, &mut App)>;

/// The colours a table is drawn in.
#[derive(Clone, Copy)]
pub struct TableLook {
    /// Behind the header row.
    pub header: Hsla,
    /// Under every row but the last, and around the whole table.
    pub divider: Hsla,
    /// Behind a selected range.
    pub selection: Hsla,
    /// A link's label, underlined.
    pub link: Hsla,
    /// Behind a `<mark>` run.
    pub mark: Hsla,
}

/// What a drag over one block of text did, in bytes of that block's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragEvent {
    /// The pointer went down at this index.
    Start { index: usize },
    /// The pointer moved to this index with the button still held.
    Extend { index: usize },
    /// A double click: the word under the pointer, as a byte range of the block.
    Word { start: usize, end: usize },
    /// The button came up.
    End,
}

/// Shaped text plus where it ended up, shared between the element's phases.
///
/// Public only because it is the element's associated layout state.
///
/// `WrappedLine` is not `Clone` in gpui-pre 0.3.14, but paint hands the lines to
/// mouse closures that outlive the frame, so the lines are shared by `Rc`.
#[derive(Default)]
pub struct Shaped {
    lines: Vec<Rc<WrappedLine>>,
    line_height: Pixels,
    origin: Point<Pixels>,
}

/// The shared shaping state handed from layout to prepaint and paint.
pub type ShapedState = Rc<RefCell<Shaped>>;

/// The hitboxes one block of text inserts: the whole block, and each link run.
///
/// The block itself always records the text beam, even while the pointer is
/// elsewhere: the platform reads the previous frame's hitboxes when the pointer
/// moves, so a cursor recorded only on hover would arrive one move too late.
pub struct TextHitboxes {
    text: Hitbox,
    links: Vec<Hitbox>,
}

/// One block of answer text that can be dragged over and copied.
pub struct SelectableText {
    text: SharedString,
    /// Emphasis covering [`Self::text`], byte for byte: bold runs and inline
    /// code. Empty draws the whole block in the surrounding style.
    spans: Vec<Span>,
    /// The links of this text, by byte range: drawn underlined and in the link
    /// colour, opened on a click instead of starting a selection.
    links: Vec<Link>,
    /// Colour a link is drawn in.
    link_color: Hsla,
    /// Definite width the text wraps at: the answer column's inner width.
    wrap_width: Pixels,
    /// Whether to wrap at [`Self::wrap_width`]; a code block lays one line per
    /// source line and scrolls sideways instead.
    wrap: bool,
    /// Range to paint as selected, in bytes of [`Self::text`].
    highlight: Option<Range<usize>>,
    /// Colour drawn behind the selected range.
    highlight_color: Hsla,
    /// Backgrounds behind search hits, in bytes of [`Self::text`].
    marks: Vec<Range<usize>>,
    /// Colour of those backgrounds.
    marks_color: Hsla,
    /// Colours for the tokens of a code block, absent outside code.
    tokens: Option<TokenColors>,
    /// Background a `<mark>` run is drawn on; `None` draws it like any text.
    mark_color: Option<Hsla>,
    /// Called on drag; the view stores the selection.
    on_drag: OnDrag,
}

impl SelectableText {
    /// A selectable block of `text`, wrapping at `wrap_width`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        text: impl Into<SharedString>,
        spans: &[Span],
        links: &[Link],
        link_color: Hsla,
        wrap_width: Pixels,
        highlight: Option<Range<usize>>,
        highlight_color: Hsla,
        on_drag: OnDrag,
    ) -> Self {
        Self {
            text: text.into(),
            spans: spans.to_vec(),
            links: links.to_vec(),
            link_color,
            wrap_width,
            wrap: true,
            highlight,
            highlight_color,
            marks: Vec::new(),
            marks_color: highlight_color,
            tokens: None,
            mark_color: None,
            on_drag,
        }
    }

    /// Lay the text out unwrapped: one line per source line, as wide as it is.
    pub fn without_wrap(mut self) -> Self {
        self.wrap = false;
        self
    }

    /// Draw `<mark>` runs on `color`.
    pub fn with_mark(mut self, color: Hsla) -> Self {
        self.mark_color = Some(color);
        self
    }

    /// Paint search hits behind the text; ranges are bytes of the text.
    pub fn with_marks(mut self, marks: Vec<Range<usize>>, color: Hsla) -> Self {
        self.marks = marks;
        self.marks_color = color;
        self
    }

    /// Draw this block's tokens in `colors`: a code block, whose runs carry a
    /// [`crate::markdown::Token`].
    pub fn with_tokens(mut self, colors: TokenColors) -> Self {
        self.tokens = Some(colors);
        self
    }
}

/// The colours a code block gives its tokens.
///
/// Kept out of the parser: which colour a keyword gets belongs to the theme, and
/// the same parse has to draw in both themes.
#[derive(Clone, Copy)]
pub(crate) struct TokenColors {
    pub comment: Hsla,
    pub string: Hsla,
    pub number: Hsla,
    pub keyword: Hsla,
    pub function: Hsla,
    pub type_name: Hsla,
    pub added: Hsla,
    pub removed: Hsla,
}

impl TokenColors {
    /// The colour of one token kind.
    fn of(self, token: crate::markdown::Token) -> Hsla {
        match token {
            crate::markdown::Token::Comment => self.comment,
            crate::markdown::Token::String => self.string,
            crate::markdown::Token::Number => self.number,
            crate::markdown::Token::Keyword => self.keyword,
            crate::markdown::Token::Function => self.function,
            crate::markdown::Token::Type => self.type_name,
            crate::markdown::Token::Added => self.added,
            crate::markdown::Token::Removed => self.removed,
        }
    }
}

/// The text runs for one block: the surrounding style, except inside `**bold**`,
/// `*italic*`, `` `code` `` and `[links](…)`.
///
/// The spans have to cover the text exactly. A block whose spans do not — a
/// partial answer being streamed, say — falls back to one plain run, because
/// shaping reads the runs as byte ranges of the string and would otherwise read
/// past its end.
fn inline_runs(
    style: &TextStyle,
    spans: &[Span],
    links: &[Link],
    link_color: Hsla,
    len: usize,
    tokens: Option<TokenColors>,
    mark: Option<Hsla>,
) -> Vec<TextRun> {
    let covered: usize = spans.iter().map(|span| span.len).sum();
    if spans.is_empty() || covered != len {
        return vec![style.to_run(len)];
    }
    let mut start = 0;
    spans
        .iter()
        .map(|span| {
            let end = start + span.len;
            let mut run = style.clone();
            if span.code {
                run.font_family = crate::app::MONO_FONT.into();
            }
            if span.bold {
                run.font_weight = FontWeight::BOLD;
            }
            if span.italic {
                run.font_style = FontStyle::Italic;
            }
            // A token of a code block, in the colour the view gave that kind.
            if let (Some(token), Some(colors)) = (span.token, tokens) {
                run.color = colors.of(token);
            }
            // A link reads as a link, not as code: the accent colour, in the
            // same font as the text around it and with no underline.
            if links
                .iter()
                .any(|link| link.start < end && start < link.end)
            {
                run.color = link_color;
            }
            if span.strike {
                // One pixel thick, in the run's own colour so it reads like the
                // text it crosses out.
                run.strikethrough = Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(run.color),
                });
            }
            if span.underline {
                // The same hairline, under the run instead of through it.
                run.underline = Some(UnderlineStyle {
                    thickness: px(1.),
                    color: Some(run.color),
                    wavy: false,
                });
            }
            // `<mark>`: a highlighted background behind the run.
            if span.marked
                && let Some(color) = mark
            {
                run.background_color = Some(color);
            }
            start = end;
            run.to_run(span.len)
        })
        .collect()
}

impl Element for SelectableText {
    type RequestLayoutState = ShapedState;
    type PrepaintState = TextHitboxes;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The surrounding `div` has already refined the text style, so the block
        // is shaped in exactly the font, size, weight and colour it would have
        // been painted in as plain text.
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = style
            .line_height
            .to_pixels(font_size.into(), window.rem_size());

        let runs = inline_runs(
            &style,
            &self.spans,
            &self.links,
            self.link_color,
            self.text.len(),
            self.tokens,
            self.mark_color,
        );

        let state: ShapedState = Rc::new(RefCell::new(Shaped::default()));
        let shaped_state = state.clone();
        let text = self.text.clone();
        let wrap_width = if self.wrap {
            Some(self.wrap_width)
        } else {
            None
        };

        let layout_id = window.request_measured_layout(Style::default(), move |_, _, window, _| {
            let lines: Vec<Rc<WrappedLine>> = window
                .text_system()
                .shape_text(text.clone(), font_size, &runs, wrap_width, None)
                .map(|lines| lines.into_iter().map(Rc::new).collect())
                .unwrap_or_default();

            let mut measured: Size<Pixels> = Size::default();
            for line in &lines {
                let line_size = line.size(line_height);
                measured.height += line_size.height;
                measured.width = measured.width.max(line_size.width).ceil();
            }

            *shaped_state.borrow_mut() = Shaped {
                lines,
                line_height,
                origin: point(px(0.), px(0.)),
            };
            measured
        });

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        state.borrow_mut().origin = bounds.origin;
        let text = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        // Each link run gets a hitbox of its own, so the pointer can show the
        // hand over it while the rest of the block shows the text beam.
        let links = {
            let shaped = state.borrow();
            let mut boxes = Vec::new();
            for link in &self.links {
                for quad in selection_quads(
                    &self.text,
                    &shaped.lines,
                    shaped.line_height,
                    shaped.origin,
                    &(link.start..link.end),
                ) {
                    boxes.push(window.insert_hitbox(quad, HitboxBehavior::Normal));
                }
            }
            boxes
        };
        TextHitboxes { text, links }
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (lines, line_height, origin) = {
            let shaped = state.borrow();
            (shaped.lines.clone(), shaped.line_height, shaped.origin)
        };
        if lines.is_empty() {
            return;
        }

        if !self.marks.is_empty() {
            for range in &self.marks {
                if range.is_empty() {
                    continue;
                }
                for quad in selection_quads(&self.text, &lines, line_height, origin, range) {
                    window.paint_quad(PaintQuad {
                        bounds: quad,
                        corner_radii: Corners::default(),
                        background: Background::from(self.marks_color),
                        border_widths: Edges::default(),
                        border_color: hsla(0., 0., 0., 0.),
                        border_style: BorderStyle::default(),
                    });
                }
            }
        }

        if let Some(range) = self.highlight.clone()
            && !range.is_empty()
        {
            for quad in selection_quads(&self.text, &lines, line_height, origin, &range) {
                window.paint_quad(PaintQuad {
                    bounds: quad,
                    corner_radii: Corners::default(),
                    background: Background::from(self.highlight_color),
                    border_widths: Edges::default(),
                    border_color: hsla(0., 0., 0., 0.),
                    border_style: BorderStyle::default(),
                });
            }
        }

        let mut line_origin = origin;
        for line in &lines {
            let _ = line.paint(
                line_origin,
                line_height,
                TextAlign::Left,
                Some(bounds),
                window,
                cx,
            );
            line_origin.y += line.size(line_height).height;
        }

        // The text beam is recorded whatever the pointer is doing, so moving the
        // pointer onto a block changes the cursor without a repaint first. Each
        // link's own hitbox then wins over it, showing the hand.
        window.set_cursor_style(CursorStyle::IBeam, &hitbox.text);
        for link in &hitbox.links {
            window.set_cursor_style(CursorStyle::PointingHand, link);
        }

        let on_drag = self.on_drag.clone();
        let down_drag = on_drag.clone();
        let down_hitbox = hitbox.text.clone();
        let down_lines = lines.clone();
        let down_text = self.text.clone();
        let down_links = self.links.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
                return;
            }
            let index = index_for_position(&down_lines, line_height, origin, event.position);
            // A press on a link opens it and leaves the selection alone: the
            // address is the only thing the label is for. The opener is checked
            // and started without a shell; a failure just does nothing, since
            // the address is still in the answer.
            if let Some(link) = down_links.iter().find(|link| link.contains(index)) {
                let _ = crate::app::open_url(&link.url);
                return;
            }
            down_drag(
                click_event(&down_text, index, event.click_count),
                window,
                cx,
            );
        });

        let move_drag = on_drag.clone();
        let move_hitbox = hitbox.text.clone();
        let move_lines = lines.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.pressed_button != Some(MouseButton::Left) {
                return;
            }
            // Only the block the pointer is over reports, so a drag that crosses
            // several blocks hands each of them its own index.
            if !move_hitbox.is_hovered(window) {
                return;
            }
            let index = index_for_position(&move_lines, line_height, origin, event.position);
            move_drag(DragEvent::Extend { index }, window, cx);
        });

        let up_drag = on_drag;
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            up_drag(DragEvent::End, window, cx);
        });
    }
}

/// A Markdown table the pointer can drag over, drawn as a grid.
///
/// Cells are shaped on their own and painted at fixed column offsets: aligning
/// columns with spaces would need a monospaced font, and a Chinese glyph is not
/// one Latin advance wide. Selection still works in bytes of a single string —
/// the table's text — so dragging and `Ctrl+C` behave like every other block.
pub struct SelectableTable {
    /// Header cells first, then the body rows.
    rows: Vec<Vec<Cell>>,
    /// The same cells as one string: what a selection is measured in.
    text: SharedString,
    /// Definite width the table is squeezed into.
    wrap_width: Pixels,
    /// Range to paint as selected, in bytes of [`Self::text`].
    highlight: Option<Range<usize>>,
    /// How each column lines up.
    aligns: Vec<Align>,
    /// Colours the header, the dividers and the selection.
    look: TableLook,
    /// The links of the table's text, in the same bytes a selection uses.
    links: Vec<Link>,
    /// Called on drag; the view stores the selection.
    on_drag: OnDrag,
}

impl SelectableTable {
    /// A table of `rows` (header first) whose cells read back as `text`.
    pub fn new(
        rows: Vec<Vec<Cell>>,
        text: impl Into<SharedString>,
        wrap_width: Pixels,
        highlight: Option<Range<usize>>,
        aligns: Vec<Align>,
        look: TableLook,
        on_drag: OnDrag,
    ) -> Self {
        let links = table_links(&rows);
        Self {
            rows,
            text: text.into(),
            wrap_width,
            highlight,
            aligns,
            look,
            links,
            on_drag,
        }
    }
}

/// Every link of the table, moved into the bytes of the table's whole text.
fn table_links(rows: &[Vec<Cell>]) -> Vec<Link> {
    let lengths: Vec<Vec<usize>> = rows
        .iter()
        .map(|row| row.iter().map(|cell| cell.text.len()).collect())
        .collect();
    let starts = cell_offsets(&lengths);
    let mut links = Vec::new();
    for (row, cells) in rows.iter().enumerate() {
        for (column, cell) in cells.iter().enumerate() {
            let base = starts[row][column];
            for link in &cell.links {
                links.push(Link {
                    start: base + link.start,
                    end: base + link.end,
                    url: link.url.clone(),
                });
            }
        }
    }
    links
}

/// A table measured into columns and rows, shared between the element's phases.
#[derive(Clone, Default)]
pub struct TableShaped {
    /// Width of each column, cell padding included.
    columns: Vec<Pixels>,
    /// Left edge of each column, from the table's origin.
    column_offsets: Vec<Pixels>,
    /// Top edge of each row, from the table's origin.
    row_offsets: Vec<Pixels>,
    row_heights: Vec<Pixels>,
    /// `cells[row][column]`, each shaped at its column's width.
    ///
    /// The table is cloned into its mouse handlers, so the line below shares the
    /// shaped `WrappedLine`s through `Rc` (they are not `Clone` in gpui-pre 0.3.14).
    cells: Vec<Vec<Vec<Rc<WrappedLine>>>>,
    /// Where each cell's text starts, right of the column's left edge: zero for
    /// a left-aligned column, the free space for a right-aligned one.
    cell_shifts: Vec<Vec<Pixels>>,
    /// Byte offset of each cell inside the table's text.
    cell_starts: Vec<Vec<usize>>,
    line_height: Pixels,
    origin: Point<Pixels>,
    len: usize,
}

/// The shaping state handed from layout to prepaint and paint.
pub type TableState = Rc<RefCell<TableShaped>>;

impl Element for SelectableTable {
    type RequestLayoutState = TableState;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        _cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // The surrounding `div` has already refined the text style, so the cells
        // are shaped in exactly the font and size their sibling blocks use.
        let font = TableFont::new(window);
        let rows = self.rows.clone();
        let wrap_width = self.wrap_width;
        let len = self.text.len();
        let aligns = self.aligns.clone();
        let link = self.look.link;
        let mark = self.look.mark;

        let state: TableState = Rc::new(RefCell::new(TableShaped::default()));
        let shaped_state = state.clone();

        let layout_id = window.request_measured_layout(Style::default(), move |_, _, window, _| {
            let laid_out = shape_table(&rows, &font, wrap_width, len, &aligns, link, mark, window);
            let measured = size(
                laid_out
                    .columns
                    .iter()
                    .copied()
                    .fold(px(0.), |total, width| total + width),
                laid_out
                    .row_heights
                    .iter()
                    .copied()
                    .fold(px(0.), |total, height| total + height),
            );
            *shaped_state.borrow_mut() = laid_out;
            measured
        });

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let mut shaped = state.borrow_mut();
        shaped.origin = bounds.origin;
        // Only the drawn frame answers the pointer: a table narrower than the
        // bubble must not take drags in the empty space beside it.
        let width = shaped
            .columns
            .iter()
            .copied()
            .fold(px(0.), |total, column| total + column);
        let height = shaped
            .row_heights
            .iter()
            .copied()
            .fold(px(0.), |total, height| total + height);
        window.insert_hitbox(
            Bounds::new(bounds.origin, size(width, height)),
            HitboxBehavior::Normal,
        )
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let shaped: Rc<TableShaped> = Rc::new(state.borrow().clone());
        let (columns, column_offsets, row_offsets, row_heights) = (
            &shaped.columns,
            &shaped.column_offsets,
            &shaped.row_offsets,
            &shaped.row_heights,
        );
        let origin = shaped.origin;
        let line_height = shaped.line_height;
        let width = columns
            .iter()
            .copied()
            .fold(px(0.), |total, column| total + column);
        let height = row_heights
            .iter()
            .copied()
            .fold(px(0.), |total, height| total + height);

        if let Some(&header_height) = row_heights.first() {
            // The band shares the frame's top corners, and keeps its bottom ones
            // when it is the whole table.
            let radii = if row_heights.len() == 1 {
                Corners::all(px(FRAME_RADIUS))
            } else {
                Corners {
                    top_left: px(FRAME_RADIUS),
                    top_right: px(FRAME_RADIUS),
                    bottom_right: px(0.),
                    bottom_left: px(0.),
                }
            };
            window.paint_quad(PaintQuad {
                bounds: Bounds::new(origin, size(width, header_height)),
                corner_radii: radii,
                background: Background::from(self.look.header),
                border_widths: Edges::default(),
                border_color: hsla(0., 0., 0., 0.),
                border_style: BorderStyle::default(),
            });
        }
        for offset in row_offsets.iter().skip(1) {
            let y = origin.y + *offset;
            window.paint_quad(filled(
                Bounds::new(point(origin.x, y), size(width, px(1.))),
                self.look.divider,
            ));
        }
        // The frame is drawn after the header band so the band cannot swallow it.
        window.paint_quad(PaintQuad {
            bounds: Bounds::new(origin, size(width, height)),
            corner_radii: Corners::all(px(FRAME_RADIUS)),
            background: Background::from(hsla(0., 0., 0., 0.)),
            border_widths: Edges::all(px(1.)),
            border_color: self.look.divider,
            border_style: BorderStyle::default(),
        });

        // A cell's inline code is shaped in the code font with no fill of its
        // own; the highlight below is the only thing painted behind text.
        let highlight = self.highlight.clone().filter(|range| !range.is_empty());
        if let Some(range) = highlight {
            for (row, cells) in shaped.cells.iter().enumerate() {
                for (column, lines) in cells.iter().enumerate() {
                    let start = shaped.cell_starts[row][column];
                    let end = start + self.rows[row][column].text.len();
                    let from = range.start.max(start);
                    let to = range.end.min(end);
                    if from >= to {
                        continue;
                    }
                    let cell_origin = point(
                        origin.x
                            + column_offsets[column]
                            + px(CELL_PAD)
                            + shaped.cell_shifts[row][column],
                        origin.y + row_offsets[row] + px(CELL_PAD_Y),
                    );
                    let text = self.rows[row][column].text.as_str();
                    let local = from - start..to - start;
                    for area in selection_quads(text, lines, line_height, cell_origin, &local) {
                        window.paint_quad(filled(area, self.look.selection));
                    }
                }
            }
        }

        for (row, cells) in shaped.cells.iter().enumerate() {
            for (column, lines) in cells.iter().enumerate() {
                let mut line_origin = cell_origin(&shaped, origin, row, column);
                for line in lines {
                    let _ = line.paint(
                        line_origin,
                        line_height,
                        TextAlign::Left,
                        Some(bounds),
                        window,
                        cx,
                    );
                    line_origin.y += line.size(line_height).height;
                }
            }
        }

        // Recorded whatever the pointer is doing, so moving onto the table shows
        // the text beam without waiting for a repaint.
        window.set_cursor_style(CursorStyle::IBeam, hitbox);

        let on_drag = self.on_drag.clone();
        let down_drag = on_drag.clone();
        let down_hitbox = hitbox.clone();
        let down_shape = shaped.clone();
        let down_text = self.text.clone();
        let down_links = self.links.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !down_hitbox.is_hovered(window) {
                return;
            }
            let index = index_for_table(&down_shape, event.position);
            if let Some(link) = down_links.iter().find(|link| link.contains(index)) {
                let _ = crate::app::open_url(&link.url);
                return;
            }
            down_drag(
                click_event(&down_text, index, event.click_count),
                window,
                cx,
            );
        });

        let move_drag = on_drag.clone();
        let move_hitbox = hitbox.clone();
        let move_shape = shaped.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if !move_hitbox.is_hovered(window) {
                return;
            }
            let index = index_for_table(&move_shape, event.position);
            move_drag(DragEvent::Extend { index }, window, cx);
        });

        let up_drag = on_drag;
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            up_drag(DragEvent::End, window, cx);
        });
    }
}

/// A quad of one flat colour, used for the header band, the dividers and the
/// selection highlight.
fn filled(bounds: Bounds<Pixels>, color: Hsla) -> PaintQuad {
    PaintQuad {
        bounds,
        corner_radii: Corners::default(),
        background: Background::from(color),
        border_widths: Edges::default(),
        border_color: hsla(0., 0., 0., 0.),
        border_style: BorderStyle::default(),
    }
}

/// The text style a table is shaped in, plus the metrics taken from it.
#[derive(Clone)]
struct TableFont {
    style: TextStyle,
    /// The same style with bold weight, for headers and `**bold**` runs.
    bold: TextStyle,
    /// The code style, for `` `inline code` `` runs.
    code: TextStyle,
    size: Pixels,
    line_height: Pixels,
}

impl TableFont {
    /// Read the style the enclosing `div` refined, the way every block does.
    fn new(window: &mut Window) -> Self {
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let line_height = style.line_height.to_pixels(size.into(), window.rem_size());
        let mut bold = style.clone();
        bold.font_weight = FontWeight::BOLD;
        let mut code = style.clone();
        code.font_family = crate::app::MONO_FONT.into();
        Self {
            style,
            bold,
            code,
            size,
            line_height,
        }
    }
}

/// Measure a table: shape every cell, size the columns, then squeeze them into
/// `wrap_width`.
fn shape_table(
    rows: &[Vec<Cell>],
    font: &TableFont,
    wrap_width: Pixels,
    len: usize,
    aligns: &[Align],
    link: Hsla,
    mark: Hsla,
    window: &mut Window,
) -> TableShaped {
    let line_height = font.line_height;
    let column_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut natural = vec![0f32; column_count];

    // First pass: every cell unwrapped, so a column can be as wide as its widest
    // cell. The shaping is reused unless the column ends up narrower than that.
    let mut measured: Vec<Vec<Vec<Rc<WrappedLine>>>> = Vec::with_capacity(rows.len());
    for (row, cells) in rows.iter().enumerate() {
        let mut shaped_row = Vec::with_capacity(cells.len());
        for (column, cell) in cells.iter().enumerate() {
            let lines = shape_cell(window, cell, font, row == 0, None, link, mark);
            let width = lines.iter().fold(0f32, |widest, line| {
                widest.max(f32::from(line.size(line_height).width))
            });
            natural[column] = natural[column].max(width);
            shaped_row.push(lines.into_iter().map(Rc::new).collect());
        }
        measured.push(shaped_row);
    }

    // Padding is part of a column's width. When the table cannot fit, the widest
    // columns give way first, so a narrow column keeps its natural size — but
    // only while every column can still be read. When the bubble is too narrow
    // for that, the table keeps its natural width and the view scrolls it
    // sideways, the way a code block does.
    let mut widths: Vec<f32> = natural.iter().map(|width| width + 2. * CELL_PAD).collect();
    let readable = f32::from(wrap_width) / column_count.max(1) as f32 >= MIN_COLUMN;
    if widths.iter().sum::<f32>() > f32::from(wrap_width) && readable {
        fit_columns(&mut widths, f32::from(wrap_width));
    }

    let columns: Vec<Pixels> = widths.iter().map(|width| px(*width)).collect();
    let mut column_offsets = Vec::with_capacity(columns.len());
    let mut offset = px(0.);
    for column in &columns {
        column_offsets.push(offset);
        offset += *column;
    }

    let mut cells: Vec<Vec<Vec<Rc<WrappedLine>>>> = Vec::with_capacity(rows.len());
    let mut cell_shifts: Vec<Vec<Pixels>> = Vec::with_capacity(rows.len());
    let mut row_heights = Vec::with_capacity(rows.len());
    for (row, cells_in_row) in rows.iter().enumerate() {
        let mut shaped_row = Vec::with_capacity(cells_in_row.len());
        let mut shifts = Vec::with_capacity(cells_in_row.len());
        let mut height = px(0.);
        for (column, cell) in cells_in_row.iter().enumerate() {
            let inner = (columns[column] - px(2. * CELL_PAD)).max(px(4.));
            let mut lines = std::mem::take(&mut measured[row][column]);
            let own_width = lines.iter().fold(0f32, |widest, line| {
                widest.max(f32::from(line.size(line_height).width))
            });
            // Alignment is a shift of the cell's text inside its column, so it
            // costs nothing to hit-test: the drag math uses the same offset.
            let align = aligns.get(column).copied().unwrap_or_default();
            shifts.push(cell_shift(align, f32::from(inner) - own_width));
            if natural[column] > f32::from(inner) {
                lines = shape_cell(window, cell, font, row == 0, Some(inner), link, mark)
                    .into_iter()
                    .map(Rc::new)
                    .collect();
            }
            let cell_height = lines
                .iter()
                .fold(px(0.), |total, line| total + line.size(line_height).height);
            height = height.max(cell_height);
            shaped_row.push(lines);
        }
        // An empty cell still owns a line, so its row does not collapse.
        row_heights.push(height.max(line_height) + px(2. * CELL_PAD_Y));
        cells.push(shaped_row);
        cell_shifts.push(shifts);
    }

    let mut row_offsets = Vec::with_capacity(row_heights.len());
    let mut offset = px(0.);
    for height in &row_heights {
        row_offsets.push(offset);
        offset += *height;
    }

    let mut lengths: Vec<Vec<usize>> = Vec::with_capacity(rows.len());
    for cells_in_row in rows {
        lengths.push(cells_in_row.iter().map(|cell| cell.text.len()).collect());
    }
    let cell_starts = cell_offsets(&lengths);

    TableShaped {
        columns,
        column_offsets,
        row_offsets,
        row_heights,
        cells,
        cell_shifts,
        cell_starts,
        line_height,
        origin: point(px(0.), px(0.)),
        len,
    }
}

/// Where a cell's text starts: its column's left edge and its row's top, plus
/// the cell padding and whatever shift its column's alignment asks for.
fn cell_origin(
    shaped: &TableShaped,
    origin: Point<Pixels>,
    row: usize,
    column: usize,
) -> Point<Pixels> {
    point(
        origin.x + shaped.column_offsets[column] + px(CELL_PAD) + shaped.cell_shifts[row][column],
        origin.y + shaped.row_offsets[row] + px(CELL_PAD_Y),
    )
}

/// How far a cell's text moves inside its column to line up.
///
/// `free` is the column's inner width minus the text's own width: a left-aligned
/// column leaves it at the right, a right-aligned one has to step over it, and a
/// centred one splits it. A cell that had to wrap has no free space, so it
/// starts at the column's left edge whatever the column says.
fn cell_shift(align: Align, free: f32) -> Pixels {
    if free <= 0. {
        return px(0.);
    }
    match align {
        Align::Left => px(0.),
        Align::Center => px(free / 2.),
        Align::Right => px(free),
    }
}

/// Narrowest a squeezed column may become: its padding plus room for a few
/// characters. Below this a cell would break almost every character onto its own
/// line.
const MIN_COLUMN: f32 = 2. * CELL_PAD + 24.;

/// Squeeze column widths into `available`, cutting the widest columns.
///
/// The columns that fit inside an equal share of the width keep their natural
/// size and the rest split what is left, so a long cell wraps while a short one
/// next to it stays as wide as its text. No column is left narrower than
/// [`MIN_COLUMN`] when there is room, and what that costs comes off the widest
/// columns.
fn fit_columns(widths: &mut [f32], available: f32) {
    let total: f32 = widths.iter().sum();
    if total <= available || widths.is_empty() {
        return;
    }

    let mut sorted = widths.to_vec();
    sorted.sort_by(f32::total_cmp);
    let count = sorted.len();
    let mut level = available / count as f32;
    let mut prefix = 0f32;
    for (index, width) in sorted.iter().enumerate() {
        let share = (available - prefix) / (count - index) as f32;
        if *width > share {
            level = share;
            break;
        }
        prefix += *width;
        level = share;
    }

    let level = level.max(0.);
    for width in widths.iter_mut() {
        *width = width.min(level);
    }

    // Lift any column the cut left unreadably narrow, taking the difference from
    // the widest columns that still have room above the floor.
    let floor = MIN_COLUMN.min(available / count as f32);
    let mut deficit = 0f32;
    for width in widths.iter_mut() {
        if *width < floor {
            deficit += floor - *width;
            *width = floor;
        }
    }
    while deficit > 0.01 {
        let Some((index, widest)) = widths
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| f32::total_cmp(left, right))
            .map(|(index, width)| (index, *width))
        else {
            break;
        };
        if widest <= floor + 0.01 {
            break;
        }
        let take = (widest - floor).min(deficit);
        widths[index] -= take;
        deficit -= take;
    }
}

/// Byte offset of every cell's first byte inside the table's text.
///
/// The text joins cells with `" | "` and rows with `\n` — the same spelling
/// [`crate::markdown::Table::text`] produces, which is what a selection and the
/// clipboard see.
fn cell_offsets(lengths: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut starts = Vec::with_capacity(lengths.len());
    let mut start = 0usize;
    for row in lengths {
        let mut cell_start = start;
        let mut offsets = Vec::with_capacity(row.len());
        for len in row {
            offsets.push(cell_start);
            cell_start += len + " | ".len();
        }
        let row_len = row.iter().sum::<usize>() + " | ".len() * row.len().saturating_sub(1);
        starts.push(offsets);
        start += row_len + 1;
    }
    starts
}

/// Shape one cell: the header row and a cell's own `**bold**` runs come out
/// bold, its `` `code` `` runs in the code font, and a link's label underlined
/// in the link colour.
fn shape_cell(
    window: &mut Window,
    cell: &Cell,
    font: &TableFont,
    header: bool,
    wrap_width: Option<Pixels>,
    link: Hsla,
    mark: Hsla,
) -> Vec<WrappedLine> {
    let text = SharedString::new(cell.text.as_str());
    let runs: Vec<TextRun> = if cell.spans.is_empty() {
        vec![text_run(
            font, header, false, false, None, false, false, false, mark, 0,
        )]
    } else {
        let mut start = 0;
        cell.spans
            .iter()
            .map(|span| {
                let end = start + span.len;
                let linked = cell
                    .links
                    .iter()
                    .any(|link| link.start < end && start < link.end);
                start = end;
                text_run(
                    font,
                    header || span.bold,
                    span.code,
                    span.italic,
                    linked.then_some(link),
                    span.strike,
                    span.underline,
                    span.marked,
                    mark,
                    span.len,
                )
            })
            .collect()
    };
    window
        .text_system()
        .shape_text(text, font.size, &runs, wrap_width, None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default()
}

/// One styled run of a cell, `len` bytes long.
///
/// `link` is the accent colour a link's label is drawn in, `None` for ordinary
/// text.
fn text_run(
    font: &TableFont,
    bold: bool,
    code: bool,
    italic: bool,
    link: Option<Hsla>,
    strike: bool,
    underline: bool,
    marked: bool,
    mark: Hsla,
    len: usize,
) -> TextRun {
    let mut style = if code {
        font.code.clone()
    } else if bold {
        font.bold.clone()
    } else {
        font.style.clone()
    };
    if bold && code {
        style.font_weight = FontWeight::BOLD;
    }
    if italic {
        style.font_style = FontStyle::Italic;
    }
    if let Some(color) = link {
        // The same look as a link in the answer: accent colour, same font, no
        // underline.
        style.color = color;
    }
    if strike {
        style.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(style.color),
        });
    }
    if underline {
        style.underline = Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(style.color),
            wavy: false,
        });
    }
    if marked {
        style.background_color = Some(mark);
    }
    style.to_run(len)
}

/// What a mouse-down on `index` reports: the word under the pointer for a double
/// click, a caret otherwise.
///
/// The composer's field does the same with [`crate::text_input::word_range`], so
/// a double click picks a word the same way in both places.
fn click_event(text: &str, index: usize, click_count: usize) -> DragEvent {
    if click_count < 2 {
        return DragEvent::Start { index };
    }
    let word = crate::text_input::word_range(text, index);
    DragEvent::Word {
        start: word.start,
        end: word.end,
    }
}

/// The byte index of a table's text closest to `position`, clamped to its ends.
fn index_for_table(shape: &TableShaped, position: Point<Pixels>) -> usize {
    let origin = shape.origin;
    if shape.cells.is_empty() || position.y < origin.y {
        return 0;
    }

    let last_row = shape.cells.len() - 1;
    let mut row = last_row;
    for candidate in 0..=last_row {
        let bottom = origin.y + shape.row_offsets[candidate] + shape.row_heights[candidate];
        if position.y <= bottom {
            row = candidate;
            break;
        }
    }

    let cells = &shape.cells[row];
    if cells.is_empty() {
        return shape.len;
    }
    let mut column = cells.len() - 1;
    for candidate in 0..cells.len() {
        let right = origin.x + shape.column_offsets[candidate] + shape.columns[candidate];
        if position.x <= right {
            column = candidate;
            break;
        }
    }

    let cell_origin = point(
        origin.x + shape.column_offsets[column] + px(CELL_PAD) + shape.cell_shifts[row][column],
        origin.y + shape.row_offsets[row] + px(CELL_PAD_Y),
    );
    let index = index_for_position(&cells[column], shape.line_height, cell_origin, position);
    (shape.cell_starts[row][column] + index).min(shape.len)
}

/// The byte index closest to `position`, clamped to the text's ends.
fn index_for_position(
    lines: &[Rc<WrappedLine>],
    line_height: Pixels,
    origin: Point<Pixels>,
    position: Point<Pixels>,
) -> usize {
    if position.y < origin.y {
        return 0;
    }

    let mut line_origin = origin;
    let mut line_start = 0;
    for line in lines {
        let line_bottom = line_origin.y + line.size(line_height).height;
        if position.y > line_bottom {
            line_origin.y = line_bottom;
            line_start += line.len() + 1;
            continue;
        }
        let within = point(position.x - line_origin.x, position.y - line_origin.y);
        let index = line
            .index_for_position(within, line_height)
            .unwrap_or_else(|index| index);
        return line_start + index;
    }
    line_start.saturating_sub(1)
}

/// One rectangle per visual row of the selected range.
///
/// Row breaks are found by watching the x coordinate fall back: `position_for_index`
/// reports the index *at* a wrap boundary as the previous row's last caret
/// position, so the y coordinate alone cannot tell where a row begins. The x
/// folding back means the previous character boundary is the row's first index.
fn selection_quads(
    text: &str,
    lines: &[Rc<WrappedLine>],
    line_height: Pixels,
    origin: Point<Pixels>,
    range: &Range<usize>,
) -> Vec<Bounds<Pixels>> {
    let mut quads = Vec::new();
    let mut line_origin = origin;
    let mut line_start = 0;

    for line in lines {
        let line_end = line_start + line.len();
        let from = range.start.clamp(line_start, line_end) - line_start;
        let to = range.end.clamp(line_start, line_end) - line_start;

        if from < to {
            let line_text = text.get(line_start..line_end).unwrap_or_default();

            // Where each visual row starts, as offsets within the line.
            let mut rows = vec![0];
            let mut boundaries: Vec<(usize, Pixels)> = Vec::new();
            let mut index = 0;
            while index < line.len() {
                if let Some(position) = line.position_for_index(index, line_height) {
                    boundaries.push((index, position.x));
                }
                index = next_boundary(line_text, index).max(index + 1);
            }
            if let Some(position) = line.position_for_index(line.len(), line_height) {
                boundaries.push((line.len(), position.x));
            }
            let mut previous = 0;
            let mut previous_x = px(0.);
            for &(index, x) in &boundaries {
                if x < previous_x {
                    rows.push(previous);
                }
                previous_x = x;
                previous = index;
            }

            for (row, row_start) in rows.iter().copied().enumerate() {
                let row_end = rows.get(row + 1).copied().unwrap_or(line.len());
                let left = from.max(row_start);
                let right = to.min(row_end);
                if left >= right {
                    continue;
                }
                let x = if left == row_start {
                    px(0.)
                } else {
                    line.position_for_index(left, line_height)
                        .map(|position| position.x)
                        .unwrap_or(px(0.))
                };
                let x_end = line
                    .position_for_index(right, line_height)
                    .map(|position| position.x)
                    .unwrap_or(x);
                quads.push(Bounds::new(
                    point(line_origin.x + x, line_origin.y + line_height * row as f32),
                    size((x_end - x).max(px(2.)), line_height),
                ));
            }
        }

        line_origin.y += line.size(line_height).height;
        line_start = line_end + 1;
    }

    quads
}

/// The start of the character after `index`, or the text length.
fn next_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    text[index..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(offset, _)| index + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_boundary_walks_characters() {
        assert_eq!(next_boundary("abc", 0), 1);
        assert_eq!(next_boundary("abc", 2), 3);
        assert_eq!(next_boundary("abc", 3), 3);
    }

    #[test]
    fn next_boundary_stays_on_char_boundaries() {
        // Three bytes per character: stepping by bytes would split them.
        assert_eq!(next_boundary("中文", 0), 3);
        assert_eq!(next_boundary("中文", 3), 6);
        assert_eq!(next_boundary("中文", 6), 6);
    }

    #[test]
    fn next_boundary_clamps_past_the_end() {
        assert_eq!(next_boundary("", 0), 0);
        assert_eq!(next_boundary("a", 5), 1);
    }

    #[test]
    fn the_widest_column_takes_the_squeeze() {
        let mut widths = vec![40., 40., 600.];
        fit_columns(&mut widths, 400.);
        assert_eq!(widths, vec![40., 40., 320.]);
    }

    #[test]
    fn a_table_that_already_fits_keeps_its_columns() {
        let mut widths = vec![60., 80.];
        fit_columns(&mut widths, 400.);
        assert_eq!(widths, vec![60., 80.]);
    }

    #[test]
    fn every_column_shares_when_all_are_too_wide() {
        let mut widths = vec![300., 300.];
        fit_columns(&mut widths, 400.);
        assert_eq!(widths, vec![200., 200.]);
    }

    #[test]
    fn a_squeezed_column_keeps_a_readable_minimum() {
        // The narrow column is lifted to `MIN_COLUMN`, and the wide one gives up
        // exactly that much so the table still fits.
        let mut widths = vec![10., 600.];
        fit_columns(&mut widths, 400.);
        assert_eq!(widths[0], MIN_COLUMN);
        assert!((widths.iter().sum::<f32>() - 400.).abs() < 0.01);

        // When even the floor cannot fit, every column shares what there is.
        let mut widths = vec![10., 600.];
        fit_columns(&mut widths, 40.);
        assert!((widths[0] - 20.).abs() < 0.01);
        assert!((widths[1] - 20.).abs() < 0.01);
    }

    #[test]
    fn cell_offsets_follow_the_table_text() {
        assert_eq!(
            cell_offsets(&[vec![1, 2], vec![3, 4]]),
            vec![vec![0, 4], vec![7, 13]]
        );
    }

    #[test]
    fn cell_offsets_match_the_markdown_text() {
        use crate::markdown::Cell;
        let table = crate::markdown::Table {
            header: vec![Cell::from("a"), Cell::from("bb")],
            rows: vec![vec![Cell::from("ccc"), Cell::from("dddd")]],
            aligns: vec![Align::Left, Align::Right],
        };
        let lengths = vec![vec![1, 2], vec![3, 4]];
        let text = table.text();
        for (row, starts) in cell_offsets(&lengths).iter().enumerate() {
            let cells = if row == 0 {
                &table.header
            } else {
                &table.rows[row - 1]
            };
            for (column, start) in starts.iter().enumerate() {
                let cell = &cells[column].text;
                assert_eq!(&text[*start..*start + cell.len()], cell);
            }
        }
    }

    #[test]
    fn a_right_aligned_cell_steps_over_its_free_space() {
        assert_eq!(cell_shift(Align::Left, 40.), px(0.));
        assert_eq!(cell_shift(Align::Right, 40.), px(40.));
        assert_eq!(cell_shift(Align::Center, 40.), px(20.));
    }

    #[test]
    fn a_cell_with_no_free_space_stays_at_the_left_edge() {
        assert_eq!(cell_shift(Align::Right, 0.), px(0.));
        assert_eq!(cell_shift(Align::Center, -5.), px(0.));
    }

    #[test]
    fn a_double_click_reports_the_word_under_the_pointer() {
        assert_eq!(
            click_event("用 read 工具", 5, 1),
            DragEvent::Start { index: 5 }
        );
        assert_eq!(
            click_event("用 read 工具", 5, 2),
            DragEvent::Word { start: 4, end: 8 }
        );
        // Punctuation is a run of its own, so a double click on it picks the
        // comma and the space beside it, not the word.
        assert_eq!(
            click_event("a, b", 2, 2),
            DragEvent::Word { start: 1, end: 3 }
        );
    }

    #[test]
    fn a_double_click_on_empty_text_selects_nothing() {
        assert_eq!(click_event("", 0, 2), DragEvent::Word { start: 0, end: 0 });
    }

    #[test]
    fn inline_runs_keep_their_styles() {
        let style = TextStyle::default();
        let plain = FontWeight::default();
        let runs = inline_runs(
            &style,
            &[
                Span {
                    len: 2,
                    bold: false,
                    italic: false,
                    code: false,
                    strike: false,
                    underline: false,
                    marked: false,
                    token: None,
                },
                Span {
                    len: 3,
                    bold: true,
                    italic: false,
                    code: false,
                    strike: false,
                    underline: false,
                    marked: false,
                    token: None,
                },
                Span {
                    len: 4,
                    bold: false,
                    italic: true,
                    code: false,
                    strike: false,
                    underline: false,
                    marked: false,
                    token: None,
                },
                Span {
                    len: 5,
                    bold: false,
                    italic: false,
                    code: true,
                    strike: false,
                    underline: false,
                    marked: false,
                    token: None,
                },
            ],
            // The italic run (bytes 5..9) is a link: it is drawn underlined in
            // the link colour.
            &[Link {
                start: 5,
                end: 9,
                url: "https://example.com".to_string(),
            }],
            hsla(0.6, 0.8, 0.5, 1.),
            14,
            None,
            None,
        );
        assert_eq!(runs.iter().map(|run| run.len).sum::<usize>(), 14);
        assert_eq!(runs[0].font.weight, plain);
        assert_eq!(runs[1].font.weight, FontWeight::BOLD);
        assert_eq!(runs[2].font.style, FontStyle::Italic);
        assert_eq!(runs[3].font.family.to_string(), crate::app::MONO_FONT);
        // Only the font changes: inline code is not filled in behind the text.
        assert!(runs[3].background_color.is_none());
        // Only the colour changes: the label keeps the text's font.
        assert_eq!(runs[2].color, hsla(0.6, 0.8, 0.5, 1.));
        assert_eq!(runs[2].font.family, style.font_family);
        assert!(runs[2].underline.is_none());
        assert!(runs[0].underline.is_none());
    }

    #[test]
    fn spans_that_do_not_cover_the_text_fall_back_to_one_run() {
        let style = TextStyle::default();
        let runs = inline_runs(
            &style,
            &[Span {
                len: 1,
                bold: true,
                italic: false,
                code: false,
                strike: false,
                underline: false,
                marked: false,
                token: None,
            }],
            &[],
            hsla(0.6, 0.8, 0.5, 1.),
            5,
            None,
            None,
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, 5);
        assert_eq!(runs[0].font.weight, style.font_weight);
    }
}

//! One small data table the panels share.
//!
//! gpui ships no table element: a table is a `div` tree plus overflow and a
//! scrollbar of its own. The model here is pure — filter, then sort, then page —
//! so it is tested without a window; the renderer draws a sortable header and
//! the rows, and when the caller gives the table a height it clips them into the
//! card with the same thin bar the context panel uses.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use super::*;

/// Element ids a table's fixed pieces take, kept clear of the header and row
/// indices so a header cell never shares an id with the rows.
const ROWS_BODY: usize = usize::MAX - 100;
const ROWS_BAR: usize = usize::MAX - 99;
const FOOTER_PREV: usize = usize::MAX - 98;
const FOOTER_NEXT: usize = usize::MAX - 97;

/// What a table remembers about what the reader asked for.
#[derive(Default)]
pub(crate) struct TableState {
    /// Column being sorted by, and whether it runs downwards.
    pub(crate) sort: Option<(usize, bool)>,
    /// Text the filter keeps; empty keeps everything.
    pub(crate) filter: String,
    /// Page on show, counting from zero.
    pub(crate) page: usize,
    /// Rows per page; zero puts them all on one page.
    pub(crate) page_size: usize,
    /// Keys of the picked rows.
    pub(crate) selected: HashSet<String>,
    /// Row a Shift press extends the selection from.
    pub(crate) anchor: Option<String>,
    /// Vertical scroll, used when the caller gave the table a height.
    pub(crate) scroll: ScrollHandle,
}

impl TableState {
    /// The rows on show, in order: filtered, sorted, then one page of them.
    pub(crate) fn visible(&self, rows: &[Row]) -> Vec<usize> {
        let kept = kept(rows, &self.filter);
        let order = ordered(rows, &kept, self.sort);
        let page = page_of(order.len(), self.page, self.page_size);
        order[page].to_vec()
    }
}

/// How one column's cells line up.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    /// Text starts at the left edge.
    Left,
    /// Figures end at the right edge, so digits line up.
    Right,
}

/// How wide a column is.
#[derive(Clone, Copy)]
pub(crate) enum Width {
    /// A fixed number of pixels.
    Fixed(f32),
    /// Everything the other columns did not take.
    Fill,
}

/// One column.
pub(crate) struct Column {
    /// Text of the header.
    pub(crate) title: &'static str,
    /// Width of every cell in this column, the header included.
    pub(crate) width: Width,
    /// Where the text sits.
    pub(crate) align: Align,
    /// True when a press on the header sorts by this column.
    pub(crate) sortable: bool,
}

impl Column {
    /// A column of figures, right aligned and sortable.
    pub(crate) fn count(title: &'static str, width: f32) -> Self {
        Self {
            title,
            width: Width::Fixed(width),
            align: Align::Right,
            sortable: true,
        }
    }

    /// A text column, left aligned and sortable.
    pub(crate) fn text(title: &'static str, width: f32) -> Self {
        Self {
            title,
            width: Width::Fixed(width),
            align: Align::Left,
            sortable: true,
        }
    }

    /// The column that takes the spare width.
    pub(crate) fn fill(title: &'static str) -> Self {
        Self {
            title,
            width: Width::Fill,
            align: Align::Left,
            sortable: true,
        }
    }
}
/// One cell: what it says, and what a sort reads out of it.
#[derive(Clone, PartialEq)]
pub(crate) enum Value {
    /// Plain text in the normal ink.
    Text(String),
    /// A number, right aligned by its column and sorted as a number.
    Count(u64),
    /// Text in the secondary ink.
    Muted(String),
    /// A dash or a zero, in the faint ink.
    Faint(String),
    /// A moment: drawn as its label, sorted by the milliseconds it carries.
    At(u64, String),
}

impl Value {
    /// What the cell draws.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Text(text) | Self::Muted(text) | Self::Faint(text) => text.clone(),
            Self::Count(count) => count.to_string(),
            Self::At(_, label) => label.clone(),
        }
    }

    /// The number a sort should use, when the cell is one.
    fn number(&self) -> Option<f64> {
        match self {
            Self::Count(count) => Some(*count as f64),
            Self::At(moment, _) => Some(*moment as f64),
            _ => None,
        }
    }
}

/// One row: the key a selection names it by, and its cells.
pub(crate) struct Row {
    /// Stable identity, so a selection survives a re-sort or a new page.
    pub(crate) key: String,
    /// One cell per column, in column order.
    pub(crate) cells: Vec<Value>,
}

impl Row {
    /// A row with its key and cells.
    pub(crate) fn new(key: impl Into<String>, cells: Vec<Value>) -> Self {
        Self {
            key: key.into(),
            cells,
        }
    }
}

/// Indices of the rows the filter keeps, in table order.
pub(crate) fn kept(rows: &[Row], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_lowercase();
    if needle.is_empty() {
        return (0..rows.len()).collect();
    }
    rows.iter()
        .enumerate()
        .filter(|(_, row)| {
            row.cells
                .iter()
                .any(|cell| cell.text().to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect()
}

/// `kept` ordered by the sort in force; ties keep the table's own order.
pub(crate) fn ordered(rows: &[Row], kept: &[usize], sort: Option<(usize, bool)>) -> Vec<usize> {
    let Some((column, descending)) = sort else {
        return kept.to_vec();
    };
    let mut order = kept.to_vec();
    // `sort_by` is stable, so equal cells stay in the order they came in.
    order.sort_by(|left, right| {
        let left_cell = rows[*left].cells.get(column);
        let right_cell = rows[*right].cells.get(column);
        let order = match (
            left_cell.and_then(Value::number),
            right_cell.and_then(Value::number),
        ) {
            (Some(left), Some(right)) => left
                .partial_cmp(&right)
                .unwrap_or(std::cmp::Ordering::Equal),
            _ => {
                let left = left_cell.map_or_else(String::new, |cell| cell.text().to_lowercase());
                let right = right_cell.map_or_else(String::new, |cell| cell.text().to_lowercase());
                left.cmp(&right)
            }
        };
        if descending { order.reverse() } else { order }
    });
    order
}

/// Where one page starts and ends, with the page pulled back into range.
pub(crate) fn page_of(total: usize, page: usize, page_size: usize) -> Range<usize> {
    if page_size == 0 {
        return 0..total;
    }
    let page = page.min(page_count(total, page_size) - 1);
    let start = page * page_size;
    start..(start + page_size).min(total)
}

/// How many pages a table has; one when paging is off.
pub(crate) fn page_count(total: usize, page_size: usize) -> usize {
    if page_size == 0 {
        1
    } else {
        total.div_ceil(page_size).max(1)
    }
}

/// A press on a sortable header: a new column starts rising, the same one turns
/// around, and a third press drops the sort again.
pub(crate) fn toggle_sort(state: &mut TableState, column: usize) {
    state.sort = match state.sort {
        Some((current, false)) if current == column => Some((column, true)),
        Some((current, true)) if current == column => None,
        _ => Some((column, false)),
    };
    state.page = 0;
}

/// A press on a row: plain replaces the selection, Shift extends it from the
/// last press, and a second press on the only picked row clears it.
pub(crate) fn select_row(
    state: &mut TableState,
    order: &[usize],
    rows: &[Row],
    key: &str,
    extend: bool,
) {
    if extend
        && let Some(anchor) = state.anchor.clone()
        && let Some(from) = order.iter().position(|at| rows[*at].key == anchor)
        && let Some(to) = order.iter().position(|at| rows[*at].key == key)
    {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        state.selected.clear();
        for at in &order[from..=to] {
            state.selected.insert(rows[*at].key.clone());
        }
        return;
    }
    let key = key.to_string();
    // A plain press replaces the selection; pressing the only picked row again
    // clears it.
    if state.selected.len() == 1 && state.selected.contains(&key) {
        state.selected.clear();
    } else {
        state.selected.clear();
        state.selected.insert(key.clone());
    }
    state.anchor = Some(key);
}

/// The look of one header cell or one cell, shared by both rows.
fn cell_box(width: Width, align: Align) -> Div {
    let cell = div().min_w(px(0.));
    let cell = match width {
        Width::Fixed(pixels) => cell.flex_shrink_0().w(px(pixels)),
        Width::Fill => cell.flex_1(),
    };
    match align {
        Align::Left => cell,
        Align::Right => cell.flex().flex_row().justify_end(),
    }
}
impl DshFlowApp {
    /// Set up a table before its first draw: the page size and the sort it opens
    /// on. A table with nothing prepared draws every row, unsorted.
    pub(crate) fn prepare_table(
        &self,
        id: &'static str,
        page_size: usize,
        sort: Option<(usize, bool)>,
    ) {
        self.tables.borrow_mut().entry(id).or_insert_with(|| TableState {
            page_size,
            sort,
            ..TableState::default()
        });
    }

    /// Draw one table.
    ///
    /// `height` is the room the rows get: `Some` clips them into the card and
    /// gives them a bar, `None` lets them flow in the page's own scroll. A press
    /// selects a row; `open` is told about it as well, for a table whose rows
    /// open something (the context panel's request list).
    pub(crate) fn render_table(
        &self,
        id: &'static str,
        columns: &[Column],
        rows: Vec<Row>,
        height: Option<Pixels>,
        open: Option<Rc<dyn Fn(&mut DshFlowApp, &str, &mut Context<DshFlowApp>)>>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The rows are shared with the click handlers, which have to outlive the
        // frame they were drawn in.
        let rows = Rc::new(rows);
        let (sort, visible, order, selected, scroll) = {
            let tables = self.tables.borrow();
            let state = tables.get(id);
            let sort = state.and_then(|state| state.sort);
            let visible = state.map_or_else(|| (0..rows.len()).collect(), |state| {
                state.visible(&rows)
            });
            // The order a Shift press runs over: every filtered row, not just the
            // page on show.
            let order: Rc<Vec<usize>> = Rc::new(state.map_or_else(Vec::new, |state| {
                ordered(&rows, &kept(&rows, &state.filter), state.sort)
            }));
            let selected = state.map(|state| state.selected.clone());
            let scroll = state.map(|state| state.scroll.clone());
            (sort, visible, order, selected, scroll)
        };
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .px_1()
            .pb_1()
            .border_b_1()
            .border_color(rgb(palette.border));
        for (index, column) in columns.iter().enumerate() {
            let sorted = sort.filter(|(at, _)| *at == index).map(|(_, desc)| desc);
            let arrow = match sorted {
                Some(true) => " ↓",
                Some(false) => " ↑",
                None => "",
            };
            let mut cell = cell_box(column.width, column.align)
                .id((id, index))
                .text_xs()
                .text_color(rgb(if sorted.is_some() {
                    palette.text
                } else {
                    palette.muted
                }))
                .child(format!("{}{arrow}", column.title));
            if column.sortable {
                cell = cell
                    .cursor_pointer()
                    .hover(move |style| style.text_color(rgb(palette.text)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        toggle_sort(view.tables.borrow_mut().entry(id).or_default(), index);
                        cx.notify();
                    }));
            }
            header = header.child(cell);
        }
        // rows-body
        let mut body = div().flex().flex_col().gap_1().w_full().min_w(px(0.));
        for at in &visible {
            let row = &rows[*at];
            let picked = selected
                .as_ref()
                .is_some_and(|picked| picked.contains(&row.key));
            let key: String = row.key.clone();
            let for_click = key.clone();
            let order_for_click = order.clone();
            let rows_for_click = rows.clone();
            let open_for_click = open.clone();
            let mut line = div()
                .id((id, *at))
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .w_full()
                .min_w(px(0.))
                .px_1()
                .py_1()
                .rounded_md()
                .bg(rgb(if picked {
                    palette.panel_soft
                } else {
                    palette.panel
                }))
                .border_1()
                .border_color(rgb(if picked {
                    palette.accent
                } else {
                    palette.border
                }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(palette.panel_soft)))
                .on_click(cx.listener(move |view, event: &gpui::ClickEvent, _, cx| {
                    let extend = event.modifiers().shift;
                    {
                        let mut tables = view.tables.borrow_mut();
                        let state = tables.entry(id).or_default();
                        select_row(state, &order_for_click, &rows_for_click, &for_click, extend);
                    }
                    if let Some(open) = open_for_click.clone() {
                        open(view, &for_click, cx);
                    }
                    cx.notify();
                }));
            for (index, cell) in row.cells.iter().enumerate() {
                let column = &columns[index.min(columns.len().saturating_sub(1))];
                let ink = match cell {
                    Value::Faint(_) => palette.faint,
                    Value::Muted(_) => palette.muted,
                    _ => palette.text,
                };
                line = line.child(
                    cell_box(column.width, column.align)
                        .text_xs()
                        .text_color(rgb(ink))
                        .line_clamp(1)
                        .child(cell.text()),
                );
            }
            body = body.child(line);
        }
        // A height turns the rows into their own scroll area, with the same bar
        // the other cards use; without one they ride the page's scroll.
        let rows_area = match height {
            Some(height) => {
                let scroll = scroll.clone().unwrap_or_else(ScrollHandle::new);
                let bar = self.render_table_bar(id, palette, cx);
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .w_full()
                    .min_w(px(0.))
                    .h(height)
                    .child(
                        div()
                            .id((id, ROWS_BODY))
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .child(body),
                    )
                    .child(bar)
                    .into_any_element()
            }
            None => body.into_any_element(),
        };

        let footer = self.render_table_footer(id, rows.len(), visible.len(), palette, cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .child(header)
            .child(rows_area)
            .when_some(footer, |this, footer| this.child(footer))
            .into_any_element()
    }

    /// Paging controls and the count, when the table asked for pages.
    fn render_table_footer(
        &self,
        id: &'static str,
        total: usize,
        shown: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (page_size, page, picked) = {
            let tables = self.tables.borrow();
            let state = tables.get(id)?;
            (state.page_size, state.page, state.selected.len())
        };
        if page_size == 0 {
            return None;
        }
        let pages = page_count(total, page_size);
        let page = page.min(pages - 1);
        let button = |label: &'static str, id_value: usize, enabled: bool, previous: bool| {
            let mut button = div()
                .id((id, id_value))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .border_1()
                .border_color(rgb(palette.border))
                .text_xs()
                .text_color(rgb(if enabled { palette.text } else { palette.faint }))
                .child(label);
            if enabled {
                button = button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(palette.panel_soft)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        let mut tables = view.tables.borrow_mut();
                        let state = tables.entry(id).or_default();
                        state.page = if previous {
                            state.page.saturating_sub(1)
                        } else {
                            state.page.saturating_add(1)
                        };
                        cx.notify();
                    }));
            }
            button
        };
        let mut footer = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.))
            .pt_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("第 {}/{} 页 · {} 行", page + 1, pages, shown)),
            )
            .child(div().flex_1().min_w(px(0.)))
            .child(button("‹", FOOTER_PREV, page > 0, true))
            .child(button("›", FOOTER_NEXT, page + 1 < pages, false));
        if picked > 0 {
            footer = footer.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.faint))
                    .child(format!("已选 {picked}")),
            );
        }
        Some(footer.into_any_element())
    }

    /// The thin bar down a table's own scroll area.
    fn render_table_bar(
        &self,
        id: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(scroll) = self.tables.borrow().get(id).map(|state| state.scroll.clone()) else {
            return div().into_any_element();
        };
        let Some((track, thumb, max, _)) = tool_bar_metrics(&scroll) else {
            return div().into_any_element();
        };
        let travel = (track - thumb).max(px(0.));
        let progress = if max > px(0.) {
            (-scroll.offset().y / max).clamp(0., 1.)
        } else {
            0.
        };
        let dragging = self
            .table_drag
            .as_ref()
            .is_some_and(|(dragging, _, _)| *dragging == id);
        let handle = scroll.clone();
        div()
            .id((id, ROWS_BAR))
            .w(px(8.))
            .flex_shrink_0()
            .h(track + px(4.))
            .flex()
            .flex_col()
            .items_center()
            .block_mouse_except_scroll()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, event: &MouseDownEvent, _, cx| {
                    let Some((track, thumb, max, top)) = tool_bar_metrics(&handle) else {
                        return;
                    };
                    let travel = (track - thumb).max(px(1.));
                    let progress = if max > px(0.) {
                        (-handle.offset().y / max).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let thumb_top = top + travel * progress;
                    let on_thumb =
                        event.position.y >= thumb_top && event.position.y <= thumb_top + thumb;
                    if !on_thumb {
                        let ratio = ((event.position.y - top) / travel).clamp(0., 1.);
                        let current = handle.offset();
                        handle.set_offset(point(current.x, -max * ratio));
                    }
                    view.table_drag = Some((id, event.position.y, handle.offset().y));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(6.))
                    .h(thumb)
                    .rounded_full()
                    .bg(rgb(if dragging {
                        palette.muted
                    } else {
                        palette.border
                    }))
                    .mt(travel * progress),
            )
            .into_any_element()
    }

    /// Follow the pointer while one table's bar is held.
    pub(crate) fn drag_table_bar(&mut self, pointer_y: Pixels, cx: &mut Context<Self>) {
        let Some((id, start_y, start_offset)) = self.table_drag else {
            return;
        };
        let Some(scroll) = self.tables.borrow().get(id).map(|state| state.scroll.clone()) else {
            return;
        };
        let Some((track, thumb, max, _)) = tool_bar_metrics(&scroll) else {
            return;
        };
        let travel = (track - thumb).max(px(1.));
        let delta = (start_y - pointer_y) * (max / travel);
        let offset = (start_offset + delta).clamp(-max, px(0.));
        let current = scroll.offset();
        scroll.set_offset(point(current.x, offset));
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, cells: Vec<Value>) -> Row {
        Row::new(key, cells)
    }

    fn table() -> Vec<Row> {
        vec![
            row("a", vec![Value::Text("alpha".into()), Value::Count(30)]),
            row("b", vec![Value::Text("Beta".into()), Value::Count(10)]),
            row("c", vec![Value::Text("gamma".into()), Value::Count(20)]),
        ]
    }

    #[test]
    fn sorting_flips_direction_and_then_switches_off() {
        let mut state = TableState::default();
        toggle_sort(&mut state, 1);
        assert_eq!(state.sort, Some((1, false)), "a new column rises");
        toggle_sort(&mut state, 1);
        assert_eq!(state.sort, Some((1, true)), "the same column falls");
        toggle_sort(&mut state, 1);
        assert_eq!(state.sort, None, "a third press drops the sort");
        toggle_sort(&mut state, 0);
        assert_eq!(state.sort, Some((0, false)), "another column starts over");
    }

    #[test]
    fn the_rows_come_out_filtered_sorted_and_paged() {
        let rows = table();
        let all: Vec<usize> = (0..rows.len()).collect();

        // Filtering reads every cell's text and ignores case.
        assert_eq!(kept(&rows, "BETA"), vec![1]);
        assert_eq!(kept(&rows, "20"), vec![2]);
        assert!(kept(&rows, "nothing").is_empty());
        assert_eq!(kept(&rows, "   "), all);

        // Numbers sort as numbers, text as lower-cased text.
        assert_eq!(ordered(&rows, &all, Some((1, false))), vec![1, 2, 0]);
        assert_eq!(ordered(&rows, &all, Some((1, true))), vec![0, 2, 1]);
        assert_eq!(ordered(&rows, &all, Some((0, false))), vec![0, 1, 2]);
        assert_eq!(ordered(&rows, &all, None), all);

        // Pages clamp at both ends and the last one is short.
        assert_eq!(page_of(7, 0, 3), 0..3);
        assert_eq!(page_of(7, 2, 3), 6..7);
        assert_eq!(page_of(7, 99, 3), 6..7, "a page past the end is the last");
        assert_eq!(page_of(3, 0, 0), 0..3, "no page size keeps them all");
        assert_eq!(page_count(7, 3), 3);
        assert_eq!(page_count(0, 3), 1);
        assert_eq!(page_count(7, 0), 1);
    }

    #[test]
    fn a_press_selects_one_row_or_extends_a_range() {
        let rows = table();
        let order: Vec<usize> = (0..rows.len()).collect();
        let mut state = TableState::default();

        select_row(&mut state, &order, &rows, "b", false);
        assert_eq!(state.selected.len(), 1);
        assert!(state.selected.contains("b"));
        assert_eq!(state.anchor.as_deref(), Some("b"));

        // Shift reaches everything between the anchor and the press.
        select_row(&mut state, &order, &rows, "a", true);
        assert_eq!(state.selected.len(), 2);
        assert!(state.selected.contains("a") && state.selected.contains("b"));
        assert!(!state.selected.contains("c"));

        // A plain press replaces the selection.
        select_row(&mut state, &order, &rows, "c", false);
        assert_eq!(state.selected.len(), 1);
        assert!(state.selected.contains("c"));

        // Pressing the same row again clears it.
        select_row(&mut state, &order, &rows, "c", false);
        assert!(state.selected.is_empty());
    }

    #[test]
    fn the_visible_page_is_what_the_renderer_draws() {
        let rows = table();
        let mut state = TableState {
            sort: Some((1, false)),
            page_size: 2,
            ..TableState::default()
        };
        assert_eq!(state.visible(&rows), vec![1, 2]);
        state.page = 1;
        assert_eq!(state.visible(&rows), vec![0]);
        state.filter = "alpha".to_string();
        state.page = 0;
        assert_eq!(state.visible(&rows), vec![0]);
    }
}
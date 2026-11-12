//! The Network tab's table: column layout, click-to-sort headings, and the
//! right-click column menu that shows or hides columns.

use ab_glyph::FontVec;
use tiny_skia::{Color, PathBuilder, Pixmap, Rect, Transform};

use super::{Colors, ROW, SCROLLBAR_GUTTER, SMALL_SIZE, TREE_SIZE, hairline};
use crate::toolbar;
use devtools::network::{NetworkLog, RequestEntry, ResourceType};

/// A column of the Network table, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NetworkColumn {
    Name,
    Status,
    Type,
    Queue,
    Headers,
    Time,
}

impl NetworkColumn {
    pub(super) const ALL: [Self; 6] = [
        Self::Name,
        Self::Status,
        Self::Type,
        Self::Queue,
        Self::Headers,
        Self::Time,
    ];

    fn title(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Status => "Status",
            Self::Type => "Type",
            Self::Queue => "Queue",
            Self::Headers => "Headers",
            Self::Time => "Time",
        }
    }

    /// Width in logical pixels; `Name` takes whatever the others leave.
    fn width(self) -> f32 {
        match self {
            Self::Name => 0.0,
            Self::Status => 76.0,
            Self::Type => 104.0,
            Self::Queue => 76.0,
            Self::Headers => 88.0,
            Self::Time => 84.0,
        }
    }

    /// Numbers read best aligned on their last digit.
    fn right_aligned(self) -> bool {
        matches!(
            self,
            Self::Status | Self::Queue | Self::Headers | Self::Time
        )
    }

    /// `Name` identifies each request, so it cannot be hidden.
    fn hideable(self) -> bool {
        self != Self::Name
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|column| *column == self)
            .unwrap_or(0)
    }

    /// The text shown in this column for `entry`.
    fn cell(self, entry: &RequestEntry) -> String {
        match self {
            Self::Name => entry.url.clone(),
            Self::Status => entry
                .status
                .map_or_else(|| "failed".to_owned(), |status| status.to_string()),
            Self::Type => resource_type_name(entry.resource_type).to_owned(),
            Self::Queue => entry
                .queue_ms
                .map_or_else(|| "—".to_owned(), |ms| format!("{ms} ms")),
            Self::Headers => entry
                .headers_ms
                .map_or_else(|| "—".to_owned(), |ms| format!("{ms} ms")),
            Self::Time => format!("{} ms", entry.duration_ms),
        }
    }

    /// Orders two entries by this column, ascending.
    fn compare(self, a: &RequestEntry, b: &RequestEntry) -> std::cmp::Ordering {
        match self {
            Self::Name => a.url.to_ascii_lowercase().cmp(&b.url.to_ascii_lowercase()),
            // Failed requests (no status) sort after every real status.
            Self::Status => (a.status.is_none(), a.status).cmp(&(b.status.is_none(), b.status)),
            Self::Type => {
                resource_type_name(a.resource_type).cmp(resource_type_name(b.resource_type))
            }
            Self::Queue => a.queue_ms.cmp(&b.queue_ms),
            Self::Headers => a.headers_ms.cmp(&b.headers_ms),
            Self::Time => a.duration_ms.cmp(&b.duration_ms),
        }
    }
}

/// Lowercase name of a resource type, as the Type column shows it.
fn resource_type_name(resource_type: ResourceType) -> &'static str {
    match resource_type {
        ResourceType::Document => "document",
        ResourceType::Stylesheet => "stylesheet",
        ResourceType::Script => "script",
        ResourceType::Image => "image",
        ResourceType::Other => "other",
    }
}

/// Direction of the active sort.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SortDirection {
    Ascending,
    Descending,
}

/// Height of the table's heading row in logical pixels.
pub(super) const HEADING: f32 = 24.0;
/// Column menu width in logical pixels.
const MENU_WIDTH: f32 = 168.0;
/// Inner padding around the column menu's items in logical pixels.
const MENU_PAD: f32 = 5.0;
/// Column menu item height in logical pixels.
const MENU_ROW: f32 = 24.0;
/// Horizontal padding inside each cell in logical pixels.
const CELL_PAD: f32 = 10.0;

/// The Network table's view state: which columns show, how rows are sorted,
/// and whether the column menu is open. Requests themselves live in the
/// panel's [`NetworkLog`].
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NetworkView {
    /// Indexed like [`NetworkColumn::ALL`].
    visible: [bool; 6],
    sort: Option<(NetworkColumn, SortDirection)>,
    /// Top-left corner of the open column menu, in logical pixels.
    menu: Option<(f32, f32)>,
}

impl NetworkView {
    pub(super) fn new() -> Self {
        Self {
            visible: [true; 6],
            sort: None,
            menu: None,
        }
    }

    pub(super) fn is_visible(&self, column: NetworkColumn) -> bool {
        self.visible[column.index()]
    }

    /// Shows or hides `column`. `Name` always stays visible.
    pub(super) fn toggle(&mut self, column: NetworkColumn) {
        if column.hideable() {
            self.visible[column.index()] ^= true;
        }
    }

    #[cfg(test)]
    pub(super) fn sort(&self) -> Option<(NetworkColumn, SortDirection)> {
        self.sort
    }

    /// Clicking a heading sorts by it ascending, then descending, then
    /// returns to request order.
    pub(super) fn click_heading(&mut self, column: NetworkColumn) {
        self.sort = match self.sort {
            Some((current, SortDirection::Ascending)) if current == column => {
                Some((column, SortDirection::Descending))
            }
            Some((current, SortDirection::Descending)) if current == column => None,
            _ => Some((column, SortDirection::Ascending)),
        };
    }

    /// Indices into `log`'s entries in display order. Ties keep request
    /// order.
    pub(super) fn order(&self, log: &NetworkLog) -> Vec<usize> {
        let entries = log.entries();
        let mut order: Vec<usize> = (0..entries.len()).collect();
        if let Some((column, direction)) = self.sort {
            order.sort_by(|&a, &b| {
                let ordering = column.compare(&entries[a], &entries[b]);
                match direction {
                    SortDirection::Ascending => ordering,
                    SortDirection::Descending => ordering.reverse(),
                }
            });
        }
        order
    }

    /// The visible columns' `(column, left, right)` edges inside `table`.
    fn columns(&self, table: Rect) -> Vec<(NetworkColumn, f32, f32)> {
        let right = table.right() - SCROLLBAR_GUTTER;
        let fixed: f32 = NetworkColumn::ALL
            .into_iter()
            .filter(|column| column.hideable() && self.is_visible(*column))
            .map(NetworkColumn::width)
            .sum();
        let name_right = (right - fixed).max(table.left() + 4.0 + CELL_PAD * 2.0);
        let mut left = table.left() + 4.0;
        NetworkColumn::ALL
            .into_iter()
            .filter(|column| self.is_visible(*column))
            .map(|column| {
                let edge = if column == NetworkColumn::Name {
                    name_right
                } else {
                    left + column.width()
                };
                let cell = (column, left, edge);
                left = edge;
                cell
            })
            .collect()
    }

    /// The column whose heading is under `cursor`.
    pub(super) fn heading_at(
        &self,
        table: Rect,
        cursor: Option<(f64, f64)>,
    ) -> Option<NetworkColumn> {
        let (x, y) = cursor?;
        let (x, y) = (x as f32, y as f32);
        if y < table.top() || y >= table.top() + HEADING {
            return None;
        }
        self.columns(table)
            .into_iter()
            .find(|(_, left, right)| x >= *left && x < *right)
            .map(|(column, _, _)| column)
    }

    /// Index into the visible rows under `cursor`.
    pub(super) fn row_at(&self, table: Rect, cursor: Option<(f64, f64)>) -> Option<usize> {
        let (x, y) = cursor?;
        let (x, y) = (x as f32, y as f32);
        let top = table.top() + HEADING;
        (x >= table.left()
            && x < table.right() - SCROLLBAR_GUTTER
            && y >= top
            && y < table.bottom())
        .then(|| ((y - top) / ROW) as usize)
    }

    /// Opens the column menu at `cursor`, kept inside `bounds`.
    pub(super) fn open_menu(&mut self, cursor: (f64, f64), bounds: Rect) {
        let height = menu_height();
        let x = (cursor.0 as f32).clamp(
            bounds.left(),
            (bounds.right() - MENU_WIDTH).max(bounds.left()),
        );
        let y = (cursor.1 as f32).clamp(bounds.top(), (bounds.bottom() - height).max(bounds.top()));
        self.menu = Some((x, y));
    }

    /// Closes the column menu; returns whether it was open.
    pub(super) fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    pub(super) fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    fn menu_rect(&self) -> Option<Rect> {
        let (x, y) = self.menu?;
        Rect::from_xywh(x, y, MENU_WIDTH, menu_height())
    }

    /// The column whose menu item is under `cursor`.
    pub(super) fn menu_item_at(&self, cursor: Option<(f64, f64)>) -> Option<NetworkColumn> {
        let rect = self.menu_rect()?;
        let (x, y) = cursor?;
        let (x, y) = (x as f32, y as f32);
        if x < rect.left() || x >= rect.right() {
            return None;
        }
        let index = ((y - rect.top() - MENU_PAD) / MENU_ROW).floor();
        (index >= 0.0)
            .then(|| NetworkColumn::ALL.get(index as usize).copied())
            .flatten()
    }

    /// Draws the heading row and the request rows starting at `offset` into
    /// `table`. `s` is the device scale.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        frame: &mut Pixmap,
        s: f32,
        palette: &toolbar::Palette,
        colors: &Colors,
        table: Rect,
        log: &NetworkLog,
        offset: usize,
        rows: usize,
        cursor: Option<(f64, f64)>,
        font: Option<&FontVec>,
    ) {
        let transform = Transform::from_scale(s, s);
        let columns = self.columns(table);
        let entries = log.entries();
        let order = self.order(log);
        let heading_bottom = table.top() + HEADING;

        // Heading background, hover, and dividers between columns.
        if let Some(rect) =
            Rect::from_ltrb(table.left(), table.top(), table.right(), heading_bottom)
        {
            toolbar::fill(
                frame,
                &PathBuilder::from_rect(rect),
                colors.header,
                transform,
            );
        }
        let hovered_heading = if self.menu.is_none() {
            self.heading_at(table, cursor)
        } else {
            None
        };
        for (column, left, right) in &columns {
            if hovered_heading == Some(*column)
                && let Some(rect) = Rect::from_ltrb(*left, table.top(), *right, heading_bottom)
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(rect),
                    palette.hover,
                    transform,
                );
            }
            if *left > table.left() + 4.0
                && let Some(rect) = Rect::from_xywh(*left, table.top(), 1.0, table.height())
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(rect),
                    colors.guide,
                    transform,
                );
            }
        }
        hairline(
            frame,
            table.left(),
            table.right(),
            heading_bottom - 1.0,
            colors.separator,
            transform,
        );

        // Rows: alternate stripes, pointer hover, then cell text.
        let hovered_row = if self.menu.is_none() {
            self.row_at(table, cursor)
        } else {
            None
        };
        let shown = rows.min(order.len().saturating_sub(offset));
        for index in 0..shown {
            let top = heading_bottom + index as f32 * ROW;
            let fill = if hovered_row == Some(index) {
                Some(palette.hover)
            } else {
                (index % 2 == 1).then_some(colors.row_hover)
            };
            if let Some(color) = fill
                && let Some(rect) = Rect::from_xywh(table.left(), top, table.width(), ROW)
            {
                toolbar::fill(frame, &PathBuilder::from_rect(rect), color, transform);
            }
        }

        let Some(font) = font else {
            return;
        };
        let device = |value: f32| value * s;
        for (column, left, right) in &columns {
            let sorted = self.sort.filter(|(sorted, _)| sorted == column);
            let title = column.title();
            let width = toolbar::text_advance(font, device(SMALL_SIZE), title) / s;
            let title_left = if column.right_aligned() {
                // Leave room for the sort arrow on the right.
                (right - CELL_PAD - 12.0 - width).max(left + CELL_PAD)
            } else {
                left + CELL_PAD
            };
            toolbar::draw_text(
                frame,
                font,
                device(SMALL_SIZE),
                device(title_left),
                device(table.top() + HEADING / 2.0 + 4.0),
                device(right - CELL_PAD),
                if sorted.is_some() {
                    palette.text
                } else {
                    palette.text_muted
                },
                title,
            );
            if let Some((_, direction)) = sorted {
                let x = (title_left + width + 6.0).min(right - CELL_PAD - 7.0);
                sort_arrow(
                    frame,
                    x,
                    table.top() + HEADING / 2.0,
                    direction,
                    palette.text,
                    transform,
                );
            }
        }
        for (index, entry_index) in order.into_iter().skip(offset).take(shown).enumerate() {
            let entry = &entries[entry_index];
            let baseline = heading_bottom + index as f32 * ROW + ROW / 2.0 + 4.0;
            for (column, left, right) in &columns {
                let text = column.cell(entry);
                let color = match (column, entry.status) {
                    (NetworkColumn::Name, _) => palette.text,
                    (NetworkColumn::Status, None) => toolbar::rgba(0xdc, 0x26, 0x26, 255),
                    (NetworkColumn::Status, Some(status)) if status >= 400 => {
                        toolbar::rgba(0xdc, 0x26, 0x26, 255)
                    }
                    _ => palette.text_muted,
                };
                let text_left = if column.right_aligned() {
                    let width = toolbar::text_advance(font, device(TREE_SIZE), &text) / s;
                    (right - CELL_PAD - width).max(left + CELL_PAD)
                } else {
                    left + CELL_PAD
                };
                toolbar::draw_text(
                    frame,
                    font,
                    device(TREE_SIZE),
                    device(text_left),
                    device(baseline),
                    device(right - CELL_PAD),
                    color,
                    &text,
                );
            }
        }
    }

    /// Draws the column menu, if open, over everything else in the panel.
    pub(super) fn draw_menu(
        &self,
        frame: &mut Pixmap,
        s: f32,
        dark: bool,
        cursor: Option<(f64, f64)>,
        font: Option<&FontVec>,
    ) {
        let Some(rect) = self.menu_rect() else {
            return;
        };
        let palette = toolbar::palette(dark);
        let rgba = toolbar::rgba;
        let transform = Transform::from_scale(s, s);
        let (x, y, w, h) = (rect.x(), rect.y(), rect.width(), rect.height());
        // Same layered soft shadow and surface as the page context menu.
        let shadow_alpha = if dark { 12 } else { 4 };
        for spread in (1..=6).rev() {
            let spread = spread as f32;
            if let Some(path) = toolbar::rounded_rect(
                x - spread,
                y - spread + 3.0,
                w + 2.0 * spread,
                h + 2.0 * spread,
                8.0 + spread,
            ) {
                toolbar::fill(frame, &path, rgba(0, 0, 0, shadow_alpha), transform);
            }
        }
        let (surface, border) = if dark {
            (rgba(0x2a, 0x2b, 0x31, 255), rgba(255, 255, 255, 30))
        } else {
            (rgba(255, 255, 255, 255), rgba(15, 23, 42, 34))
        };
        if let Some(path) = toolbar::rounded_rect(x, y, w, h, 8.0) {
            toolbar::fill(frame, &path, surface, transform);
        }
        if let Some(path) = toolbar::rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, 7.5) {
            toolbar::stroke(frame, &path, border, 1.0, transform);
        }
        let hovered = self.menu_item_at(cursor).filter(|column| column.hideable());
        for (index, column) in NetworkColumn::ALL.into_iter().enumerate() {
            let top = y + MENU_PAD + index as f32 * MENU_ROW;
            let selected = hovered == Some(column);
            if selected
                && let Some(path) =
                    toolbar::rounded_rect(x + MENU_PAD, top, w - 2.0 * MENU_PAD, MENU_ROW, 5.0)
            {
                toolbar::fill(frame, &path, palette.accent, transform);
            }
            let color = if selected {
                Color::WHITE
            } else if column.hideable() {
                palette.text
            } else {
                palette.icon_disabled
            };
            if self.is_visible(column) {
                let (cx, cy) = (x + MENU_PAD + 14.0, top + MENU_ROW / 2.0);
                let mut check = PathBuilder::new();
                check.move_to(cx - 4.0, cy);
                check.line_to(cx - 1.0, cy + 3.0);
                check.line_to(cx + 4.5, cy - 3.5);
                if let Some(path) = check.finish() {
                    toolbar::stroke(frame, &path, color, 1.6, transform);
                }
            }
            if let Some(font) = font {
                toolbar::draw_text(
                    frame,
                    font,
                    13.0 * s,
                    (x + MENU_PAD + 28.0) * s,
                    (top + MENU_ROW / 2.0 + 4.5) * s,
                    (x + w - MENU_PAD - 8.0) * s,
                    color,
                    column.title(),
                );
            }
        }
    }
}

fn menu_height() -> f32 {
    2.0 * MENU_PAD + NetworkColumn::ALL.len() as f32 * MENU_ROW
}

/// A small filled triangle centered vertically on `cy`, pointing up for
/// ascending and down for descending.
fn sort_arrow(
    frame: &mut Pixmap,
    x: f32,
    cy: f32,
    direction: SortDirection,
    color: Color,
    transform: Transform,
) {
    let (tip, base) = match direction {
        SortDirection::Ascending => (cy - 3.0, cy + 2.0),
        SortDirection::Descending => (cy + 3.0, cy - 2.0),
    };
    let mut arrow = PathBuilder::new();
    arrow.move_to(x, base);
    arrow.line_to(x + 7.0, base);
    arrow.line_to(x + 3.5, tip);
    arrow.close();
    if let Some(path) = arrow.finish() {
        toolbar::fill(frame, &path, color, transform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> NetworkLog {
        let mut log = NetworkLog::new();
        for (url, status, kind, time) in [
            ("https://a.test/b.js", Some(200), ResourceType::Script, 30),
            ("https://a.test/", Some(200), ResourceType::Document, 10),
            ("https://a.test/a.css", None, ResourceType::Stylesheet, 20),
            (
                "https://a.test/c.css",
                Some(404),
                ResourceType::Stylesheet,
                20,
            ),
        ] {
            log.record(RequestEntry::new(url, status, kind, time));
        }
        log
    }

    fn table() -> Rect {
        Rect::from_xywh(0.0, 100.0, 800.0, 200.0).unwrap()
    }

    #[test]
    fn heading_clicks_cycle_ascending_descending_and_request_order() {
        let log = log();
        let mut view = NetworkView::new();
        assert_eq!(view.order(&log), [0, 1, 2, 3]);
        view.click_heading(NetworkColumn::Time);
        assert_eq!(
            view.sort(),
            Some((NetworkColumn::Time, SortDirection::Ascending))
        );
        assert_eq!(view.order(&log), [1, 2, 3, 0], "ties keep request order");
        view.click_heading(NetworkColumn::Time);
        assert_eq!(view.order(&log), [0, 2, 3, 1]);
        view.click_heading(NetworkColumn::Time);
        assert_eq!(view.sort(), None);
        assert_eq!(view.order(&log), [0, 1, 2, 3]);

        view.click_heading(NetworkColumn::Status);
        assert_eq!(view.order(&log), [0, 1, 3, 2], "failed requests sort last");
        view.click_heading(NetworkColumn::Name);
        assert_eq!(
            view.sort(),
            Some((NetworkColumn::Name, SortDirection::Ascending)),
            "a different column starts ascending"
        );
        assert_eq!(view.order(&log), [1, 2, 0, 3]);
    }

    #[test]
    fn headings_are_hit_tested_by_column_and_follow_hidden_columns() {
        let mut view = NetworkView::new();
        let table = table();
        let at = |x: f32| Some((f64::from(x), f64::from(table.top() + 5.0)));
        let columns = view.columns(table);
        assert_eq!(columns.len(), 6);
        for (column, left, right) in &columns {
            assert!(left < right);
            assert_eq!(view.heading_at(table, at(left + 1.0)), Some(*column));
        }
        assert_eq!(
            view.heading_at(table, Some((20.0, f64::from(table.top() + HEADING + 2.0)))),
            None,
            "the first row is not a heading"
        );

        view.toggle(NetworkColumn::Type);
        view.toggle(NetworkColumn::Name);
        assert!(!view.is_visible(NetworkColumn::Type));
        assert!(
            view.is_visible(NetworkColumn::Name),
            "Name cannot be hidden"
        );
        let narrowed = view.columns(table);
        assert_eq!(
            narrowed
                .iter()
                .map(|(column, _, _)| *column)
                .collect::<Vec<_>>(),
            [
                NetworkColumn::Name,
                NetworkColumn::Status,
                NetworkColumn::Queue,
                NetworkColumn::Headers,
                NetworkColumn::Time
            ]
        );
        assert!(narrowed[0].2 > columns[0].2, "Name widens into the space");
    }

    #[test]
    fn column_menu_opens_inside_the_panel_and_hit_tests_items() {
        let mut view = NetworkView::new();
        let bounds = table();
        view.open_menu((790.0, 290.0), bounds);
        let rect = view.menu_rect().unwrap();
        assert!(rect.right() <= bounds.right() && rect.bottom() <= bounds.bottom());
        for (index, column) in NetworkColumn::ALL.into_iter().enumerate() {
            let y = rect.top() + MENU_PAD + index as f32 * MENU_ROW + MENU_ROW / 2.0;
            let cursor = Some((f64::from(rect.left() + 20.0), f64::from(y)));
            assert_eq!(view.menu_item_at(cursor), Some(column));
        }
        assert!(view.close_menu());
        assert!(!view.close_menu());
    }
}

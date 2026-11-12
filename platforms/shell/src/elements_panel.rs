//! The bottom DevTools dock: a small read-only Elements tree derived from
//! loaded HTML (not the live JS DOM), the Compatibility audit from
//! [`crate::compat_panel`], the Network and Console session logs, and the
//! Performance timeline.

use ab_glyph::FontVec;
use tiny_skia::{Color, Path, PathBuilder, Pixmap, Rect, Transform};

use crate::compat_panel::CompatView;
use network_table::NetworkView;

mod network_table;
use crate::toolbar;
use devtools::network::NetworkLog;
use devtools::performance::{EventCategory, PerformanceLog};
use devtools::{Console, Level};

/// The dock's tabs, in header order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockTab {
    Elements,
    Compatibility,
    Network,
    Performance,
    Console,
}

impl DockTab {
    const ALL: [Self; 5] = [
        Self::Elements,
        Self::Compatibility,
        Self::Network,
        Self::Performance,
        Self::Console,
    ];

    /// Tab width in logical pixels; the Compatibility tab leaves room for
    /// its issue count.
    fn width(self) -> f32 {
        match self {
            Self::Elements => 84.0,
            Self::Compatibility => 146.0,
            Self::Network => 92.0,
            Self::Performance => 128.0,
            Self::Console => 92.0,
        }
    }
}

/// Parsed element hierarchy, the Compatibility view, and docked-panel
/// scrolling state. Each tab keeps its own scroll position.
pub(crate) struct ElementsPanel {
    rows: Vec<(usize, String)>,
    message: Option<&'static str>,
    offset: usize,
    visible: usize,
    wheel_remainder: f64,
    bounds: Option<Rect>,
    tab: DockTab,
    /// The Compatibility tab's content. The app fills it with audit results.
    pub(crate) compat: CompatView,
    pub(crate) network: NetworkLog,
    /// How the Network tab shows `network`: columns, sort, column menu.
    network_view: NetworkView,
    pub(crate) console: Console,
    pub(crate) performance: PerformanceLog,
}

impl ElementsPanel {
    /// Parses only the loaded HTML snapshot. No network requests or scripts run.
    pub(crate) fn new(html: Option<&str>) -> Self {
        let mut panel = Self {
            rows: Vec::new(),
            message: None,
            offset: 0,
            visible: 0,
            wheel_remainder: 0.0,
            bounds: None,
            tab: DockTab::Elements,
            compat: CompatView::new(),
            network: NetworkLog::new(),
            network_view: NetworkView::new(),
            console: Console::new(),
            performance: PerformanceLog::new(),
        };
        panel.parse_rows(html);
        panel
    }

    /// Shows a newly navigated-to document while the panel stays open.
    ///
    /// Rebuilds the Elements tree from `html` and returns the Compatibility
    /// tab to its not-yet-audited state, since both describe the old page.
    /// The selected tab, the Network table's columns and sort, and the
    /// panel's size are kept. Scrolling returns to the top. The caller
    /// supplies the new page's Network, Console, and Performance data.
    pub(crate) fn load_document(&mut self, html: Option<&str>) {
        self.rows.clear();
        self.message = None;
        self.offset = 0;
        self.wheel_remainder = 0.0;
        self.compat = CompatView::new();
        self.network_view.close_menu();
        // The next `draw` resizes, re-fitting rows around any new footer.
        self.parse_rows(html);
    }

    /// Fills the Elements rows from `html`, or sets the status message
    /// explaining why there are none.
    fn parse_rows(&mut self, html: Option<&str>) {
        let Some(html) = html else {
            self.message = Some("No loaded HTML available");
            return;
        };
        if html.len() > 1024 * 1024 {
            self.message = Some("HTML exceeds the basic inspector's 1 MiB limit");
            return;
        }
        let Ok(document) = std::panic::catch_unwind(|| html::parse_raw_html(html.to_owned()))
        else {
            self.message = Some("Could not parse loaded HTML");
            return;
        };
        let mut stack = vec![(document.root, 0)];
        while let Some((id, depth)) = stack.pop() {
            let Some(node) = document.node(id) else {
                continue;
            };
            let child_depth = if let html::NodeKind::Element(element) = node.get_kind() {
                self.rows.push((depth, format!("<{}>", element.get_name())));
                depth + 1
            } else {
                depth
            };
            if self.rows.len() >= 20_000 {
                self.message = Some("Element list limited to 20,000 entries");
                break;
            }
            stack.extend(
                node.get_children()
                    .iter()
                    .rev()
                    .map(|child| (*child, child_depth)),
            );
        }
        if self.rows.is_empty() {
            self.message = Some("No HTML elements found in the loaded snapshot");
        }
    }

    /// Whether the Elements tree lists a `<name>` element.
    #[cfg(test)]
    pub(crate) fn shows_element(&self, name: &str) -> bool {
        let tag = format!("<{name}>");
        self.rows.iter().any(|(_, row)| *row == tag)
    }

    /// Fits a bottom dock into the content area, leaving the upper page visible.
    pub(crate) fn resize(&mut self, width: f64, height: f64) {
        let available = (height - 60.0).max(0.0);
        let dock_height = (available * 0.5).min(320.0).round();
        self.bounds = Rect::from_xywh(
            INSET,
            (height - f64::from(INSET) - dock_height) as f32,
            (width - 2.0 * f64::from(INSET)).max(0.0) as f32,
            dock_height as f32,
        );
        let footer = if self.message.is_some() { FOOTER } else { 0.0 };
        let tree = dock_height - f64::from(HEADER + 2.0 * TREE_PAD + footer);
        self.visible = (tree / f64::from(ROW)).max(0.0) as usize;
        self.offset = self.offset.min(self.max_offset());
        let body = dock_height - f64::from(HEADER + 2.0 * TREE_PAD);
        self.compat
            .set_visible((body / f64::from(ROW)).max(0.0) as usize);
    }

    /// The tab currently shown.
    pub(crate) fn tab(&self) -> DockTab {
        self.tab
    }

    /// Shows `tab`.
    pub(crate) fn set_tab(&mut self, tab: DockTab) {
        self.tab = tab;
        self.network_view.close_menu();
        self.offset = self.offset.min(self.max_offset());
    }

    /// Handles a primary click inside the panel body (not on a tab or the
    /// close button). A click closes an open column menu, choosing the item
    /// under the pointer; on the Network tab a click on a column heading
    /// sorts by that column. Returns whether anything changed.
    pub(crate) fn click(&mut self, cursor: Option<(f64, f64)>) -> bool {
        if self.network_view.menu_open() {
            if let Some(column) = self.network_view.menu_item_at(cursor) {
                self.network_view.toggle(column);
            }
            self.network_view.close_menu();
            return true;
        }
        if self.tab == DockTab::Network
            && let Some(table) = self.network_table()
            && let Some(column) = self.network_view.heading_at(table, cursor)
        {
            self.network_view.click_heading(column);
            return true;
        }
        false
    }

    /// Handles a secondary (right) click. On the Network table it opens the
    /// column menu at the pointer; elsewhere it closes the menu. Returns
    /// whether anything changed.
    pub(crate) fn context_click(&mut self, cursor: Option<(f64, f64)>) -> bool {
        if self.tab == DockTab::Network
            && let (Some(table), Some(bounds), Some(point)) =
                (self.network_table(), self.bounds, cursor)
            && Self::in_rect(Some(table), cursor)
        {
            self.network_view.open_menu(point, bounds);
            return true;
        }
        self.network_view.close_menu()
    }

    /// Closes the column menu, if open, so Escape dismisses it before the
    /// panel. Returns whether it was open.
    pub(crate) fn dismiss_menu(&mut self) -> bool {
        self.network_view.close_menu()
    }

    /// The Network table's area: below the tab strip, inside the panel.
    fn network_table(&self) -> Option<Rect> {
        let bounds = self.bounds?;
        Rect::from_ltrb(
            bounds.left(),
            bounds.top() + HEADER,
            bounds.right(),
            bounds.bottom() - TREE_PAD,
        )
    }

    pub(crate) fn set_network(&mut self, network: NetworkLog) {
        self.network = network;
        self.offset = self.offset.min(self.max_offset());
    }

    pub(crate) fn set_console(&mut self, console: Console) {
        self.console = console;
        self.offset = self.offset.min(self.max_offset());
    }

    pub(crate) fn set_performance(&mut self, performance: PerformanceLog) {
        self.performance = performance;
        self.offset = self.offset.min(self.max_offset());
    }

    pub(crate) fn clear_active(&mut self) {
        match self.tab {
            DockTab::Network => self.network.clear(),
            DockTab::Console => self.console.clear(),
            DockTab::Performance => self.performance.clear(),
            DockTab::Elements | DockTab::Compatibility => {}
        }
        self.offset = 0;
    }

    /// Header tab rectangles in logical pixels.
    fn tab_rects(&self) -> impl Iterator<Item = (DockTab, Rect)> + '_ {
        let mut left = self.bounds.map_or(0.0, |bounds| bounds.left() + 6.0);
        DockTab::ALL.into_iter().filter_map(move |tab| {
            let bounds = self.bounds?;
            let rect =
                Rect::from_xywh(left, bounds.top(), tab.width(), HEADER.min(bounds.height()))?;
            left += tab.width();
            (rect.right() <= bounds.right()).then_some((tab, rect))
        })
    }

    /// The header tab under `cursor`, if any.
    pub(crate) fn tab_at(&self, cursor: Option<(f64, f64)>) -> Option<DockTab> {
        self.tab_rects()
            .find(|(_, rect)| Self::in_rect(Some(*rect), cursor))
            .map(|(tab, _)| tab)
    }

    fn tab_label(&self, tab: DockTab) -> String {
        match (tab, self.compat.issue_count()) {
            (DockTab::Elements, _) => "Elements".to_owned(),
            (DockTab::Compatibility, Some(count)) => format!("Compatibility ({count})"),
            (DockTab::Compatibility, None) => "Compatibility".to_owned(),
            (DockTab::Network, _) => format!("Network ({})", self.network.entries().len()),
            (DockTab::Performance, _) => {
                format!("Performance ({})", self.performance.entries().len())
            }
            (DockTab::Console, _) => format!("Console ({})", self.console.entries().len()),
        }
    }

    fn max_offset(&self) -> usize {
        self.active_row_count()
            .saturating_sub(self.page_rows().max(1))
    }

    /// Rows of the active tab that fit at once. The Performance timeline
    /// and the Network table give room to their heading rows.
    fn page_rows(&self) -> usize {
        match self.tab {
            DockTab::Performance => self.visible.saturating_sub(1),
            DockTab::Network => self.network_table().map_or(0, |table| {
                ((table.height() - network_table::HEADING) / ROW).max(0.0) as usize
            }),
            _ => self.visible,
        }
    }

    fn active_row_count(&self) -> usize {
        match self.tab {
            DockTab::Elements => self.rows.len(),
            DockTab::Compatibility => 0,
            DockTab::Network => self.network.entries().len(),
            DockTab::Performance => self.performance.entries().len(),
            DockTab::Console => self.console.entries().len(),
        }
    }

    /// Scrolls the active tab by rows (fractional wheel deltas are
    /// accumulated).
    pub(crate) fn scroll(&mut self, rows: f64) {
        if self.tab == DockTab::Compatibility {
            self.compat.scroll(rows);
            return;
        }
        self.wheel_remainder += rows;
        let whole = self.wheel_remainder.trunc() as isize;
        self.wheel_remainder -= whole as f64;
        self.offset = self
            .offset
            .saturating_add_signed(whole)
            .min(self.max_offset());
    }

    /// Handles navigation keys while the inspector has keyboard focus.
    pub(crate) fn key(&mut self, key: &winit::keyboard::NamedKey) {
        use winit::keyboard::NamedKey;
        if self.tab == DockTab::Compatibility {
            self.compat.key(key);
            return;
        }
        self.wheel_remainder = 0.0;
        match key {
            NamedKey::ArrowDown => self.scroll(1.0),
            NamedKey::ArrowUp => self.scroll(-1.0),
            NamedKey::PageDown => self.scroll(self.page_rows().max(1) as f64),
            NamedKey::PageUp => self.scroll(-(self.page_rows().max(1) as f64)),
            NamedKey::Home => self.offset = 0,
            NamedKey::End => self.offset = self.max_offset(),
            _ => {}
        }
    }

    fn close_rect(&self) -> Option<Rect> {
        let bounds = self.bounds?;
        if bounds.width() < 44.0 || bounds.height() < 30.0 {
            return None;
        }
        let size = 24.0;
        Rect::from_xywh(
            bounds.right() - 8.0 - size,
            bounds.top() + (HEADER - size) / 2.0,
            size,
            size,
        )
    }

    fn in_rect(rect: Option<Rect>, cursor: Option<(f64, f64)>) -> bool {
        let (Some(rect), Some((x, y))) = (rect, cursor) else {
            return false;
        };
        x >= f64::from(rect.left())
            && x < f64::from(rect.right())
            && y >= f64::from(rect.top())
            && y < f64::from(rect.bottom())
    }

    /// Whether the pointer belongs to the panel rather than the webpage below.
    pub(crate) fn contains(&self, cursor: Option<(f64, f64)>) -> bool {
        Self::in_rect(self.bounds, cursor)
    }

    /// Hit-tests the Close (×) control, in logical pixels.
    pub(crate) fn close_at(&self, cursor: Option<(f64, f64)>) -> bool {
        Self::in_rect(self.close_rect(), cursor)
    }

    /// Index into the visible rows under `cursor`, used for hover feedback.
    fn row_at(&self, cursor: Option<(f64, f64)>) -> Option<usize> {
        let bounds = self.bounds?;
        let (x, y) = cursor?;
        if !self.contains(cursor) || x >= f64::from(bounds.right() - SCROLLBAR_GUTTER) {
            return None;
        }
        let top = f64::from(bounds.top() + HEADER + TREE_PAD);
        let index = ((y - top) / f64::from(ROW)).floor();
        (index >= 0.0
            && (index as usize)
                < self
                    .visible
                    .min(self.rows.len().saturating_sub(self.offset)))
        .then_some(index as usize)
    }

    /// Draws only the bottom panel; page pixels above its bounds are preserved.
    ///
    /// `cursor` is in logical pixels and drives hover feedback. The header uses
    /// `ui_font`; the element tree uses `mono_font`, falling back to `ui_font`.
    pub(crate) fn draw(
        &mut self,
        frame: &mut Pixmap,
        scale: f64,
        dark: bool,
        cursor: Option<(f64, f64)>,
        ui_font: Option<&FontVec>,
        mono_font: Option<&FontVec>,
    ) {
        self.resize(
            f64::from(frame.width()) / scale,
            f64::from(frame.height()) / scale,
        );
        let Some(bounds) = self.bounds else {
            return;
        };
        let palette = toolbar::palette(dark);
        let colors = Colors::new(dark);
        let s = scale as f32;
        let transform = Transform::from_scale(s, s);

        // Body: square top edge (docked against the page), bottom corners
        // rounded to match the page card they sit inside.
        if let Some(body) = docked_rect(bounds, toolbar::PAGE_RADIUS) {
            toolbar::fill(frame, &body, colors.background, transform);
        }
        let header_height = HEADER.min(bounds.height());
        if let Some(header) =
            Rect::from_xywh(bounds.left(), bounds.top(), bounds.width(), header_height)
        {
            toolbar::fill(
                frame,
                &PathBuilder::from_rect(header),
                colors.header,
                transform,
            );
        }
        hairline(
            frame,
            bounds.left(),
            bounds.right(),
            bounds.top(),
            colors.border,
            transform,
        );
        if bounds.height() > HEADER {
            hairline(
                frame,
                bounds.left(),
                bounds.right(),
                bounds.top() + HEADER - 1.0,
                colors.separator,
                transform,
            );
        }

        if let Some(close) = self.close_rect() {
            let hovered = Self::in_rect(Some(close), cursor);
            if hovered
                && let Some(path) =
                    toolbar::rounded_rect(close.x(), close.y(), close.width(), close.height(), 6.0)
            {
                toolbar::fill(frame, &path, palette.hover, transform);
            }
            let (cx, cy) = (
                close.x() + close.width() / 2.0,
                close.y() + close.height() / 2.0,
            );
            let mut cross = PathBuilder::new();
            cross.move_to(cx - 4.0, cy - 4.0);
            cross.line_to(cx + 4.0, cy + 4.0);
            cross.move_to(cx + 4.0, cy - 4.0);
            cross.line_to(cx - 4.0, cy + 4.0);
            if let Some(path) = cross.finish() {
                let color = if hovered {
                    palette.text
                } else {
                    palette.text_muted
                };
                toolbar::stroke(frame, &path, color, 1.5, transform);
            }
        }

        if self.tab == DockTab::Compatibility {
            if let Some(body) = Rect::from_ltrb(
                bounds.left(),
                bounds.top() + HEADER + TREE_PAD,
                bounds.right(),
                bounds.bottom() - TREE_PAD,
            ) {
                self.compat
                    .draw(frame, body, scale, dark, cursor, ui_font, mono_font);
            }
            if let Some(ui_font) = ui_font.or(mono_font) {
                self.draw_header(frame, s, dark, cursor, ui_font);
            }
            return;
        }

        // Every tab draws its header strip, headings, and tables in the UI
        // font; only code-like content (tags, console output) is monospace.
        let header_font = ui_font.or(mono_font);
        if self.tab == DockTab::Performance {
            let body_top = bounds.top() + HEADER + TREE_PAD;
            self.draw_performance(frame, s, palette, colors, bounds, body_top, header_font);
            if let Some(font) = header_font {
                self.draw_header(frame, s, dark, cursor, font);
            }
            return;
        }

        if self.tab == DockTab::Network {
            if let Some(table) = self.network_table() {
                self.network_view.draw(
                    frame,
                    s,
                    &palette,
                    &colors,
                    table,
                    &self.network,
                    self.offset,
                    self.page_rows(),
                    cursor,
                    header_font,
                );
                self.draw_scrollbar(
                    frame,
                    s,
                    &colors,
                    bounds,
                    table.top() + network_table::HEADING,
                );
            }
            if let Some(font) = header_font {
                self.draw_header(frame, s, dark, cursor, font);
            }
            self.network_view
                .draw_menu(frame, s, dark, cursor, header_font);
            return;
        }

        let tree_top = bounds.top() + HEADER + TREE_PAD;
        let footer = if self.message.is_some() && self.tab == DockTab::Elements {
            FOOTER
        } else {
            0.0
        };
        let tree_bottom = bounds.bottom() - footer;
        let hovered_row = self.row_at(cursor);
        let rows_shown = self
            .visible
            .min(self.active_row_count().saturating_sub(self.offset));
        let indent_x = |depth: usize| {
            bounds.left() + 14.0 + (depth as f32 * INDENT).min((bounds.width() - 120.0).max(0.0))
        };

        if let Some(row) = hovered_row
            && let Some(rect) = Rect::from_xywh(
                bounds.left(),
                tree_top + row as f32 * ROW,
                bounds.width(),
                ROW,
            )
        {
            toolbar::fill(
                frame,
                &PathBuilder::from_rect(rect),
                colors.row_hover,
                transform,
            );
        }

        if self.tab == DockTab::Console {
            if let Some(font) = mono_font.or(ui_font) {
                self.draw_console(
                    frame,
                    s,
                    palette,
                    colors,
                    bounds,
                    tree_top,
                    tree_bottom,
                    rows_shown,
                    font,
                );
            }
            if let Some(font) = header_font {
                self.draw_header(frame, s, dark, cursor, font);
            }
            return;
        }

        // Indent guides connect each element to its ancestors' columns.
        for (index, (depth, _)) in self
            .rows
            .iter()
            .skip(self.offset)
            .take(rows_shown)
            .enumerate()
        {
            let top = tree_top + index as f32 * ROW;
            for level in 0..*depth {
                let x = indent_x(level) + 4.0;
                if let Some(rect) = Rect::from_xywh(x, top, 1.0, ROW) {
                    toolbar::fill(
                        frame,
                        &PathBuilder::from_rect(rect),
                        colors.guide,
                        transform,
                    );
                }
            }
        }

        // Scrollbar thumb, only when the tree overflows.
        if self.rows.len() > self.visible && self.visible > 0 {
            let track_top = tree_top;
            let track_height = (tree_bottom - TREE_PAD - track_top).max(0.0);
            let thumb_height =
                (track_height * self.visible as f32 / self.rows.len() as f32).max(24.0);
            let progress = self.offset as f32 / self.max_offset().max(1) as f32;
            let thumb_top = track_top + (track_height - thumb_height).max(0.0) * progress;
            if let Some(path) = toolbar::rounded_rect(
                bounds.right() - 9.0,
                thumb_top,
                5.0,
                thumb_height.min(track_height),
                2.5,
            ) {
                toolbar::fill(frame, &path, colors.scrollbar, transform);
            }
        }

        if self.message.is_some() && bounds.height() >= HEADER + footer {
            hairline(
                frame,
                bounds.left(),
                bounds.right(),
                tree_bottom,
                colors.separator,
                transform,
            );
        }

        let Some(ui_font) = ui_font.or(mono_font) else {
            return;
        };
        let mono_font = mono_font.unwrap_or(ui_font);
        let device = |value: f32| value * s;

        self.draw_header(frame, s, dark, cursor, ui_font);

        if let Some(message) = self.message
            && bounds.height() >= HEADER + footer
        {
            toolbar::draw_text(
                frame,
                ui_font,
                device(SMALL_SIZE),
                device(bounds.left() + 14.0),
                device(bounds.bottom() - FOOTER / 2.0 + 4.0),
                device(bounds.right() - 14.0),
                palette.text_muted,
                message,
            );
        }

        // Element rows: `<` and `>` in a muted tone, tag names highlighted.
        let clip = device(bounds.right() - SCROLLBAR_GUTTER);
        for (index, (depth, tag)) in self
            .rows
            .iter()
            .skip(self.offset)
            .take(rows_shown)
            .enumerate()
        {
            let baseline = tree_top + index as f32 * ROW + ROW / 2.0 + 4.0;
            let name = tag.trim_start_matches('<').trim_end_matches('>');
            let mut pen = device(indent_x(*depth) + 12.0);
            for (part, color) in [
                ("<", colors.punctuation),
                (name, colors.tag),
                (">", colors.punctuation),
            ] {
                pen = toolbar::draw_text(
                    frame,
                    mono_font,
                    device(TREE_SIZE),
                    pen,
                    device(baseline),
                    clip,
                    color,
                    part,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_console(
        &self,
        frame: &mut Pixmap,
        s: f32,
        palette: toolbar::Palette,
        colors: Colors,
        bounds: Rect,
        top: f32,
        bottom: f32,
        rows_shown: usize,
        font: &FontVec,
    ) {
        let device = |value: f32| value * s;
        let clip = device(bounds.right() - SCROLLBAR_GUTTER);
        for (index, entry) in self
            .console
            .entries()
            .iter()
            .skip(self.offset)
            .take(rows_shown)
            .enumerate()
        {
            let row_top = top + index as f32 * ROW + 13.0;
            let color = match entry.level {
                Level::Log => palette.text,
                Level::Warn => toolbar::rgba(0xd9, 0x77, 0x06, 255),
                Level::Error => toolbar::rgba(0xdc, 0x26, 0x26, 255),
            };
            let prefix = match entry.level {
                Level::Log => "log",
                Level::Warn => "warn",
                Level::Error => "error",
            };
            let text = format!("[{prefix}] {}", entry.message);
            toolbar::draw_text(
                frame,
                font,
                device(TREE_SIZE),
                device(bounds.left() + 14.0),
                device(row_top),
                clip,
                color,
                &text,
            );
        }
        if self.console.entries().len() > self.visible && self.visible > 0 {
            let track_height = (bottom - TREE_PAD - top).max(0.0);
            let thumb_height = (track_height * self.visible as f32
                / self.console.entries().len() as f32)
                .max(24.0);
            let progress = self.offset as f32 / self.max_offset().max(1) as f32;
            let thumb_top = top + (track_height - thumb_height).max(0.0) * progress;
            if let Some(path) = toolbar::rounded_rect(
                bounds.right() - 9.0,
                thumb_top,
                5.0,
                thumb_height.min(track_height),
                2.5,
            ) {
                toolbar::fill(frame, &path, colors.scrollbar, Transform::from_scale(s, s));
            }
        }
    }

    /// The Performance tab's header subtitle: event count, timeline span,
    /// and how to start over.
    fn performance_summary(&self) -> String {
        let events = self.performance.entries().len();
        if events == 0 {
            return "No events yet · load or resize a page".to_owned();
        }
        format!(
            "{events} events over {} · Delete clears",
            format_ms(self.performance.span().as_secs_f64() * 1000.0)
        )
    }

    /// Draws the Performance timeline below `top`: a time axis, then one row
    /// per event with its label, duration, and a bar placed on the shared
    /// axis and colored by category. `s` is the device scale.
    #[allow(clippy::too_many_arguments)]
    fn draw_performance(
        &self,
        frame: &mut Pixmap,
        s: f32,
        palette: toolbar::Palette,
        colors: Colors,
        bounds: Rect,
        top: f32,
        font: Option<&FontVec>,
    ) {
        let transform = Transform::from_scale(s, s);
        let device = |value: f32| value * s;
        // Bars and gridlines are drawn even when no font loaded; text is not.
        let text = |frame: &mut Pixmap,
                    size: f32,
                    left: f32,
                    baseline: f32,
                    right: f32,
                    color: Color,
                    content: &str| {
            if let Some(font) = font {
                toolbar::draw_text(
                    frame,
                    font,
                    device(size),
                    device(left),
                    device(baseline),
                    device(right),
                    color,
                    content,
                );
            }
        };
        let geometry = TimelineGeometry::new(bounds);
        let axis_baseline = top + 13.0;
        let rows_top = top + ROW;
        let rows_bottom = bounds.bottom() - TREE_PAD;
        let span_ms = self.performance.span().as_secs_f64() * 1000.0;

        // Column headings and the time axis with gridlines through the rows.
        for (heading, left, right) in [
            ("Event", geometry.label_left, geometry.duration_left),
            ("Time", geometry.duration_left, geometry.timeline_left),
        ] {
            text(
                frame,
                SMALL_SIZE,
                left,
                axis_baseline,
                right - 6.0,
                palette.text_muted,
                heading,
            );
        }
        let timeline_width = geometry.timeline_right - geometry.timeline_left;
        if timeline_width > 40.0 {
            for tick in 0..=AXIS_TICKS {
                let fraction = tick as f32 / AXIS_TICKS as f32;
                let x = geometry.timeline_left + timeline_width * fraction;
                if let Some(rect) = Rect::from_xywh(x, rows_top - 4.0, 1.0, rows_bottom - rows_top)
                {
                    toolbar::fill(
                        frame,
                        &PathBuilder::from_rect(rect),
                        colors.guide,
                        transform,
                    );
                }
                if span_ms <= 0.0 {
                    // An empty timeline has no scale to label.
                    continue;
                }
                let label = format_ms(span_ms * f64::from(fraction));
                let width = font.map_or(0.0, |font| {
                    toolbar::text_advance(font, device(SMALL_SIZE), &label) / s
                });
                // Keep the first and last labels inside the timeline.
                let left = (x - width / 2.0)
                    .max(geometry.timeline_left)
                    .min(geometry.timeline_right - width);
                text(
                    frame,
                    SMALL_SIZE,
                    left,
                    axis_baseline,
                    geometry.timeline_right,
                    palette.text_muted,
                    &label,
                );
            }
        }
        hairline(
            frame,
            bounds.left(),
            bounds.right(),
            rows_top - 4.0,
            colors.separator,
            transform,
        );

        let rows_shown = self
            .page_rows()
            .min(self.performance.entries().len().saturating_sub(self.offset));
        for (index, event) in self
            .performance
            .entries()
            .iter()
            .skip(self.offset)
            .take(rows_shown)
            .enumerate()
        {
            let row_top = rows_top + index as f32 * ROW;
            let baseline = row_top + ROW / 2.0 + 4.0;
            if index % 2 == 1
                && let Some(row) = Rect::from_xywh(bounds.left(), row_top, bounds.width(), ROW)
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(row),
                    colors.row_hover,
                    transform,
                );
            }
            let color = category_color(event.category);
            if let Some(path) = toolbar::rounded_rect(
                geometry.label_left,
                row_top + ROW / 2.0 - 4.0,
                8.0,
                8.0,
                2.0,
            ) {
                toolbar::fill(frame, &path, color, transform);
            }
            text(
                frame,
                TREE_SIZE,
                geometry.label_left + 14.0,
                baseline,
                geometry.duration_left - 8.0,
                palette.text,
                &event.label,
            );
            text(
                frame,
                TREE_SIZE,
                geometry.duration_left,
                baseline,
                geometry.timeline_left - 8.0,
                palette.text_muted,
                &format_ms(event.duration_ms()),
            );
            if let Some((left, width)) = geometry.bar(self.performance.bar_fraction(event))
                && let Some(path) =
                    toolbar::rounded_rect(left, row_top + 5.0, width, ROW - 10.0, 2.0)
            {
                toolbar::fill(frame, &path, color, transform);
            }
        }

        self.draw_scrollbar(frame, s, &colors, bounds, rows_top);
    }

    /// Draws the active tab's scrollbar thumb along the panel's right edge,
    /// over a track from `top` to the panel's bottom padding, when its rows
    /// overflow. `s` is the device scale.
    fn draw_scrollbar(&self, frame: &mut Pixmap, s: f32, colors: &Colors, bounds: Rect, top: f32) {
        let total = self.active_row_count();
        let page = self.page_rows();
        if total <= page || page == 0 {
            return;
        }
        let track_height = (bounds.bottom() - TREE_PAD - top).max(0.0);
        let thumb_height = (track_height * page as f32 / total as f32).max(24.0);
        let progress = self.offset as f32 / self.max_offset().max(1) as f32;
        let thumb_top = top + (track_height - thumb_height).max(0.0) * progress;
        if let Some(path) = toolbar::rounded_rect(
            bounds.right() - 9.0,
            thumb_top,
            5.0,
            thumb_height.min(track_height),
            2.5,
        ) {
            toolbar::fill(frame, &path, colors.scrollbar, Transform::from_scale(s, s));
        }
    }

    /// Draws the header text: the tabs (active one underlined), a muted
    /// description of the active tab, and a keyboard hint beside the close
    /// button. `s` is the device scale.
    fn draw_header(
        &self,
        frame: &mut Pixmap,
        s: f32,
        dark: bool,
        cursor: Option<(f64, f64)>,
        ui_font: &FontVec,
    ) {
        let Some(bounds) = self.bounds else {
            return;
        };
        if bounds.height() < HEADER {
            return;
        }
        let palette = toolbar::palette(dark);
        let transform = Transform::from_scale(s, s);
        let device = |value: f32| value * s;
        let baseline = bounds.top() + HEADER / 2.0 + 4.5;
        let content_right = self
            .close_rect()
            .map_or(bounds.right() - 8.0, |close| close.left() - 8.0);
        let mut subtitle_left = bounds.left() + 14.0;
        for (tab, rect) in self.tab_rects() {
            let label = self.tab_label(tab);
            let active = tab == self.tab;
            if !active
                && Self::in_rect(Some(rect), cursor)
                && let Some(path) = toolbar::rounded_rect(
                    rect.x() + 2.0,
                    rect.y() + 5.0,
                    rect.width() - 4.0,
                    rect.height() - 10.0,
                    6.0,
                )
            {
                toolbar::fill(frame, &path, palette.hover, transform);
            }
            let width = toolbar::text_advance(ui_font, device(TITLE_SIZE), &label) / s;
            let left = rect.left() + ((rect.width() - width) / 2.0).max(4.0);
            toolbar::draw_text(
                frame,
                ui_font,
                device(TITLE_SIZE),
                device(left),
                device(baseline),
                device(rect.right().min(content_right)),
                if active {
                    palette.text
                } else {
                    palette.text_muted
                },
                &label,
            );
            if active
                && let Some(underline) =
                    Rect::from_xywh(left - 4.0, bounds.top() + HEADER - 2.0, width + 8.0, 2.0)
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(underline),
                    palette.accent,
                    transform,
                );
            }
            subtitle_left = rect.right() + 12.0;
        }
        let hint = "Esc to close";
        let hint_width = toolbar::text_advance(ui_font, device(SMALL_SIZE), hint) / s;
        let hint_left = content_right - 4.0 - hint_width;
        let subtitle = match self.tab {
            DockTab::Elements => "HTML snapshot · read-only".to_owned(),
            DockTab::Compatibility => self.compat.summary(),
            DockTab::Network => "Completed resource requests · session log".to_owned(),
            DockTab::Performance => self.performance_summary(),
            DockTab::Console => "JavaScript messages · session log".to_owned(),
        };
        let subtitle_width = toolbar::text_advance(ui_font, device(SMALL_SIZE), &subtitle) / s;
        let show_hint = hint_left > subtitle_left + subtitle_width + 16.0;
        if subtitle_left < content_right {
            toolbar::draw_text(
                frame,
                ui_font,
                device(SMALL_SIZE),
                device(subtitle_left),
                device(baseline),
                device(if show_hint {
                    hint_left - 16.0
                } else {
                    content_right
                }),
                palette.text_muted,
                &subtitle,
            );
        }
        if show_hint {
            toolbar::draw_text(
                frame,
                ui_font,
                device(SMALL_SIZE),
                device(hint_left),
                device(baseline),
                device(content_right),
                palette.text_muted,
                hint,
            );
        }
    }
}

/// Column positions of the Performance timeline, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TimelineGeometry {
    /// Left edge of the category swatch and event label.
    label_left: f32,
    /// Left edge of the duration column.
    duration_left: f32,
    /// Left edge of the bar area, where time zero is drawn.
    timeline_left: f32,
    /// Right edge of the bar area, where the end of the span is drawn.
    timeline_right: f32,
}

impl TimelineGeometry {
    /// Splits the panel `bounds` into label, duration, and bar columns. The
    /// label column takes a third of the width, capped so the bars keep
    /// most of a wide panel.
    fn new(bounds: Rect) -> Self {
        let label_left = bounds.left() + 14.0;
        let label_width = (bounds.width() * 0.33).clamp(120.0, 300.0);
        let duration_left = label_left + label_width;
        let timeline_left = duration_left + 76.0;
        let timeline_right = (bounds.right() - SCROLLBAR_GUTTER - 8.0).max(timeline_left);
        Self {
            label_left,
            duration_left,
            timeline_left,
            timeline_right,
        }
    }

    /// The `(left, width)` of a bar at `(start, width)` fractions of the
    /// span. Bars are at least [`MIN_BAR`] wide so instant events stay
    /// visible, and never extend past the bar area. `None` when the area
    /// has no room.
    fn bar(&self, (start, width): (f64, f64)) -> Option<(f32, f32)> {
        let area = self.timeline_right - self.timeline_left;
        if area < MIN_BAR {
            return None;
        }
        let width = (width as f32 * area).max(MIN_BAR);
        let left = (self.timeline_left + start as f32 * area).min(self.timeline_right - width);
        Some((left, width))
    }
}

/// Formats fractional milliseconds for the timeline: tenths below 100 ms,
/// whole milliseconds below 10 s, then seconds.
fn format_ms(ms: f64) -> String {
    if ms < 100.0 {
        format!("{ms:.1} ms")
    } else if ms < 10_000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

/// The bar and swatch color for `category`. The hues are spread apart so
/// they read in both the light and dark themes.
fn category_color(category: EventCategory) -> Color {
    let rgba = toolbar::rgba;
    match category {
        EventCategory::Network => rgba(0x3b, 0x82, 0xf6, 255),
        EventCategory::Parse => rgba(0x14, 0xb8, 0xa6, 255),
        EventCategory::Style => rgba(0xa8, 0x55, 0xf7, 255),
        EventCategory::Layout => rgba(0xec, 0x48, 0x99, 255),
        EventCategory::Paint => rgba(0x22, 0xc5, 0x5e, 255),
        EventCategory::Script => rgba(0xf5, 0x9e, 0x0b, 255),
    }
}

/// Gap in logical pixels between the panel and the window edges, matching the
/// page card's inset.
const INSET: f32 = 8.0;
/// Header strip height in logical pixels.
const HEADER: f32 = 36.0;
/// Element row height in logical pixels.
const ROW: f32 = 20.0;
/// Horizontal step per tree depth in logical pixels.
const INDENT: f32 = 16.0;
/// Vertical padding above and below the tree in logical pixels.
const TREE_PAD: f32 = 6.0;
/// Status footer height in logical pixels, shown only with a message.
const FOOTER: f32 = 26.0;
/// Width reserved on the right for the scrollbar, in logical pixels.
const SCROLLBAR_GUTTER: f32 = 14.0;
const TITLE_SIZE: f32 = 13.0;
const SMALL_SIZE: f32 = 12.0;
const TREE_SIZE: f32 = 12.5;
/// Intervals the Performance time axis is divided into.
const AXIS_TICKS: u32 = 4;
/// Narrowest Performance bar in logical pixels, so instant events show.
const MIN_BAR: f32 = 2.0;

/// Panel-specific colors layered on top of the shared chrome palette.
struct Colors {
    background: Color,
    header: Color,
    border: Color,
    separator: Color,
    row_hover: Color,
    guide: Color,
    scrollbar: Color,
    tag: Color,
    punctuation: Color,
}

impl Colors {
    fn new(dark: bool) -> Self {
        let rgba = toolbar::rgba;
        if dark {
            Self {
                background: rgba(0x1c, 0x1d, 0x21, 255),
                header: rgba(0x25, 0x26, 0x2b, 255),
                border: rgba(0x3a, 0x3c, 0x43, 255),
                separator: rgba(255, 255, 255, 18),
                row_hover: rgba(255, 255, 255, 12),
                guide: rgba(255, 255, 255, 16),
                scrollbar: rgba(255, 255, 255, 60),
                tag: rgba(0x6c, 0xb8, 0xf0, 255),
                punctuation: rgba(0x7d, 0x85, 0x92, 255),
            }
        } else {
            Self {
                background: rgba(255, 255, 255, 255),
                header: rgba(0xf6, 0xf7, 0xf9, 255),
                border: rgba(0xd6, 0xd9, 0xdf, 255),
                separator: rgba(15, 23, 42, 22),
                row_hover: rgba(0x00, 0x6e, 0xb6, 16),
                guide: rgba(15, 23, 42, 18),
                scrollbar: rgba(15, 23, 42, 70),
                tag: rgba(0x88, 0x12, 0x80, 255),
                punctuation: rgba(0x8a, 0x90, 0x9a, 255),
            }
        }
    }
}

/// A rectangle with square top corners and `radius`-rounded bottom corners.
fn docked_rect(rect: Rect, radius: f32) -> Option<Path> {
    let (x, y, w, h) = (rect.x(), rect.y(), rect.width(), rect.height());
    let r = radius.min(w / 2.0).min(h);
    let k = r * 0.4477;
    let mut builder = PathBuilder::new();
    builder.move_to(x, y);
    builder.line_to(x + w, y);
    builder.line_to(x + w, y + h - r);
    builder.cubic_to(x + w, y + h - k, x + w - k, y + h, x + w - r, y + h);
    builder.line_to(x + r, y + h);
    builder.cubic_to(x + k, y + h, x, y + h - k, x, y + h - r);
    builder.close();
    builder.finish()
}

/// Fills a one-logical-pixel horizontal rule whose top edge is `y`.
fn hairline(frame: &mut Pixmap, left: f32, right: f32, y: f32, color: Color, transform: Transform) {
    if let Some(rect) = Rect::from_ltrb(left, y, right, y + 1.0) {
        toolbar::fill(frame, &PathBuilder::from_rect(rect), color, transform);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_element_relationships_without_inventing_wrappers() {
        let panel = ElementsPanel::new(Some(
            "<html><head><title>T</title></head><body><main><p>Hello</p></main></body></html>",
        ));
        assert_eq!(
            panel.rows,
            [
                (0, "<html>"),
                (1, "<head>"),
                (2, "<title>"),
                (1, "<body>"),
                (2, "<main>"),
                (3, "<p>")
            ]
            .map(|(d, s)| (d, s.to_owned()))
        );
        let fragment = ElementsPanel::new(Some("<p>Only this element</p>"));
        assert_eq!(fragment.rows, [(0, "<p>".to_owned())]);
        assert!(ElementsPanel::new(None).message.is_some());
        assert!(ElementsPanel::new(Some("")).rows.is_empty());
    }

    #[test]
    fn scrolling_and_resizing_keep_the_panel_and_close_button_visible() {
        let mut panel = ElementsPanel::new(Some(&"<p>row</p>".repeat(100)));
        for (width, height) in [(800.0, 600.0), (420.0, 280.0), (1200.0, 900.0)] {
            panel.resize(width, height);
            let bounds = panel.bounds.unwrap();
            assert!(bounds.top() > 60.0 && bounds.bottom() <= height as f32);
            assert!(bounds.right() <= width as f32);
            let close = panel.close_rect().unwrap();
            assert!(panel.close_at(Some((
                f64::from(close.left() + 1.0),
                f64::from(close.top() + 1.0)
            ))));
            panel.key(&winit::keyboard::NamedKey::End);
            assert_eq!(panel.offset, panel.max_offset());
            panel.key(&winit::keyboard::NamedKey::Home);
            for _ in 0..4 {
                panel.scroll(0.25);
            }
            assert_eq!(panel.offset, 1);
        }
    }

    fn performance_panel(events: u64) -> ElementsPanel {
        use devtools::performance::TimelineEvent;
        use std::time::{Duration, Instant};
        let origin = Instant::now();
        let mut log = PerformanceLog::new();
        for index in 0..events {
            log.record(TimelineEvent::new(
                EventCategory::ALL[index as usize % EventCategory::ALL.len()],
                format!("event {index}"),
                origin + Duration::from_millis(index * 10),
                Duration::from_millis(10),
            ));
        }
        let mut panel = ElementsPanel::new(Some("<p>x</p>"));
        panel.set_tab(DockTab::Performance);
        panel.set_performance(log);
        panel
    }

    #[test]
    fn performance_tab_scrolls_to_its_last_event_below_the_time_axis() {
        let mut panel = performance_panel(50);
        panel.resize(800.0, 600.0);
        assert!(panel.visible > 1);
        panel.key(&winit::keyboard::NamedKey::End);
        assert_eq!(panel.offset + panel.page_rows(), 50);
        panel.clear_active();
        assert!(panel.performance.entries().is_empty());
        assert_eq!(panel.offset, 0);
        assert_eq!(panel.tab_label(DockTab::Performance), "Performance (0)");
    }

    #[test]
    fn timeline_bars_follow_the_span_and_stay_inside_the_bar_area() {
        let geometry = TimelineGeometry::new(Rect::from_xywh(0.0, 0.0, 900.0, 300.0).unwrap());
        assert!(geometry.label_left < geometry.duration_left);
        assert!(geometry.duration_left < geometry.timeline_left);
        assert!(geometry.timeline_left < geometry.timeline_right);
        let area = geometry.timeline_right - geometry.timeline_left;
        assert_eq!(
            geometry.bar((0.0, 1.0)),
            Some((geometry.timeline_left, area))
        );
        let (left, width) = geometry.bar((0.5, 0.25)).unwrap();
        assert!((left - (geometry.timeline_left + area / 2.0)).abs() < 0.01);
        assert!((width - area / 4.0).abs() < 0.01);
        // A zero-length event at the very end is still drawn, inside the area.
        let (left, width) = geometry.bar((1.0, 0.0)).unwrap();
        assert_eq!(width, MIN_BAR);
        assert!(left + width <= geometry.timeline_right);
        assert_eq!(format_ms(3.25), "3.2 ms");
        assert_eq!(format_ms(320.4), "320 ms");
        assert_eq!(format_ms(12_500.0), "12.5 s");
    }

    #[test]
    fn performance_tab_draws_timeline_rows() {
        let mut frame = Pixmap::new(800, 600).unwrap();
        let before = frame.clone();
        let mut panel = performance_panel(3);
        panel.draw(&mut frame, 1.0, true, None, None, None);
        let bounds = panel.bounds.unwrap();
        let geometry = TimelineGeometry::new(bounds);
        // The first event spans the first third of the timeline; sample the
        // middle of its bar in the first row.
        let (left, width) = geometry
            .bar(
                panel
                    .performance
                    .bar_fraction(&panel.performance.entries()[0]),
            )
            .unwrap();
        let x = (left + width / 2.0) as u32;
        let y = (bounds.top() + HEADER + TREE_PAD + ROW + ROW / 2.0) as u32;
        let pixel = frame.pixel(x, y).unwrap();
        let network = category_color(EventCategory::Network).to_color_u8();
        assert_eq!(
            (pixel.red(), pixel.green(), pixel.blue()),
            (network.red(), network.green(), network.blue())
        );
        assert_ne!(frame.data(), before.data());
    }

    fn network_panel() -> ElementsPanel {
        use devtools::network::{RequestEntry, ResourceType};
        let mut log = NetworkLog::new();
        log.record(RequestEntry::new(
            "https://a.test/z.js",
            Some(200),
            ResourceType::Script,
            9,
        ));
        log.record(RequestEntry::new(
            "https://a.test/",
            Some(200),
            ResourceType::Document,
            3,
        ));
        let mut panel = ElementsPanel::new(Some("<p>x</p>"));
        panel.set_network(log);
        panel.set_tab(DockTab::Network);
        panel.resize(800.0, 600.0);
        panel
    }

    #[test]
    fn clicking_a_network_heading_sorts_and_right_click_offers_columns() {
        use network_table::{NetworkColumn, SortDirection};
        let mut panel = network_panel();
        let table = panel.network_table().unwrap();
        let name_heading = Some((40.0, f64::from(table.top() + 6.0)));
        assert!(panel.click(name_heading));
        assert_eq!(
            panel.network_view.sort(),
            Some((NetworkColumn::Name, SortDirection::Ascending))
        );
        assert_eq!(panel.network_view.order(&panel.network), [1, 0]);
        let first_row = Some((40.0, f64::from(table.top() + network_table::HEADING + 5.0)));
        assert!(!panel.click(first_row), "rows do not change the sort");

        // Right-click opens the column menu; choosing Type hides it.
        assert!(panel.context_click(first_row));
        let type_item = (0..400)
            .map(|y| Some((50.0, f64::from(table.top()) + f64::from(y) / 2.0)))
            .find(|cursor| panel.network_view.menu_item_at(*cursor) == Some(NetworkColumn::Type))
            .expect("the menu lists Type");
        assert!(panel.click(type_item));
        assert!(!panel.network_view.is_visible(NetworkColumn::Type));
        assert!(!panel.network_view.menu_open());

        // Escape and tab switches close the menu; other tabs have none.
        panel.context_click(first_row);
        assert!(panel.dismiss_menu());
        panel.context_click(first_row);
        panel.set_tab(DockTab::Console);
        assert!(!panel.network_view.menu_open());
        assert!(!panel.context_click(first_row));
    }

    #[test]
    fn network_rows_start_below_the_heading_row() {
        let panel = network_panel();
        let table = panel.network_table().unwrap();
        let rows_top = table.top() + network_table::HEADING;
        assert!(rows_top >= table.top() + ROW, "the heading has its own row");
        let last_row_bottom = rows_top + panel.page_rows() as f32 * ROW;
        assert!(last_row_bottom <= table.bottom());
    }

    #[test]
    fn every_tab_draws_its_header_strip_in_the_ui_font() {
        // The strip must not change when only the monospace font changes.
        let ui = crate::toolbar::load_ui_font([
            "/System/Library/Fonts/SFNS.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "C:/Windows/Fonts/segoeui.ttf",
        ]);
        let mono = crate::page_source::load_font();
        let (Some(ui), Some(mono)) = (ui, mono) else {
            eprintln!("skipping: system fonts unavailable");
            return;
        };
        for tab in DockTab::ALL {
            let strip = |mono_font: &FontVec| {
                let mut frame = Pixmap::new(900, 600).unwrap();
                let mut panel = network_panel();
                panel.set_tab(tab);
                panel.draw(&mut frame, 1.0, true, None, Some(&ui), Some(mono_font));
                let bounds = panel.bounds.unwrap();
                let (top, bottom) = (bounds.top() as usize, (bounds.top() + HEADER) as usize);
                frame.data()[top * 900 * 4..bottom * 900 * 4].to_vec()
            };
            assert!(
                strip(&mono) == strip(&ui),
                "{tab:?} header uses the mono font"
            );
        }
    }

    #[test]
    fn loading_a_new_document_keeps_the_tab_and_network_view() {
        use network_table::NetworkColumn;
        let mut panel = network_panel();
        panel.network_view.click_heading(NetworkColumn::Time);
        panel.network_view.toggle(NetworkColumn::Type);
        panel.offset = 1;
        panel.load_document(Some("<main><section></section></main>"));
        assert_eq!(panel.tab(), DockTab::Network);
        assert!(panel.network_view.sort().is_some());
        assert!(!panel.network_view.is_visible(NetworkColumn::Type));
        assert_eq!(panel.offset, 0);
        assert!(panel.shows_element("section"));
        assert!(!panel.shows_element("p"));
        assert_eq!(
            *panel.compat.state(),
            crate::compat_panel::CompatState::Idle
        );
        panel.load_document(None);
        assert!(panel.rows.is_empty() && panel.message.is_some());
    }

    #[test]
    fn drawing_leaves_upper_webpage_pixels_untouched_at_each_scale() {
        for scale in [1.0, 2.0] {
            let mut frame = Pixmap::new((800.0 * scale) as u32, (600.0 * scale) as u32).unwrap();
            frame.fill(Color::from_rgba8(180, 20, 40, 255));
            let before = frame.clone();
            let mut panel = ElementsPanel::new(Some("<body><p>Test</p></body>"));
            panel.draw(&mut frame, scale, false, None, None, None);
            let top = (f64::from(panel.bounds.unwrap().top()) * scale) as usize;
            let preserved = top * frame.width() as usize * 4;
            assert_eq!(&frame.data()[..preserved], &before.data()[..preserved]);
            assert_ne!(frame.data(), before.data());
        }
    }
}

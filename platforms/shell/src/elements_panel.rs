//! The bottom DevTools dock with two tabs: a small read-only Elements tree
//! derived from loaded HTML (not the live JS DOM), and the Compatibility
//! audit from [`crate::compat_panel`].

use ab_glyph::FontVec;
use tiny_skia::{Color, Path, PathBuilder, Pixmap, Rect, Transform};

use crate::compat_panel::CompatView;
use crate::toolbar;
use devtools::network::{NetworkLog, ResourceType};
use devtools::{Console, Level};

/// The dock's tabs, in header order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockTab {
    Elements,
    Compatibility,
    Network,
    Console,
}

impl DockTab {
    const ALL: [Self; 4] = [
        Self::Elements,
        Self::Compatibility,
        Self::Network,
        Self::Console,
    ];

    /// Tab width in logical pixels; the Compatibility tab leaves room for
    /// its issue count.
    fn width(self) -> f32 {
        match self {
            Self::Elements => 84.0,
            Self::Compatibility => 146.0,
            Self::Network => 92.0,
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
    pub(crate) console: Console,
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
            console: Console::new(),
        };
        let Some(html) = html else {
            panel.message = Some("No loaded HTML available");
            return panel;
        };
        if html.len() > 1024 * 1024 {
            panel.message = Some("HTML exceeds the basic inspector's 1 MiB limit");
            return panel;
        }
        let Ok(document) = std::panic::catch_unwind(|| html::parse_raw_html(html.to_owned()))
        else {
            panel.message = Some("Could not parse loaded HTML");
            return panel;
        };
        let mut stack = vec![(document.root, 0)];
        while let Some((id, depth)) = stack.pop() {
            let Some(node) = document.node(id) else {
                continue;
            };
            let child_depth = if let html::NodeKind::Element(element) = node.get_kind() {
                panel
                    .rows
                    .push((depth, format!("<{}>", element.get_name())));
                depth + 1
            } else {
                depth
            };
            if panel.rows.len() >= 20_000 {
                panel.message = Some("Element list limited to 20,000 entries");
                break;
            }
            stack.extend(
                node.get_children()
                    .iter()
                    .rev()
                    .map(|child| (*child, child_depth)),
            );
        }
        if panel.rows.is_empty() {
            panel.message = Some("No HTML elements found in the loaded snapshot");
        }
        panel
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
    }

    pub(crate) fn set_network(&mut self, network: NetworkLog) {
        self.network = network;
        self.offset = self.offset.min(self.max_offset());
    }

    pub(crate) fn set_console(&mut self, console: Console) {
        self.console = console;
        self.offset = self.offset.min(self.max_offset());
    }

    pub(crate) fn clear_active(&mut self) {
        match self.tab {
            DockTab::Network => self.network.clear(),
            DockTab::Console => self.console.clear(),
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
            (DockTab::Console, _) => format!("Console ({})", self.console.entries().len()),
        }
    }

    fn max_offset(&self) -> usize {
        self.active_row_count().saturating_sub(self.visible.max(1))
    }

    fn active_row_count(&self) -> usize {
        match self.tab {
            DockTab::Elements => self.rows.len(),
            DockTab::Compatibility => 0,
            DockTab::Network => self.network.entries().len(),
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
            NamedKey::PageDown => self.scroll(self.visible.max(1) as f64),
            NamedKey::PageUp => self.scroll(-(self.visible.max(1) as f64)),
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

        if matches!(self.tab, DockTab::Network | DockTab::Console) {
            let Some(network_font) = mono_font.or(ui_font) else {
                return;
            };
            if self.tab == DockTab::Network {
                self.draw_network(
                    frame,
                    s,
                    dark,
                    palette,
                    colors,
                    bounds,
                    tree_top,
                    tree_bottom,
                    rows_shown,
                    network_font,
                );
            } else {
                self.draw_console(
                    frame,
                    s,
                    dark,
                    palette,
                    colors,
                    bounds,
                    tree_top,
                    tree_bottom,
                    rows_shown,
                    network_font,
                );
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

    fn draw_network(
        &self,
        frame: &mut Pixmap,
        s: f32,
        dark: bool,
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
        let header = "Name                                      Status  Type       Time";
        toolbar::draw_text(
            frame,
            font,
            device(TREE_SIZE),
            device(bounds.left() + 14.0),
            device(top + 13.0),
            clip,
            palette.text_muted,
            header,
        );
        for (index, entry) in self
            .network
            .entries()
            .iter()
            .skip(self.offset)
            .take(rows_shown)
            .enumerate()
        {
            let row_top = top + 20.0 + index as f32 * ROW;
            let Some(row) = Rect::from_xywh(bounds.left(), row_top - 15.0, bounds.width(), ROW)
            else {
                continue;
            };
            if index % 2 == 1 {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(row),
                    colors.row_hover,
                    Transform::from_scale(s, s),
                );
            }
            let status = entry
                .status
                .map_or("pending".to_owned(), |status| status.to_string());
            let kind = match entry.resource_type {
                ResourceType::Document => "document",
                ResourceType::Stylesheet => "stylesheet",
                ResourceType::Script => "script",
                ResourceType::Image => "image",
                ResourceType::Other => "other",
            };
            let text = format!(
                "{:<42} {:>7}  {:<10} {:>4} ms",
                entry.url, status, kind, entry.duration_ms
            );
            toolbar::draw_text(
                frame,
                font,
                device(TREE_SIZE),
                device(bounds.left() + 14.0),
                device(row_top),
                clip,
                palette.text,
                &text,
            );
        }
        if self.network.entries().len() > self.visible && self.visible > 0 {
            let track_height = (bottom - TREE_PAD - top).max(0.0);
            let thumb_height = (track_height * self.visible as f32
                / self.network.entries().len() as f32)
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
        self.draw_header(frame, s, dark, None, font);
    }

    fn draw_console(
        &self,
        frame: &mut Pixmap,
        s: f32,
        dark: bool,
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
        self.draw_header(frame, s, dark, None, font);
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

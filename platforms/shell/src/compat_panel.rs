//! The DevTools Compatibility tab: lists the features the current page uses
//! that the engine does not implement, using `devtools::compat`.

use std::sync::Arc;

use ab_glyph::FontVec;
use devtools::compat::{
    Area, AuditReport, Auditor, Evidence, PageScriptEngine, PageSources, SourcePreview,
};
use tiny_skia::{Color, PathBuilder, Pixmap, Rect, Transform};
use webapis::{AlertSink, ConsoleSink};

use crate::toolbar;

/// The origin probe pages run in. `.invalid` can never resolve.
const PROBE_ORIGIN: &str = "https://compat-audit.invalid/";

struct Silent;

impl ConsoleSink for Silent {
    fn log(&self, _message: &str) {}
}

impl AlertSink for Silent {
    fn alert(&self, _message: &str) {}
}

/// Evaluates audit probes in the same realm setup real pages get. Probes see
/// an ordinary `https` origin (so origin-gated APIs are judged as a typical
/// site sees them) backed by in-memory storage, so they leave no trace.
pub(crate) struct ShellScriptEngine {
    runtime: browser::BrowserRuntime,
    controller: Arc<net::RequestController>,
}

impl ShellScriptEngine {
    pub(crate) fn new() -> Self {
        Self {
            runtime: browser::BrowserRuntime::in_memory(),
            controller: browser::request_controller(None),
        }
    }
}

impl PageScriptEngine for ShellScriptEngine {
    fn evaluate_page(&self, html: &str) -> js::JsResult<js::Value> {
        self.runtime
            .evaluate_page_scripts(
                html,
                Arc::new(Silent),
                Arc::new(Silent),
                Arc::clone(&self.controller),
                self.runtime.security_context_for_url(PROBE_ORIGIN),
            )
            .map(|values| values.last().cloned().unwrap_or(js::Value::Undefined))
    }
}

/// Audits a loaded page.
pub(crate) fn audit_page(
    auditor: &Auditor<ShellScriptEngine>,
    page: &browser::Page,
    script_error: Option<&str>,
) -> AuditReport {
    auditor.audit(
        &PageSources {
            html: &page.html,
            stylesheets: &page.stylesheets,
            scripts: &page.scripts,
        },
        script_error,
    )
}

/// The auditor type the shell keeps for the browser session.
pub(crate) type ShellAuditor = Auditor<ShellScriptEngine>;

/// Where the Compatibility tab is in its life cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompatState {
    /// No audit has been requested for this dock yet.
    Idle,
    /// An audit is running in the background.
    Auditing,
    /// The audit finished.
    Ready(AuditReport),
    /// The page cannot be audited (for example, nothing is loaded).
    Unavailable(&'static str),
}

/// What one display row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// An area heading such as "CSS properties".
    Section,
    /// The first row of a finding, with its evidence.
    Item(Option<Evidence>),
    /// A wrapped row of a finding's explanation.
    Detail,
    /// Status or summary text.
    Note,
}

/// One unwrapped entry: a heading, a finding, or a note.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: LineKind,
    text: String,
    /// Right-aligned metadata for items: count, location, and evidence.
    meta: String,
    /// The explanation shown under an item.
    detail: String,
    /// Index into the report's findings, for finding rows.
    finding: Option<usize>,
}

/// One display row after wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    kind: LineKind,
    text: String,
    meta: String,
    /// Index into the report's findings, on a finding's first row.
    finding: Option<usize>,
}

/// The code preview popover opened by right-clicking a finding.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OpenPreview {
    /// Index into the report's findings.
    finding: usize,
    /// Pointer x where the menu was requested, in logical pixels.
    x: f32,
    /// The clicked row's top and bottom edges, in logical pixels.
    row_top: f32,
    row_bottom: f32,
}

/// The Compatibility tab's content and its scrolling state.
pub(crate) struct CompatView {
    state: CompatState,
    lines: Vec<Line>,
    /// The body width, in logical pixels, `lines` were wrapped for.
    wrapped_width: Option<f32>,
    offset: usize,
    visible: usize,
    wheel_remainder: f64,
    /// The body rectangle of the last draw, in logical pixels, for hit tests.
    body: Option<Rect>,
    /// The open code preview, if any.
    preview: Option<OpenPreview>,
}

/// Row height in logical pixels, matching the Elements tree.
const ROW: f32 = 20.0;
const TEXT_SIZE: f32 = 12.5;
const META_SIZE: f32 = 11.5;
/// Left edge of item text and detail text relative to the body, in logical
/// pixels.
const ITEM_INDENT: f32 = 30.0;
const SCROLLBAR_GUTTER: f32 = 14.0;

impl CompatView {
    pub(crate) fn new() -> Self {
        Self {
            state: CompatState::Idle,
            lines: Vec::new(),
            wrapped_width: None,
            offset: 0,
            visible: 0,
            wheel_remainder: 0.0,
            body: None,
            preview: None,
        }
    }

    pub(crate) fn state(&self) -> &CompatState {
        &self.state
    }

    /// Replaces the state and re-flows the rows on the next draw.
    pub(crate) fn set_state(&mut self, state: CompatState) {
        self.state = state;
        self.wrapped_width = None;
        self.lines.clear();
        self.offset = 0;
        self.wheel_remainder = 0.0;
        self.preview = None;
    }

    /// Short text for the dock header, such as "12 issues · 80 checked".
    pub(crate) fn summary(&self) -> String {
        match &self.state {
            CompatState::Idle | CompatState::Auditing => "Auditing page…".to_owned(),
            CompatState::Unavailable(reason) => (*reason).to_owned(),
            CompatState::Ready(report) => format!(
                "{} not working · {} features checked",
                report.findings.len(),
                report.features_checked
            ),
        }
    }

    /// The number shown on the tab once the audit is done.
    pub(crate) fn issue_count(&self) -> Option<usize> {
        match &self.state {
            CompatState::Ready(report) => Some(report.findings.len()),
            _ => None,
        }
    }

    fn entries(&self) -> Vec<Entry> {
        let note = |text: &str| Entry {
            kind: LineKind::Note,
            text: text.to_owned(),
            meta: String::new(),
            detail: String::new(),
            finding: None,
        };
        let report = match &self.state {
            CompatState::Idle | CompatState::Auditing => {
                return vec![note(
                    "Auditing… detecting the features this page uses and probing the engine for each one.",
                )];
            }
            CompatState::Unavailable(reason) => return vec![note(reason)],
            CompatState::Ready(report) => report,
        };
        let mut entries = Vec::new();
        if report.findings.is_empty() {
            entries.push(note(&format!(
                "Every one of the {} features this page uses passed its probe.",
                report.features_checked
            )));
        }
        let mut area: Option<Area> = None;
        for (index, finding) in report.findings.iter().enumerate() {
            if area != Some(finding.area) {
                area = Some(finding.area);
                let count = report.in_area(finding.area).count();
                entries.push(Entry {
                    kind: LineKind::Section,
                    text: finding.area.label().to_owned(),
                    meta: count.to_string(),
                    detail: String::new(),
                    finding: None,
                });
            }
            let mut meta = Vec::new();
            if finding.occurrences > 1 {
                meta.push(format!("×{}", finding.occurrences));
            }
            meta.extend(finding.location.clone());
            meta.push(finding.evidence.label().to_owned());
            entries.push(Entry {
                kind: LineKind::Item(Some(finding.evidence)),
                text: finding.feature.clone(),
                meta: meta.join(" · "),
                detail: finding.detail.clone(),
                finding: Some(index),
            });
        }
        if !report.unverified.is_empty() {
            entries.push(Entry {
                kind: LineKind::Section,
                text: "Not verified (no static probe)".to_owned(),
                meta: report.unverified.len().to_string(),
                detail: String::new(),
                finding: None,
            });
            for item in &report.unverified {
                entries.push(Entry {
                    kind: LineKind::Item(None),
                    text: item.feature.clone(),
                    meta: item.location.clone().unwrap_or_default(),
                    detail: item.reason.clone(),
                    finding: None,
                });
            }
        }
        entries
    }

    /// Wraps entries into rows for a body `width` logical pixels wide.
    /// `measure` returns a string's width in logical pixels at `size`.
    fn wrap(&mut self, width: f32, measure: &dyn Fn(&str, f32) -> f32) {
        let text_width = (width - ITEM_INDENT - SCROLLBAR_GUTTER - 8.0).max(80.0);
        let mut lines = Vec::new();
        for entry in self.entries() {
            match entry.kind {
                LineKind::Note => {
                    for row in wrap_words(&entry.text, text_width, TEXT_SIZE, measure) {
                        lines.push(Line {
                            kind: LineKind::Note,
                            text: row,
                            meta: String::new(),
                            finding: None,
                        });
                    }
                }
                _ => {
                    lines.push(Line {
                        kind: entry.kind,
                        text: entry.text,
                        meta: entry.meta,
                        finding: entry.finding,
                    });
                    for row in wrap_words(&entry.detail, text_width, META_SIZE, measure) {
                        lines.push(Line {
                            kind: LineKind::Detail,
                            text: row,
                            meta: String::new(),
                            finding: None,
                        });
                    }
                }
            }
        }
        self.lines = lines;
        self.wrapped_width = Some(width);
        self.offset = self.offset.min(self.max_offset());
    }

    /// Sets how many rows fit in the body.
    pub(crate) fn set_visible(&mut self, rows: usize) {
        self.visible = rows;
        self.offset = self.offset.min(self.max_offset());
    }

    fn max_offset(&self) -> usize {
        self.lines.len().saturating_sub(self.visible.max(1))
    }

    /// Scrolls by rows; fractional wheel deltas accumulate.
    pub(crate) fn scroll(&mut self, rows: f64) {
        self.preview = None;
        self.wheel_remainder += rows;
        let whole = self.wheel_remainder.trunc() as isize;
        self.wheel_remainder -= whole as f64;
        self.offset = self
            .offset
            .saturating_add_signed(whole)
            .min(self.max_offset());
    }

    /// Handles navigation keys while this tab is shown.
    pub(crate) fn key(&mut self, key: &winit::keyboard::NamedKey) {
        use winit::keyboard::NamedKey;
        self.wheel_remainder = 0.0;
        self.preview = None;
        let page = self.visible.max(1) as f64;
        match key {
            NamedKey::ArrowDown => self.scroll(1.0),
            NamedKey::ArrowUp => self.scroll(-1.0),
            NamedKey::PageDown => self.scroll(page),
            NamedKey::PageUp => self.scroll(-page),
            NamedKey::Home => self.offset = 0,
            NamedKey::End => self.offset = self.max_offset(),
            _ => {}
        }
    }

    /// The index into `lines` of the row under `cursor`, if the pointer is
    /// over a row of the last drawn body (not its scrollbar gutter).
    fn line_at(&self, cursor: Option<(f64, f64)>) -> Option<usize> {
        let body = self.body?;
        let (x, y) = cursor?;
        let (x, y) = (x as f32, y as f32);
        if x < body.left() || x >= body.right() - SCROLLBAR_GUTTER || y < body.top() {
            return None;
        }
        let row = ((y - body.top()) / ROW) as usize;
        let shown = self
            .visible
            .min(self.lines.len().saturating_sub(self.offset));
        (row < shown).then_some(self.offset + row)
    }

    /// The source preview of the finding at `index` of the report.
    fn source_preview(&self, index: usize) -> Option<&SourcePreview> {
        match &self.state {
            CompatState::Ready(report) => report.findings.get(index)?.preview.as_ref(),
            _ => None,
        }
    }

    /// Handles a secondary (right) click: on a finding whose failing code is
    /// known it opens a preview of that code; anywhere else it closes an
    /// open preview. Returns whether anything changed.
    pub(crate) fn context_click(&mut self, cursor: Option<(f64, f64)>) -> bool {
        let target = self.line_at(cursor).and_then(|line| {
            let finding = self.lines[line].finding?;
            self.source_preview(finding)?;
            Some((line, finding))
        });
        match (target, self.body, cursor) {
            (Some((line, finding)), Some(body), Some((x, _))) => {
                let row_top = body.top() + (line - self.offset) as f32 * ROW;
                self.preview = Some(OpenPreview {
                    finding,
                    x: x as f32,
                    row_top,
                    row_bottom: row_top + ROW,
                });
                true
            }
            _ => self.close_preview(),
        }
    }

    /// Closes the code preview. Returns whether it was open.
    pub(crate) fn close_preview(&mut self) -> bool {
        self.preview.take().is_some()
    }

    /// Whether the code preview is open.
    #[cfg(test)]
    pub(crate) fn preview_open(&self) -> bool {
        self.preview.is_some()
    }

    /// Draws the rows into `body` (logical pixels) of `frame`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        frame: &mut Pixmap,
        body: Rect,
        scale: f64,
        dark: bool,
        cursor: Option<(f64, f64)>,
        ui_font: Option<&FontVec>,
        mono_font: Option<&FontVec>,
    ) {
        let s = scale as f32;
        let transform = Transform::from_scale(s, s);
        let width = body.width();
        self.body = Some(body);
        if self.wrapped_width != Some(width) || self.lines.is_empty() {
            match ui_font {
                Some(font) => self.wrap(width, &|text, size| {
                    toolbar::text_advance(font, size * s, text) / s
                }),
                None => self.wrap(width, &estimate),
            }
        }
        self.set_visible((body.height() / ROW).max(0.0) as usize);
        let colors = Colors::new(dark);
        let palette = toolbar::palette(dark);
        let shown = self
            .visible
            .min(self.lines.len().saturating_sub(self.offset));

        // Hover highlight for the finding under the pointer (or the one
        // whose code is being previewed).
        let highlighted = match self.preview {
            Some(open) => self
                .lines
                .iter()
                .position(|line| line.finding == Some(open.finding)),
            None => self.line_at(cursor),
        };
        if let Some(line) = highlighted
            && line >= self.offset
        {
            let row = line - self.offset;
            if row < shown
                && matches!(self.lines[line].kind, LineKind::Item(_))
                && let Some(rect) =
                    Rect::from_xywh(body.left(), body.top() + row as f32 * ROW, width, ROW)
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(rect),
                    colors.row_hover,
                    transform,
                );
            }
        }

        if self.lines.len() > self.visible && self.visible > 0 {
            let track = body.height() - 4.0;
            let thumb = (track * self.visible as f32 / self.lines.len() as f32).max(24.0);
            let progress = self.offset as f32 / self.max_offset().max(1) as f32;
            if let Some(path) = toolbar::rounded_rect(
                body.right() - 9.0,
                body.top() + 2.0 + (track - thumb).max(0.0) * progress,
                5.0,
                thumb.min(track),
                2.5,
            ) {
                toolbar::fill(frame, &path, colors.scrollbar, transform);
            }
        }

        let Some(ui_font) = ui_font.or(mono_font) else {
            return;
        };
        let mono_font = mono_font.unwrap_or(ui_font);
        let device = |value: f32| value * s;
        let clip = body.right() - SCROLLBAR_GUTTER;
        for (index, line) in self.lines.iter().skip(self.offset).take(shown).enumerate() {
            let top = body.top() + index as f32 * ROW;
            let baseline = top + ROW / 2.0 + 4.0;
            match line.kind {
                LineKind::Section => {
                    if index > 0
                        && let Some(rule) = Rect::from_xywh(body.left(), top, width, 1.0)
                    {
                        toolbar::fill(
                            frame,
                            &PathBuilder::from_rect(rule),
                            colors.separator,
                            transform,
                        );
                    }
                    let pen = toolbar::draw_text(
                        frame,
                        ui_font,
                        device(TEXT_SIZE),
                        device(body.left() + 14.0),
                        device(baseline),
                        device(clip),
                        colors.section,
                        &line.text,
                    );
                    toolbar::draw_text(
                        frame,
                        ui_font,
                        device(META_SIZE),
                        pen + device(8.0),
                        device(baseline),
                        device(clip),
                        palette.text_muted,
                        &line.meta,
                    );
                }
                LineKind::Item(evidence) => {
                    let dot = colors.evidence(evidence);
                    if let Some(path) = toolbar::rounded_rect(
                        body.left() + 16.0,
                        top + ROW / 2.0 - 3.5,
                        7.0,
                        7.0,
                        3.5,
                    ) {
                        toolbar::fill(frame, &path, dot, transform);
                    }
                    let meta_width =
                        toolbar::text_advance(ui_font, device(META_SIZE), &line.meta) / s;
                    let feature_left = body.left() + ITEM_INDENT;
                    let meta_left = (clip - 4.0 - meta_width).max(feature_left + 120.0);
                    toolbar::draw_text(
                        frame,
                        mono_font,
                        device(TEXT_SIZE),
                        device(feature_left),
                        device(baseline),
                        device(meta_left - 10.0),
                        palette.text,
                        &line.text,
                    );
                    toolbar::draw_text(
                        frame,
                        ui_font,
                        device(META_SIZE),
                        device(meta_left),
                        device(baseline),
                        device(clip),
                        palette.text_muted,
                        &line.meta,
                    );
                }
                LineKind::Detail | LineKind::Note => {
                    let left = if line.kind == LineKind::Detail {
                        ITEM_INDENT
                    } else {
                        14.0
                    };
                    toolbar::draw_text(
                        frame,
                        ui_font,
                        device(if line.kind == LineKind::Detail {
                            META_SIZE
                        } else {
                            TEXT_SIZE
                        }),
                        device(body.left() + left),
                        device(baseline - 2.0),
                        device(clip),
                        palette.text_muted,
                        &line.text,
                    );
                }
            }
        }
        if let Some(open) = self.preview
            && let Some(preview) = self.source_preview(open.finding)
        {
            draw_preview(frame, body, s, dark, open, preview, ui_font, mono_font);
        }
    }
}

/// Code row height inside the preview, in logical pixels.
const CODE_ROW: f32 = 17.0;
const CODE_SIZE: f32 = 12.0;
const PREVIEW_HEADER: f32 = 26.0;
const PREVIEW_PAD: f32 = 8.0;

/// Draws the code preview popover for `open` inside `body`.
#[allow(clippy::too_many_arguments)]
fn draw_preview(
    frame: &mut Pixmap,
    body: Rect,
    s: f32,
    dark: bool,
    open: OpenPreview,
    preview: &SourcePreview,
    ui_font: &FontVec,
    mono_font: &FontVec,
) {
    let transform = Transform::from_scale(s, s);
    let device = |value: f32| value * s;
    let colors = Colors::new(dark);
    let palette = toolbar::palette(dark);

    let width = (body.width() - 16.0).clamp(160.0, 820.0);
    let height = PREVIEW_HEADER + preview.lines.len() as f32 * CODE_ROW + PREVIEW_PAD;
    let left = (open.x - 24.0)
        .min(body.right() - width - 8.0)
        .max(body.left() + 8.0);
    let below = open.row_bottom + 2.0;
    let top = if below + height <= body.bottom() - 4.0 {
        below
    } else {
        (open.row_top - height - 2.0).max(body.top() + 4.0)
    };
    if let Some(path) = toolbar::rounded_rect(left, top + 2.0, width, height, 8.0) {
        toolbar::fill(frame, &path, colors.preview_shadow, transform);
    }
    if let Some(path) = toolbar::rounded_rect(left, top, width, height, 8.0) {
        toolbar::fill(frame, &path, colors.preview_background, transform);
        toolbar::stroke(frame, &path, colors.preview_border, 1.0, transform);
    }
    let right = left + width - 12.0;

    // Header: where the code is, and how to dismiss it.
    let mut place = format!("{} · line {}", preview.source, preview.line);
    if let Some(column) = preview.column {
        place.push_str(&format!(", column {column}"));
    }
    let hint = "Esc to close";
    let hint_width = toolbar::text_advance(ui_font, device(META_SIZE), hint) / s;
    let header_baseline = top + PREVIEW_HEADER / 2.0 + 4.0;
    toolbar::draw_text(
        frame,
        ui_font,
        device(META_SIZE),
        device(left + 12.0),
        device(header_baseline),
        device(right - hint_width - 12.0),
        palette.text,
        &place,
    );
    toolbar::draw_text(
        frame,
        ui_font,
        device(META_SIZE),
        device(right - hint_width),
        device(header_baseline),
        device(right),
        palette.text_muted,
        hint,
    );
    if let Some(rule) = Rect::from_xywh(left, top + PREVIEW_HEADER - 1.0, width, 1.0) {
        toolbar::fill(
            frame,
            &PathBuilder::from_rect(rule),
            colors.separator,
            transform,
        );
    }

    // Gutter of line numbers, then the code.
    let last_number = preview.first_line + preview.lines.len().saturating_sub(1);
    let gutter = toolbar::text_advance(mono_font, device(CODE_SIZE), &last_number.to_string()) / s;
    let number_right = left + 12.0 + gutter;
    let code_left = number_right + 14.0;
    let char_width = (toolbar::text_advance(mono_font, device(CODE_SIZE), "0") / s).max(1.0);
    let fits = (((right - code_left) / char_width) as usize).max(8);
    let skip = horizontal_skip(preview, fits);
    let cut = preview.first_column > 1 || skip > 0;
    for (index, text) in preview.lines.iter().enumerate() {
        let number = preview.first_line + index;
        let row_top = top + PREVIEW_HEADER + 4.0 + index as f32 * CODE_ROW;
        let baseline = row_top + CODE_ROW / 2.0 + 4.0;
        let focus = number == preview.line;
        if focus && let Some(rect) = Rect::from_xywh(left + 1.0, row_top, width - 2.0, CODE_ROW) {
            toolbar::fill(
                frame,
                &PathBuilder::from_rect(rect),
                colors.preview_focus,
                transform,
            );
        }
        let label = number.to_string();
        let label_width = toolbar::text_advance(mono_font, device(CODE_SIZE), &label) / s;
        toolbar::draw_text(
            frame,
            mono_font,
            device(CODE_SIZE),
            device(number_right - label_width),
            device(baseline),
            device(number_right),
            if focus {
                palette.text
            } else {
                palette.text_muted
            },
            &label,
        );
        let mut x = code_left;
        if cut {
            x = toolbar::draw_text(
                frame,
                mono_font,
                device(CODE_SIZE),
                device(x),
                device(baseline),
                device(right),
                palette.text_muted,
                "…",
            ) / s;
        }
        let shown: String = text.chars().skip(skip).collect();
        toolbar::draw_text(
            frame,
            mono_font,
            device(CODE_SIZE),
            device(x),
            device(baseline),
            device(right),
            if focus {
                palette.text
            } else {
                palette.text_muted
            },
            &shown,
        );
        // Underline the character the parser stopped at.
        if focus
            && let Some(column) = preview.column
            && let Some(at) = column
                .saturating_sub(preview.first_column)
                .checked_sub(skip)
        {
            let before: String = text.chars().skip(skip).take(at).collect();
            let glyph = text.chars().nth(skip + at).unwrap_or(' ').to_string();
            let start = x + toolbar::text_advance(mono_font, device(CODE_SIZE), &before) / s;
            let span = (toolbar::text_advance(mono_font, device(CODE_SIZE), &glyph) / s).max(6.0);
            if start < right
                && let Some(rect) = Rect::from_xywh(
                    start,
                    row_top + CODE_ROW - 3.0,
                    span.min(right - start),
                    2.0,
                )
            {
                toolbar::fill(
                    frame,
                    &PathBuilder::from_rect(rect),
                    colors.probed,
                    transform,
                );
            }
        }
    }
}

/// How many leading characters of each excerpt line to hide so the failing
/// column is visible when `fits` characters fit across the preview.
fn horizontal_skip(preview: &SourcePreview, fits: usize) -> usize {
    let Some(column) = preview.column else {
        return 0;
    };
    let at = column.saturating_sub(preview.first_column);
    if at < fits * 2 / 3 { 0 } else { at - fits / 3 }
}

/// Approximate text width when no font is loaded (tests).
fn estimate(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * 0.55
}

/// Greedy word wrapping to `width` logical pixels.
fn wrap_words(
    text: &str,
    width: f32,
    size: f32,
    measure: &dyn Fn(&str, f32) -> f32,
) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if !current.is_empty() && measure(&candidate, size) > width {
            rows.push(std::mem::replace(&mut current, word.to_owned()));
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

/// Colors specific to the Compatibility tab.
struct Colors {
    section: Color,
    separator: Color,
    row_hover: Color,
    scrollbar: Color,
    probed: Color,
    observed: Color,
    name_match: Color,
    documented: Color,
    unverified: Color,
    preview_background: Color,
    preview_border: Color,
    preview_shadow: Color,
    preview_focus: Color,
}

impl Colors {
    fn new(dark: bool) -> Self {
        let rgba = toolbar::rgba;
        if dark {
            Self {
                section: rgba(0x6c, 0xb8, 0xf0, 255),
                separator: rgba(255, 255, 255, 18),
                row_hover: rgba(255, 255, 255, 12),
                scrollbar: rgba(255, 255, 255, 60),
                probed: rgba(0xf2, 0x6d, 0x6d, 255),
                observed: rgba(0xf0, 0xa0, 0x50, 255),
                name_match: rgba(0xe6, 0xc4, 0x5c, 255),
                documented: rgba(0x9a, 0xa3, 0xb0, 255),
                unverified: rgba(0x6b, 0x72, 0x7d, 255),
                preview_background: rgba(0x1c, 0x1f, 0x26, 255),
                preview_border: rgba(255, 255, 255, 30),
                preview_shadow: rgba(0, 0, 0, 90),
                preview_focus: rgba(0xf2, 0x6d, 0x6d, 40),
            }
        } else {
            Self {
                section: rgba(0x0b, 0x5c, 0xad, 255),
                separator: rgba(15, 23, 42, 22),
                row_hover: rgba(0x00, 0x6e, 0xb6, 16),
                scrollbar: rgba(15, 23, 42, 70),
                probed: rgba(0xd9, 0x30, 0x25, 255),
                observed: rgba(0xe0, 0x7a, 0x10, 255),
                name_match: rgba(0xc9, 0x9a, 0x06, 255),
                documented: rgba(0x6b, 0x72, 0x80, 255),
                unverified: rgba(0xb0, 0xb6, 0xc0, 255),
                preview_background: rgba(255, 255, 255, 255),
                preview_border: rgba(15, 23, 42, 40),
                preview_shadow: rgba(15, 23, 42, 30),
                preview_focus: rgba(0xd9, 0x30, 0x25, 26),
            }
        }
    }

    fn evidence(&self, evidence: Option<Evidence>) -> Color {
        match evidence {
            Some(Evidence::Probed) => self.probed,
            Some(Evidence::Observed) => self.observed,
            Some(Evidence::NameMatch) => self.name_match,
            Some(Evidence::Documented) => self.documented,
            None => self.unverified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> AuditReport {
        let page = browser::Page {
            url: "https://example.test/".into(),
            html: "<html><body><ul><li>x</li></ul><script>a</script></body></html>".into(),
            stylesheets: vec![".x { text-shadow: 1px 1px #000; cursor: pointer }".into()],
            scripts: vec!["var t = /a+/.test('a'); var o = new IntersectionObserver(f);".into()],
            resources: Default::default(),
        };
        audit_page(&Auditor::new(ShellScriptEngine::new()), &page, None)
    }

    #[test]
    fn real_page_realm_reports_missing_apis_and_syntax() {
        let report = report();
        let features: Vec<&str> = report.findings.iter().map(|f| f.feature.as_str()).collect();
        for expected in [
            "Regular expression literals",
            "IntersectionObserver",
            "text-shadow",
            "List markers",
        ] {
            assert!(
                features.contains(&expected),
                "{expected} missing from {features:?}"
            );
        }
        // `document` is installed by the page realm, so it is never reported.
        assert!(!features.contains(&"document"));
        assert!(
            report
                .unverified
                .iter()
                .any(|item| item.feature == "cursor")
        );
    }

    #[test]
    fn rows_group_findings_by_area_and_wrap_details() {
        let mut view = CompatView::new();
        assert_eq!(view.summary(), "Auditing page…");
        view.set_state(CompatState::Ready(report()));
        view.wrap(300.0, &estimate);
        let sections: Vec<&str> = view
            .lines
            .iter()
            .filter(|line| line.kind == LineKind::Section)
            .map(|line| line.text.as_str())
            .collect();
        // The script uses a regular-expression literal, so the engine cannot parse it at all.
        assert_eq!(sections.first(), Some(&"Script execution"));
        assert!(sections.contains(&"JavaScript syntax"));
        assert!(sections.contains(&"CSS properties"));
        assert_eq!(sections.last(), Some(&"Not verified (no static probe)"));
        // Narrow bodies wrap details over several rows.
        let details = view
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Detail)
            .count();
        let items = view
            .lines
            .iter()
            .filter(|l| matches!(l.kind, LineKind::Item(_)))
            .count();
        assert!(details > items);
        assert!(view.summary().contains("not working"));
        let findings = view
            .lines
            .iter()
            .filter(|l| matches!(l.kind, LineKind::Item(Some(_))))
            .count();
        assert_eq!(view.issue_count(), Some(findings));
    }

    #[test]
    fn scrolling_is_clamped_and_wheel_deltas_accumulate() {
        let mut view = CompatView::new();
        view.set_state(CompatState::Ready(report()));
        view.wrap(240.0, &estimate);
        view.set_visible(5);
        view.key(&winit::keyboard::NamedKey::End);
        assert_eq!(view.offset, view.max_offset());
        view.scroll(1_000.0);
        assert_eq!(view.offset, view.max_offset());
        view.key(&winit::keyboard::NamedKey::Home);
        for _ in 0..4 {
            view.scroll(0.25);
        }
        assert_eq!(view.offset, 1);
    }

    #[test]
    fn right_clicking_a_finding_previews_its_code() {
        let page = browser::Page {
            url: "https://example.test/".into(),
            html: "<html><body><script>a</script></body></html>".into(),
            stylesheets: Vec::new(),
            scripts: vec!["var a = 1;\nvar b = ;\nvar c = 3;".into()],
            resources: Default::default(),
        };
        let report = audit_page(&Auditor::new(ShellScriptEngine::new()), &page, None);
        let mut view = CompatView::new();
        view.set_state(CompatState::Ready(report));
        view.wrap(600.0, &estimate);
        view.set_visible(20);
        view.body = Rect::from_xywh(0.0, 100.0, 600.0, 400.0);
        let row_of = |view: &CompatView, feature: &str| {
            view.lines
                .iter()
                .position(|line| line.text == feature)
                .unwrap_or_else(|| panic!("{feature} is listed"))
        };
        let at_row = |row: usize| Some((60.0, 100.0 + row as f64 * f64::from(ROW) + 5.0));

        let parse = row_of(&view, "inline script #1 cannot be parsed");
        assert!(view.context_click(at_row(parse)));
        assert!(view.preview_open());
        let open = view.preview.unwrap();
        let preview = view.source_preview(open.finding).unwrap();
        assert_eq!(preview.line, 2);
        assert_eq!(preview.focus(), "var b = ;");

        // Detail rows and empty space close the preview instead.
        assert!(view.context_click(at_row(parse + 1)));
        assert!(!view.preview_open());
        assert!(!view.context_click(Some((60.0, 99.0))));

        // Scrolling and new audits close it too, and drawing without fonts
        // with it open is harmless.
        view.context_click(at_row(parse));
        let mut frame = Pixmap::new(600, 500).unwrap();
        view.draw(&mut frame, view.body.unwrap(), 1.0, true, None, None, None);
        view.scroll(1.0);
        assert!(!view.preview_open());
        view.scroll(-1.0);
        view.context_click(at_row(parse));
        view.set_state(CompatState::Auditing);
        assert!(!view.preview_open());
    }

    #[test]
    fn previews_scroll_sideways_to_keep_the_column_visible() {
        let line = "x".repeat(200);
        let near = SourcePreview::new("a.js", &line, 1, Some(10)).unwrap();
        assert_eq!(horizontal_skip(&near, 80), 0);
        let far = SourcePreview::new("a.js", &line, 1, Some(150)).unwrap();
        let skip = horizontal_skip(&far, 80);
        let at = far.column.unwrap() - far.first_column;
        assert!(skip <= at && at - skip < 80, "column stays on screen");
        let unknown = SourcePreview::new("a.js", &line, 1, None).unwrap();
        assert_eq!(horizontal_skip(&unknown, 80), 0);
    }

    #[test]
    fn wrap_words_never_splits_words() {
        let rows = wrap_words("one two three four", 50.0, 10.0, &estimate);
        assert!(rows.len() > 1);
        assert_eq!(rows.join(" "), "one two three four");
    }
}

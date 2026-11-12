//! The DevTools Compatibility tab: lists the features the current page uses
//! that the engine does not implement, using `devtools::compat`.

use std::sync::Arc;

use ab_glyph::FontVec;
use devtools::compat::{Area, AuditReport, Auditor, Evidence, PageScriptEngine, PageSources};
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
                browser::security_context_for_url(PROBE_ORIGIN),
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
}

/// One display row after wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    kind: LineKind,
    text: String,
    meta: String,
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
        for finding in &report.findings {
            if area != Some(finding.area) {
                area = Some(finding.area);
                let count = report.in_area(finding.area).count();
                entries.push(Entry {
                    kind: LineKind::Section,
                    text: finding.area.label().to_owned(),
                    meta: count.to_string(),
                    detail: String::new(),
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
            });
        }
        if !report.unverified.is_empty() {
            entries.push(Entry {
                kind: LineKind::Section,
                text: "Not verified (no static probe)".to_owned(),
                meta: report.unverified.len().to_string(),
                detail: String::new(),
            });
            for item in &report.unverified {
                entries.push(Entry {
                    kind: LineKind::Item(None),
                    text: item.feature.clone(),
                    meta: item.location.clone().unwrap_or_default(),
                    detail: item.reason.clone(),
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
                        });
                    }
                }
                _ => {
                    lines.push(Line {
                        kind: entry.kind,
                        text: entry.text,
                        meta: entry.meta,
                    });
                    for row in wrap_words(&entry.detail, text_width, META_SIZE, measure) {
                        lines.push(Line {
                            kind: LineKind::Detail,
                            text: row,
                            meta: String::new(),
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

        // Hover highlight for the finding under the pointer.
        if let Some((x, y)) = cursor
            && x >= f64::from(body.left())
            && x < f64::from(body.right() - SCROLLBAR_GUTTER)
            && y >= f64::from(body.top())
        {
            let row = ((y - f64::from(body.top())) / f64::from(ROW)) as usize;
            if row < shown
                && matches!(self.lines[self.offset + row].kind, LineKind::Item(_))
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
    }
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
            stylesheets: vec![".x { box-shadow: 1px 1px #000; cursor: pointer }".into()],
            scripts: vec!["var t = typeof 1; var o = new IntersectionObserver(f);".into()],
            font_faces: Vec::new(),
        };
        audit_page(&Auditor::new(ShellScriptEngine::new()), &page, None)
    }

    #[test]
    fn real_page_realm_reports_missing_apis_and_syntax() {
        let report = report();
        let features: Vec<&str> = report.findings.iter().map(|f| f.feature.as_str()).collect();
        for expected in [
            "typeof operator",
            "IntersectionObserver",
            "box-shadow",
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
        // The script uses `typeof`, so the engine cannot parse it at all.
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
    fn wrap_words_never_splits_words() {
        let rows = wrap_words("one two three four", 50.0, 10.0, &estimate);
        assert!(rows.len() > 1);
        assert_eq!(rows.join(" "), "one two three four");
    }
}

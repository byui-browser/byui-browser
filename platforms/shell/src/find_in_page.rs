//! Find searches laid-out text only. Unicode lowercase matching is locale independent;
//! it does not perform canonical normalization or full Unicode case folding (ß != ss).
//! Whitespace is collapsed for phrase matching. Separate block contexts never join.

use ab_glyph::{Font, FontVec, ScaleFont};
use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Transform};

use crate::toolbar;
use std::ops::Range;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const HEIGHT: f32 = 40.0;

/// A visible layout text fragment, with exact document-space CSS bounds.
#[derive(Clone, Debug)]
pub(crate) struct TextRun {
    pub text: String,
    pub rect: layout::Rect,
    /// Starts a separate block context or follows an atomic inline boundary.
    pub boundary: bool,
    /// A new line in the same inline context; a collapsed whitespace boundary.
    pub new_line: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Field,
    Previous,
    Next,
    Close,
}

#[derive(Default)]
pub(crate) struct FindInPage {
    pub query: String,
    pub focused: bool,
    caret: usize,
    selected: bool,
    runs: Vec<TextRun>,
    matches: Vec<Vec<SourceRange>>,
    current: usize,
}

/// Original UTF-8 range in a layout run. One match may cover many runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceRange {
    pub run: usize,
    pub bytes: Range<usize>,
}

fn normalized(text: &str) -> Vec<char> {
    let mut result = Vec::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() {
            if result.last() != Some(&' ') {
                result.push(' ');
            }
        } else {
            result.push(c);
        }
    }
    result
}

impl FindInPage {
    pub fn focus(&mut self) {
        self.focused = true;
        self.selected = true;
    }

    pub fn replace_runs(&mut self, runs: Vec<TextRun>) {
        self.runs = runs;
        self.search();
    }

    pub fn search(&mut self) {
        self.matches.clear();
        let needle = normalized(&self.query);
        if needle.is_empty() || needle.iter().all(|c| *c == ' ') {
            self.current = 0;
            return;
        }
        // Assemble laid-out text before segmenting: a grapheme can span inline elements.
        // Synthetic wrap spaces and block separators have no source rectangle.
        let mut raw = String::new();
        let mut origins = Vec::new();
        for (run_index, run) in self.runs.iter().enumerate() {
            if run.boundary && !raw.is_empty() {
                raw.push('\0');
                origins.push(None);
            } else if run.new_line && !raw.ends_with(char::is_whitespace) {
                raw.push(' ');
                origins.push(None);
            }
            for (byte, c) in run.text.char_indices() {
                raw.push(c);
                origins.push(Some(SourceRange {
                    run: run_index,
                    bytes: byte..byte + c.len_utf8(),
                }));
            }
        }
        let mut text = String::new();
        // Each normalized scalar points to all original scalars in its grapheme.
        let mut locations: Vec<(usize, Rc<Vec<SourceRange>>)> = Vec::new();
        let mut original_index = 0;
        for (cluster, grapheme) in raw.graphemes(true).enumerate() {
            let length = grapheme.chars().count();
            let ranges: Vec<_> = origins[original_index..original_index + length]
                .iter()
                .flatten()
                .cloned()
                .collect();
            let ranges = Rc::new(ranges);
            original_index += length;
            for c in grapheme.chars().flat_map(char::to_lowercase) {
                let c = if c.is_whitespace() { ' ' } else { c };
                if c == ' ' && text.ends_with(' ') {
                    if let Some((_, last)) = locations.last_mut() {
                        Rc::make_mut(last).extend(ranges.iter().cloned());
                    }
                    continue;
                }
                text.push(c);
                locations.push((cluster, ranges.clone()));
            }
        }
        let needle: String = needle.into_iter().collect();
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
            .collect();
        for (offset, _) in text.match_indices(&needle) {
            let start = offsets
                .binary_search(&offset)
                .expect("UTF-8 match boundary");
            let end = offsets
                .binary_search(&(offset + needle.len()))
                .expect("UTF-8 match boundary");
            // Partial lowercase expansions or combining/ZWJ clusters cannot be highlighted exactly.
            if (start > 0 && locations[start - 1].0 == locations[start].0)
                || (end < locations.len() && locations[end - 1].0 == locations[end].0)
            {
                continue;
            }
            let mut ranges: Vec<SourceRange> = Vec::new();
            let mut previous_cluster = None;
            for (cluster, source) in &locations[start..end] {
                if previous_cluster == Some(*cluster) {
                    continue;
                }
                previous_cluster = Some(*cluster);
                for range in source.iter() {
                    if let Some(last) = ranges.last_mut()
                        && last.run == range.run
                        && range.bytes.start <= last.bytes.end
                    {
                        last.bytes.end = last.bytes.end.max(range.bytes.end);
                    } else {
                        ranges.push(range.clone());
                    }
                }
            }
            if !ranges.is_empty() {
                self.matches.push(ranges);
            }
        }
        self.current = self.current.min(self.matches.len().saturating_sub(1));
    }

    pub fn target(&self) -> Option<layout::Rect> {
        self.matches
            .get(self.current)?
            .first()
            .map(|range| self.runs[range.run].rect)
    }
    /// Scroll using the selected fragment's real CSS position, clamped to the document.
    pub fn scroll_target(&self, max_scroll: f32) -> Option<f32> {
        self.target().map(|rect| rect.y.clamp(0.0, max_scroll))
    }

    /// Original fragment ranges and their selected/ordinary palette color.
    pub fn highlight_ranges(&self, dark: bool) -> Vec<(SourceRange, paint::Color)> {
        let ordinary = if dark {
            paint::Color {
                r: 150,
                g: 120,
                b: 0,
                a: 125,
            }
        } else {
            paint::Color {
                r: 255,
                g: 225,
                b: 0,
                a: 130,
            }
        };
        let selected = if dark {
            paint::Color {
                r: 255,
                g: 150,
                b: 40,
                a: 180,
            }
        } else {
            paint::Color {
                r: 255,
                g: 145,
                b: 0,
                a: 165,
            }
        };
        self.matches
            .iter()
            .enumerate()
            .flat_map(|(index, ranges)| {
                let color = if index == self.current {
                    selected
                } else {
                    ordinary
                };
                ranges.iter().cloned().map(move |range| (range, color))
            })
            .collect()
    }

    pub fn count(&self) -> String {
        format!(
            "{}/{}",
            if self.matches.is_empty() {
                0
            } else {
                self.current + 1
            },
            self.matches.len()
        )
    }
    pub fn step(&mut self, previous: bool) {
        if !self.matches.is_empty() {
            self.current = if previous {
                (self.current + self.matches.len() - 1) % self.matches.len()
            } else {
                (self.current + 1) % self.matches.len()
            };
        }
    }
    pub fn insert(&mut self, text: &str) {
        if self.selected {
            self.query.clear();
            self.caret = 0;
            self.selected = false;
        }
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.query.insert_str(self.caret, &text);
        self.caret += text.len();
        self.current = 0;
        self.search();
    }
    pub fn select_all(&mut self) {
        self.selected = true;
    }
    pub fn move_caret(&mut self, direction: i8) {
        self.selected = false;
        self.caret = if direction < 0 {
            self.query[..self.caret]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        } else {
            self.query[self.caret..]
                .chars()
                .next()
                .map_or(self.caret, |c| self.caret + c.len_utf8())
        };
    }
    pub fn end(&mut self, end: bool) {
        self.selected = false;
        self.caret = if end { self.query.len() } else { 0 };
    }
    pub fn delete(&mut self, forward: bool) {
        if self.selected {
            self.query.clear();
            self.caret = 0;
            self.selected = false;
        } else if forward {
            if let Some(c) = self.query[self.caret..].chars().next() {
                self.query.drain(self.caret..self.caret + c.len_utf8());
            }
        } else {
            let end = self.caret;
            self.move_caret(-1);
            self.query.drain(self.caret..end);
        }
        self.current = 0;
        self.search();
    }

    /// Logical bounds shared by drawing and hit testing; never overlap toolbar buttons.
    pub fn regions(width: f32) -> [(Action, Rect); 4] {
        let width = width.max(160.0);
        let x = 12.0;
        let y = toolbar::PAGE_TOP as f32 + 5.0;
        let right = width - 12.0;
        let field = (right - x - 154.0).max(1.0);
        [
            (Action::Field, Rect::from_xywh(x, y, field, 30.0).unwrap()),
            (
                Action::Previous,
                Rect::from_xywh(right - 102.0, y, 32.0, 30.0).unwrap(),
            ),
            (
                Action::Next,
                Rect::from_xywh(right - 66.0, y, 32.0, 30.0).unwrap(),
            ),
            (
                Action::Close,
                Rect::from_xywh(right - 30.0, y, 30.0, 30.0).unwrap(),
            ),
        ]
    }
    pub fn action_at(width: f32, cursor: Option<(f64, f64)>) -> Option<Action> {
        let (x, y) = cursor?;
        Self::regions(width)
            .into_iter()
            .find(|(_, r)| {
                x >= r.left() as f64
                    && x < r.right() as f64
                    && y >= r.top() as f64
                    && y < r.bottom() as f64
            })
            .map(|(a, _)| a)
    }
    pub fn draw(&self, pixmap: &mut Pixmap, scale: f64, dark: bool, font: Option<&FontVec>) {
        let scale = scale as f32;
        let width = pixmap.width() as f32 / scale;
        let palette = toolbar::palette(dark);
        let mut paint = Paint::default();
        paint.set_color(palette.field);
        let bar = Rect::from_xywh(0.0, toolbar::PAGE_TOP as f32, width, HEIGHT).unwrap();
        pixmap.fill_path(
            &PathBuilder::from_rect(bar),
            &paint,
            tiny_skia::FillRule::Winding,
            Transform::from_scale(scale, scale),
            None,
        );
        for (action, rect) in Self::regions(width) {
            paint.set_color(if action == Action::Field {
                palette.hover
            } else {
                palette.field
            });
            pixmap.fill_path(
                &PathBuilder::from_rect(rect),
                &paint,
                tiny_skia::FillRule::Winding,
                Transform::from_scale(scale, scale),
                None,
            );
            if let Some(font) = font {
                let size = 13.0 * scale;
                let metrics = font.as_scaled(ab_glyph::PxScale::from(size));
                let baseline =
                    (rect.y() + 15.0) * scale + (metrics.ascent() + metrics.descent()) / 2.0;
                let text = match action {
                    Action::Field => {
                        if self.query.is_empty() {
                            "Find in page"
                        } else {
                            &self.query
                        }
                    }
                    Action::Previous => "↑",
                    Action::Next => "↓",
                    Action::Close => "×",
                };
                let start = (rect.x() + 6.0) * scale;
                let clip = (rect.right() - 5.0) * scale;
                let mut offset = 0.0;
                if action == Action::Field && self.focused {
                    let caret = toolbar::text_advance(font, size, &self.query[..self.caret]);
                    offset = (caret - (clip - start - 3.0)).max(0.0);
                }
                // Render query into a bounded field pixmap to clip both edges when scrolling.
                if action == Action::Field {
                    let mut field = Pixmap::new(
                        (rect.width() * scale).ceil() as u32,
                        (rect.height() * scale).ceil() as u32,
                    )
                    .unwrap();
                    field.fill(if self.selected && self.focused {
                        palette.selection
                    } else {
                        palette.hover
                    });
                    toolbar::draw_text(
                        &mut field,
                        font,
                        size,
                        6.0 * scale - offset,
                        baseline - rect.y() * scale,
                        field_width(rect, scale),
                        palette.text,
                        text,
                    );
                    if self.focused && !self.selected {
                        let caret = 6.0 * scale
                            + toolbar::text_advance(font, size, &self.query[..self.caret])
                            - offset;
                        let mut p = Paint::default();
                        p.set_color(palette.accent);
                        if let Some(r) = Rect::from_xywh(caret, 6.0 * scale, scale, 18.0 * scale) {
                            field.fill_path(
                                &PathBuilder::from_rect(r),
                                &p,
                                tiny_skia::FillRule::Winding,
                                Transform::identity(),
                                None,
                            );
                        }
                    }
                    pixmap.draw_pixmap(
                        (rect.x() * scale) as i32,
                        (rect.y() * scale) as i32,
                        field.as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        Transform::identity(),
                        None,
                    );
                } else {
                    toolbar::draw_text(
                        pixmap,
                        font,
                        size,
                        start,
                        baseline,
                        clip,
                        palette.text,
                        text,
                    );
                }
            }
        }
        if let Some(font) = font {
            let x = width - 162.0;
            let count = self.count();
            let count_size =
                11.0_f32.min(46.0 / (toolbar::text_advance(font, scale, &count) / scale).max(1.0));
            toolbar::draw_text(
                pixmap,
                font,
                count_size * scale,
                x * scale,
                (toolbar::PAGE_TOP as f32 + 25.0) * scale,
                (width - 116.0) * scale,
                palette.text_muted,
                &self.count(),
            );
        }
    }
}
fn field_width(rect: Rect, scale: f32) -> f32 {
    (rect.width() - 5.0) * scale
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(text: &str, y: f32, boundary: bool, new_line: bool) -> TextRun {
        TextRun {
            text: text.into(),
            rect: layout::Rect {
                x: 0.0,
                y,
                width: 100.0,
                height: 16.0,
            },
            boundary,
            new_line,
        }
    }
    fn search(query: &str, runs: Vec<TextRun>) -> FindInPage {
        let mut find = FindInPage::default();
        find.replace_runs(runs);
        find.insert(query);
        find
    }
    #[test]
    fn ranges_cover_only_matched_substrings_and_each_fragment_of_a_phrase() {
        let f = search("word", vec![run("a word word z", 0.0, true, false)]);
        let ranges = f.highlight_ranges(false);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0.bytes, 2..6);
        assert_eq!(ranges[1].0.bytes, 7..11);
        assert_ne!(ranges[0].1, ranges[1].1);
        let f = search(
            "hello world",
            vec![
                run("Hello ", 0.0, true, false),
                run("world", 20.0, false, true),
            ],
        );
        let ranges = f.highlight_ranges(false);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].0.bytes, 0..6);
        assert_eq!(ranges[1].0.bytes, 0..5);
        assert_eq!(ranges[0].1, ranges[1].1);
    }
    #[test]
    fn selection_moves_colors_and_empty_query_removes_all_backgrounds() {
        let mut f = search("a", vec![run("a a", 0.0, true, false)]);
        for dark in [false, true] {
            let before = f.highlight_ranges(dark);
            f.step(false);
            let after = f.highlight_ranges(dark);
            assert_eq!(before[0].1, after[1].1);
            assert_eq!(before[1].1, after[0].1);
            f.step(true);
        }
        f.select_all();
        f.insert("");
        assert!(f.highlight_ranges(false).is_empty());
    }
    #[test]
    fn expansions_combining_and_zwj_sequences_require_complete_graphemes() {
        let f = search("i\u{307}", vec![run("İ", 0.0, true, false)]);
        assert_eq!(f.count(), "1/1");
        assert_eq!(f.highlight_ranges(false)[0].0.bytes, 0..2);
        assert_eq!(search("i", vec![run("İ", 0.0, true, false)]).count(), "0/0");
        let runs = vec![
            run("e", 0.0, true, false),
            run("\u{301}", 0.0, false, false),
        ];
        assert_eq!(search("e", runs.clone()).count(), "0/0");
        let f = search("e\u{301}", runs);
        assert_eq!(f.count(), "1/1");
        assert_eq!(f.highlight_ranges(false).len(), 2);
        assert_eq!(
            search("👩", vec![run("👩‍💻", 0.0, true, false)]).count(),
            "0/0"
        );
        assert_eq!(
            search("👩‍💻", vec![run("👩‍💻", 0.0, true, false)]).count(),
            "1/1"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn query_and_counter_render_with_system_font_at_multiple_scales() {
        let font = toolbar::load_ui_font([
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ])
        .unwrap();
        for scale in [1.0, 1.5, 2.0] {
            for dark in [false, true] {
                let mut f = search("Rust", vec![run("rust rust", 0.0, true, false)]);
                f.focus();
                let mut before =
                    Pixmap::new((420.0 * scale) as u32, (280.0 * scale) as u32).unwrap();
                f.draw(&mut before, scale, dark, Some(&font));
                f.insert("é😀 long query ".repeat(20).as_str());
                let mut after = Pixmap::new(before.width(), before.height()).unwrap();
                f.draw(&mut after, scale, dark, Some(&font));
                assert_ne!(before.data(), after.data());
                let top = (toolbar::PAGE_TOP * scale) as usize * after.width() as usize * 4;
                assert!(after.data()[..top].iter().all(|b| *b == 0));
            }
        }
    }

    #[test]
    fn repeated_case_insensitive_matches_and_wrapping_are_real() {
        let mut f = search("rust", vec![run("Rust rust RUST", 120.0, true, false)]);
        assert_eq!(f.count(), "1/3");
        f.step(false);
        assert_eq!(f.count(), "2/3");
        f.step(true);
        assert_eq!(f.count(), "1/3");
        f.step(true);
        assert_eq!(f.count(), "3/3");
        f.step(false);
        assert_eq!(f.count(), "1/3");
        assert_eq!(f.scroll_target(90.0), Some(90.0));
        assert_eq!(f.scroll_target(500.0), Some(120.0));
    }
    #[test]
    fn empty_whitespace_and_missing_queries_have_no_results() {
        for q in ["", "   ", "absent"] {
            let mut f = search(q, vec![run("visible", 0.0, true, false)]);
            f.step(false);
            f.step(true);
            assert_eq!(f.count(), "0/0");
            assert!(f.target().is_none());
        }
    }
    #[test]
    fn inline_fragments_and_wrapped_words_join_but_blocks_do_not() {
        let runs = vec![
            run("Hello ", 0.0, true, false),
            run("wo", 0.0, false, false),
            run("rld", 0.0, false, false),
            run("again", 20.0, false, true),
            run("next block", 60.0, true, false),
        ];
        assert_eq!(search("hello world again", runs.clone()).count(), "1/1");
        assert_eq!(search("again next", runs).count(), "0/0");
    }
    #[test]
    fn unicode_lowercase_is_safe_and_not_full_case_folding_or_normalization() {
        let runs = vec![run("CAFÉ café 😀 Straße STRASSE İ", 0.0, true, false)];
        assert_eq!(search("café", runs.clone()).count(), "1/2");
        assert_eq!(search("😀", runs.clone()).count(), "1/1");
        assert_eq!(search("straße", runs.clone()).count(), "1/1");
        assert_eq!(search("cafe\u{301}", runs.clone()).count(), "0/0");
        assert_eq!(search("i\u{307}", runs).count(), "1/1");
        let mut f = FindInPage::default();
        f.insert("é😀");
        f.delete(false);
        assert_eq!(f.query, "é");
        f.end(false);
        f.delete(true);
        assert!(f.query.is_empty());
        f.insert("αβ");
        f.move_caret(-1);
        f.insert("😀");
        assert_eq!(f.query, "α😀β");
        f.select_all();
        f.insert("new");
        assert_eq!(f.query, "new");
    }
    #[test]
    fn layout_replacement_recomputes_counts_and_clamps_selection() {
        let mut f = search("x", vec![run("x x", 0.0, true, false)]);
        f.step(false);
        f.replace_runs(vec![run("x", 300.0, true, false)]);
        assert_eq!(f.count(), "1/1");
        assert_eq!(f.scroll_target(400.0), Some(300.0));
        f.replace_runs(vec![]);
        assert_eq!(f.count(), "0/0");
    }
    #[test]
    fn find_row_reserves_page_space_and_buttons_stay_in_window() {
        for width in [420, 640, 1120] {
            for scale in [1.0, 1.5, 2.0] {
                let before = toolbar::page_rect(width, 720, scale).unwrap();
                let after = toolbar::page_rect_with_find(width, 720, scale, true).unwrap();
                assert_eq!(after.y + after.height, before.y + before.height);
                assert_eq!(after.y - before.y, (HEIGHT as f64 * scale).round() as u32);
            }
            for (action, rect) in FindInPage::regions(width as f32) {
                assert!(rect.left() >= 0.0 && rect.right() <= width as f32);
                assert_eq!(
                    FindInPage::action_at(
                        width as f32,
                        Some(((rect.x() + 1.0) as f64, (rect.y() + 1.0) as f64))
                    ),
                    Some(action)
                );
            }
        }
    }
    #[test]
    fn draw_is_confined_below_toolbar_in_both_themes() {
        let f = search("rust", vec![run("rust", 0.0, true, false)]);
        for dark in [false, true] {
            let mut pixmap = Pixmap::new(420, 280).unwrap();
            f.draw(&mut pixmap, 1.0, dark, None);
            let top = toolbar::PAGE_TOP as usize * 420 * 4;
            assert!(pixmap.data()[..top].iter().all(|b| *b == 0));
            assert!(
                pixmap.data()[top..top + HEIGHT as usize * 420 * 4]
                    .iter()
                    .any(|b| *b != 0)
            );
        }
    }
}

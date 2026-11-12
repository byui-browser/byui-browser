//! Typed state for the DevTools Performance panel.
//!
//! Engine stages report each piece of work they finish as a [`TimelineEvent`]
//! (downloading `style.css`, parsing a stylesheet, painting, ...). The
//! [`PerformanceLog`] collects them in start order and maps each one onto a
//! shared time axis, so the panel can draw a timeline of the page's life.
//!
//! Limitation: events are recorded continuously rather than during an
//! explicit recording session, and only the most recent
//! [`PerformanceLog::MAX_EVENTS`] are kept. Clearing the log starts a fresh
//! timeline.

use std::time::{Duration, Instant};

/// The kind of work an event measures. The panel colors bars by category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    /// Downloading a document, stylesheet, or script.
    Network,
    /// Parsing HTML or CSS source text.
    Parse,
    /// Matching selectors and computing styles for the document.
    Style,
    /// Building the layout tree and computing box geometry.
    Layout,
    /// Building the display list and rasterizing it into pixels.
    Paint,
    /// Running page JavaScript, including event listeners.
    Script,
}

impl EventCategory {
    /// Every category, in the order the panel lists them in its legend.
    pub const ALL: [Self; 6] = [
        Self::Network,
        Self::Parse,
        Self::Style,
        Self::Layout,
        Self::Paint,
        Self::Script,
    ];

    /// A short lowercase name for display, such as `"network"`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Parse => "parse",
            Self::Style => "style",
            Self::Layout => "layout",
            Self::Paint => "paint",
            Self::Script => "script",
        }
    }
}

/// One finished piece of work: what happened, when it started, and how long
/// it took.
///
/// `start` is a monotonic [`Instant`], so events recorded on different threads
/// of the same process share one clock and line up on the timeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// What kind of work this was.
    pub category: EventCategory,
    /// Human-readable description shown in the panel, such as
    /// `"Download style.css"` or `"Parse stylesheet 1"`.
    pub label: String,
    /// When the work started.
    pub start: Instant,
    /// How long the work took.
    pub duration: Duration,
}

impl TimelineEvent {
    /// An event that started at `start` and ran for `duration`.
    pub fn new(
        category: EventCategory,
        label: impl Into<String>,
        start: Instant,
        duration: Duration,
    ) -> Self {
        Self {
            category,
            label: label.into(),
            start,
            duration,
        }
    }

    /// An event that started at `start` and finished now.
    pub fn since(category: EventCategory, label: impl Into<String>, start: Instant) -> Self {
        Self::new(category, label, start, start.elapsed())
    }

    /// Runs `work`, measuring it as an event, and returns its result together
    /// with the event.
    pub fn measure<T>(
        category: EventCategory,
        label: impl Into<String>,
        work: impl FnOnce() -> T,
    ) -> (T, Self) {
        let start = Instant::now();
        let value = work();
        (value, Self::since(category, label, start))
    }

    /// When the work finished.
    pub fn end(&self) -> Instant {
        self.start + self.duration
    }

    /// The duration in fractional milliseconds, for display.
    pub fn duration_ms(&self) -> f64 {
        self.duration.as_secs_f64() * 1000.0
    }
}

/// Session-local event history for the Performance panel.
///
/// Events are kept sorted by start time regardless of the order they were
/// recorded in, since concurrent downloads finish out of order. Once
/// [`Self::MAX_EVENTS`] are stored, recording another drops the oldest.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PerformanceLog {
    events: Vec<TimelineEvent>,
}

impl PerformanceLog {
    /// The most events kept; older events are dropped first.
    pub const MAX_EVENTS: usize = 1000;

    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one event at its place in start order.
    pub fn record(&mut self, event: TimelineEvent) {
        log::trace!(
            "performance event recorded: {} {:?} ({:.2} ms)",
            event.category.name(),
            event.label,
            event.duration_ms()
        );
        let index = self
            .events
            .partition_point(|existing| existing.start <= event.start);
        self.events.insert(index, event);
        if self.events.len() > Self::MAX_EVENTS {
            let excess = self.events.len() - Self::MAX_EVENTS;
            self.events.drain(..excess);
        }
    }

    /// Records every event in `events`.
    pub fn extend(&mut self, events: impl IntoIterator<Item = TimelineEvent>) {
        for event in events {
            self.record(event);
        }
    }

    /// All events, sorted by start time.
    pub fn entries(&self) -> &[TimelineEvent] {
        &self.events
    }

    /// Removes every event, starting a fresh timeline.
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// The start of the timeline: the earliest event's start, if any.
    pub fn origin(&self) -> Option<Instant> {
        self.events.first().map(|event| event.start)
    }

    /// Time from the earliest start to the latest end. Zero when empty.
    pub fn span(&self) -> Duration {
        let Some(origin) = self.origin() else {
            return Duration::ZERO;
        };
        self.events
            .iter()
            .map(|event| event.end().saturating_duration_since(origin))
            .max()
            .unwrap_or_default()
    }

    /// How long after the timeline's origin `event` started.
    pub fn offset(&self, event: &TimelineEvent) -> Duration {
        self.origin().map_or(Duration::ZERO, |origin| {
            event.start.saturating_duration_since(origin)
        })
    }

    /// Where `event` sits on the timeline, as `(start, width)` fractions of
    /// [`Self::span`], each in `0.0..=1.0`. The panel multiplies these by its
    /// timeline width in pixels. An empty or zero-length timeline puts every
    /// event at `(0.0, 0.0)`.
    pub fn bar_fraction(&self, event: &TimelineEvent) -> (f64, f64) {
        let span = self.span().as_secs_f64();
        if span <= 0.0 {
            return (0.0, 0.0);
        }
        let start = (self.offset(event).as_secs_f64() / span).clamp(0.0, 1.0);
        let width = (event.duration.as_secs_f64() / span).clamp(0.0, 1.0 - start);
        (start, width)
    }

    /// Total time spent in `category`, summed over its events. Overlapping
    /// events (such as parallel downloads) are each counted in full.
    pub fn total(&self, category: EventCategory) -> Duration {
        self.events
            .iter()
            .filter(|event| event.category == category)
            .map(|event| event.duration)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(origin: Instant, start_ms: u64, duration_ms: u64, label: &str) -> TimelineEvent {
        TimelineEvent::new(
            EventCategory::Network,
            label,
            origin + Duration::from_millis(start_ms),
            Duration::from_millis(duration_ms),
        )
    }

    #[test]
    fn keeps_events_in_start_order_when_recorded_out_of_order() {
        let origin = Instant::now();
        let mut log = PerformanceLog::new();
        log.record(at(origin, 10, 5, "second"));
        log.record(at(origin, 0, 32, "first"));
        log.record(at(origin, 40, 2, "third"));
        let labels: Vec<_> = log.entries().iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["first", "second", "third"]);
        assert_eq!(log.origin(), Some(origin));
        assert_eq!(log.span(), Duration::from_millis(42));
        assert_eq!(log.offset(&log.entries()[1]), Duration::from_millis(10));
    }

    #[test]
    fn bar_fractions_map_events_onto_the_span() {
        let origin = Instant::now();
        let mut log = PerformanceLog::new();
        log.record(at(origin, 0, 50, "a"));
        log.record(at(origin, 50, 50, "b"));
        assert_eq!(log.bar_fraction(&log.entries()[0]), (0.0, 0.5));
        assert_eq!(log.bar_fraction(&log.entries()[1]), (0.5, 0.5));

        let empty = PerformanceLog::new();
        assert_eq!(empty.span(), Duration::ZERO);
        assert_eq!(empty.bar_fraction(&at(origin, 0, 0, "x")), (0.0, 0.0));
    }

    #[test]
    fn caps_history_by_dropping_the_oldest_events() {
        let origin = Instant::now();
        let mut log = PerformanceLog::new();
        for index in 0..PerformanceLog::MAX_EVENTS as u64 + 5 {
            log.record(at(origin, index, 1, "event"));
        }
        assert_eq!(log.entries().len(), PerformanceLog::MAX_EVENTS);
        assert_eq!(
            log.origin(),
            Some(origin + Duration::from_millis(5)),
            "the five oldest events were dropped"
        );
        log.clear();
        assert!(log.entries().is_empty());
    }

    #[test]
    fn measure_times_work_and_totals_sum_by_category() {
        let (value, event) = TimelineEvent::measure(EventCategory::Paint, "Paint", || 7);
        assert_eq!(value, 7);
        assert_eq!(event.label, "Paint");
        assert_eq!(event.end(), event.start + event.duration);

        let origin = Instant::now();
        let mut log = PerformanceLog::new();
        log.record(at(origin, 0, 3, "a"));
        log.record(at(origin, 1, 4, "b"));
        log.record(TimelineEvent::new(
            EventCategory::Paint,
            "Paint",
            origin,
            Duration::from_millis(2),
        ));
        assert_eq!(log.total(EventCategory::Network), Duration::from_millis(7));
        assert_eq!(log.total(EventCategory::Paint), Duration::from_millis(2));
        assert_eq!(log.total(EventCategory::Script), Duration::ZERO);
    }
}

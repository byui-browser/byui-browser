//! Platform-independent state for recorded browser visits.

/// Owns the URLs visited during a browser session, in visit order.
///
/// The most recently recorded visit is the current URL. Repeated visits are
/// retained. This in-memory state does not validate URLs, navigate, fetch pages,
/// persist history, or implement Back/Forward navigation. Callers decide when
/// a visit has completed and should be recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BrowserState {
    visited_urls: Vec<String>,
}

impl BrowserState {
    /// Creates an empty session with no current URL or recorded visits.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies `url` into the visit history and makes it the current URL.
    ///
    /// Stores the supplied text unchanged, including internal URLs such as
    /// `byui://welcome`. Every call records a visit, even for a repeated URL.
    pub fn record_visit(&mut self, url: &str) {
        self.visited_urls.push(url.to_owned());
    }

    /// Borrows the most recently visited URL, or `None` before the first visit.
    pub fn current_url(&self) -> Option<&str> {
        self.visited_urls.last().map(String::as_str)
    }

    /// Borrows all recorded URLs in visit order, including repeated visits.
    pub fn visited_urls(&self) -> &[String] {
        &self.visited_urls
    }
}

#[cfg(test)]
mod tests {
    use super::BrowserState;

    #[test]
    fn empty_state_has_no_current_url_or_visits() {
        let state = BrowserState::new();
        assert_eq!(state.current_url(), None);
        assert!(state.visited_urls().is_empty());
        assert_eq!(state, BrowserState::default());
    }

    #[test]
    fn first_visit_records_an_owned_internal_url() {
        let mut state = BrowserState::new();
        let mut url = String::from("byui://welcome");
        state.record_visit(&url);
        url.clear();

        assert_eq!(state.current_url(), Some("byui://welcome"));
        assert_eq!(state.visited_urls(), &["byui://welcome"]);
    }

    #[test]
    fn multiple_visits_preserve_order_and_update_current_url() {
        let mut state = BrowserState::new();
        state.record_visit("byui://welcome");
        state.record_visit("https://example.com/");
        state.record_visit("https://www.byui.edu/");

        assert_eq!(state.current_url(), Some("https://www.byui.edu/"));
        assert_eq!(
            state.visited_urls(),
            &[
                "byui://welcome",
                "https://example.com/",
                "https://www.byui.edu/",
            ]
        );
    }

    #[test]
    fn repeated_urls_are_kept_as_separate_visits() {
        let mut state = BrowserState::new();
        state.record_visit("https://example.com/");
        state.record_visit("https://example.com/");
        state.record_visit("byui://welcome");
        state.record_visit("https://example.com/");

        assert_eq!(state.current_url(), Some("https://example.com/"));
        assert_eq!(
            state.visited_urls(),
            &[
                "https://example.com/",
                "https://example.com/",
                "byui://welcome",
                "https://example.com/",
            ]
        );
    }
}

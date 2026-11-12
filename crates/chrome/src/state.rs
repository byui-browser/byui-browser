//! Platform-independent state for recorded browser visits.

/// Owns the URLs visited during a browser session, in visit order.
///
/// The most recently recorded visit is the current URL. Repeated visits are
/// retained. This in-memory state does not validate URLs, navigate, fetch pages,
/// persist history. Callers decide when
/// a visit has completed and should be recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BrowserState {
    visited_urls: Vec<String>,
    current_index: Option<usize>,
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
        if let Some(index) = self.current_index {
            self.visited_urls.truncate(index + 1);
        }
        self.visited_urls.push(url.to_owned());
        self.current_index = Some(self.visited_urls.len() - 1);
        log::debug!(
            "visit recorded: {url} ({} in history)",
            self.visited_urls.len()
        );
    }

    /// Replaces the current visit with `url`, or records it when history is
    /// empty. Used by Location's `replace()` navigation behavior.
    pub fn replace_current(&mut self, url: &str) {
        if let Some(index) = self.current_index {
            self.visited_urls[index] = url.to_owned();
        } else if let Some(current) = self.visited_urls.last_mut() {
            *current = url.to_owned();
            self.current_index = Some(self.visited_urls.len() - 1);
        } else {
            self.visited_urls.push(url.to_owned());
            self.current_index = Some(0);
        }
    }

    /// Borrows the most recently visited URL, or `None` before the first visit.
    pub fn current_url(&self) -> Option<&str> {
        self.current_index
            .and_then(|index| self.visited_urls.get(index))
            .map(String::as_str)
    }

    /// Borrows all recorded URLs in visit order, including repeated visits.
    pub fn visited_urls(&self) -> &[String] {
        &self.visited_urls
    }

    pub fn can_go_back(&self) -> bool {
        self.current_index.is_some_and(|index| index > 0)
    }

    pub fn can_go_forward(&self) -> bool {
        self.current_index
            .is_some_and(|index| index + 1 < self.visited_urls.len())
    }

    pub fn go_back(&mut self) -> Option<String> {
        let index = self.current_index?;
        (index > 0).then(|| {
            let index = index - 1;
            self.current_index = Some(index);
            self.visited_urls[index].clone()
        })
    }

    pub fn go_forward(&mut self) -> Option<String> {
        let index = self.current_index?;
        (index + 1 < self.visited_urls.len()).then(|| {
            let index = index + 1;
            self.current_index = Some(index);
            self.visited_urls[index].clone()
        })
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
    fn replace_current_replaces_only_the_latest_visit() {
        let mut state = BrowserState::new();
        state.record_visit("https://first.example/");
        state.record_visit("https://second.example/");
        state.replace_current("https://replacement.example/");

        assert_eq!(
            state.visited_urls(),
            ["https://first.example/", "https://replacement.example/"]
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

    #[test]
    fn back_and_forward_move_without_losing_history() {
        let mut state = BrowserState::new();
        state.record_visit("https://a.example/");
        state.record_visit("https://b.example/");
        state.record_visit("https://c.example/");

        assert_eq!(state.go_back().as_deref(), Some("https://b.example/"));
        assert!(state.can_go_back());
        assert!(state.can_go_forward());
        assert_eq!(state.go_back().as_deref(), Some("https://a.example/"));
        assert!(!state.can_go_back());
        assert_eq!(state.go_forward().as_deref(), Some("https://b.example/"));
        assert_eq!(state.current_url(), Some("https://b.example/"));
    }

    #[test]
    fn new_navigation_truncates_forward_history() {
        let mut state = BrowserState::new();
        state.record_visit("https://a.example/");
        state.record_visit("https://b.example/");
        let _ = state.go_back();
        state.record_visit("https://new.example/");

        assert_eq!(
            state.visited_urls(),
            ["https://a.example/", "https://new.example/"]
        );
        assert!(!state.can_go_forward());
    }
}

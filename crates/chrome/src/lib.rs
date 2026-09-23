//! Browser chrome / UX: tabs, address bar, menus, settings, bookmarks.
//!
//! **Owning team**: Browser UX Team
//!
//! Platform-specific shells live under `platforms/`; this crate holds
//! shared UI logic that those shells host.

#![forbid(unsafe_code)]

/// Label for a newly opened tab before navigating to a page.
pub const DEFAULT_TAB_TITLE: &str = "New Tab";

/// Handle for an open tab.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u32);

/// One open tab.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    /// Text currently shown in the address bar.
    pub url: String,
}

/// Owns the tab strip and tracks which tab is active.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TabManager {
    tabs: Vec<Tab>,
    active: Option<TabId>,
}

impl TabManager {
    /// A window with no tabs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens a new tab at `url` and makes it active.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn open(&mut self, url: &str) -> TabId {
        todo!("TODO(chrome): open tab for {url:?}")
    }

    /// Closes a tab. Closing the active tab must activate a neighbor.
    ///
    /// Currently only handles the empty window.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn close(&mut self, id: TabId) {
        if self.tabs.is_empty() {
            return;
        }
        todo!("TODO(chrome): close {id:?}")
    }

    /// The active tab, if any tab is open.
    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    /// Number of open tabs.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_has_no_tabs_and_no_active_tab() {
        let manager = TabManager::new();
        assert_eq!(manager.tab_count(), 0);
        assert_eq!(manager.active(), None);
    }

    #[test]
    fn closing_in_empty_window_is_a_no_op() {
        let mut manager = TabManager::new();
        manager.close(TabId(0));
        assert_eq!(manager.tab_count(), 0);
    }
}

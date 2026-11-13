//! Browser chrome / UX: tabs, address bar, menus, settings, bookmarks.
//!
//! **Owning team**: Browser UX Team
//!
//! Platform-specific shells live under `platforms/`; this crate holds
//! shared UI logic that those shells host.

#![forbid(unsafe_code)]

mod state;

pub use state::BrowserState;

/// Label for a newly opened tab before navigating to a page.
pub const DEFAULT_TAB_TITLE: &str = "New Tab";

/// Handle for a tab, unique within the manager that allocated it.
///
/// Managers never reuse IDs, including after a tab closes. IDs from different
/// managers (or independently used clones) are not globally unique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u32);

/// Metadata for one open tab; this does not own a loaded page or page session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// Stable ID allocated by the owning manager.
    pub id: TabId,
    /// Text currently shown in the address bar.
    pub url: String,
}

/// Owns ordered tab metadata and tracks which tab is active.
///
/// New tabs append in opening order. Closing a tab preserves the relative order
/// of survivors. This is state only: it does not draw a strip, fetch URLs, manage
/// per-tab history, or create browser page sessions. The shared shell must supply
/// those services when it connects this manager to the desktop window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TabManager {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    // One past the last issued u32 ID may be represented, without wrapping.
    next_id: u64,
}

impl TabManager {
    /// A window with no tabs.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a tab, copies `url` unchanged, and makes the new tab active.
    ///
    /// Does not validate or load the URL. IDs are never recycled.
    ///
    /// # Panics
    ///
    /// Panics before changing state if all `u32` IDs have been issued during
    /// this manager's lifetime. The existing infallible signature is preserved;
    /// wrapping would let stale IDs refer to unrelated tabs.
    pub fn open(&mut self, url: &str) -> TabId {
        let id = TabId(u32::try_from(self.next_id).expect("tab IDs exhausted"));
        self.tabs.push(Tab {
            id,
            url: url.to_owned(),
        });
        self.next_id += 1;
        self.active = Some(id);
        id
    }

    /// Activates an existing tab without changing its URL or tab ordering.
    ///
    /// Returns `true` for an existing ID, including the already active tab.
    /// Unknown or closed IDs return `false` and leave the manager unchanged.
    pub fn activate(&mut self, id: TabId) -> bool {
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return false;
        }
        self.active = Some(id);
        true
    }

    /// Borrows tab metadata in opening order, excluding closed tabs.
    ///
    /// The slice is read-only so callers cannot invalidate IDs or selection.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Updates address metadata for an existing tab without navigating or activating it.
    /// Returns `false` for an unknown ID and leaves other tabs unchanged.
    pub fn update_url(&mut self, id: TabId, url: &str) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return false;
        };
        tab.url = url.to_owned();
        true
    }

    /// Closes a tab, preserving the order of survivors.
    ///
    /// Closing the active tab activates its right neighbor, or its left neighbor
    /// when it was last. Closing the final tab leaves no active tab. Closing an
    /// inactive tab does not change selection. Unknown IDs are ignored.
    pub fn close(&mut self, id: TabId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.active == Some(id) {
            self.active = self
                .tabs
                .get(index)
                .or_else(|| self.tabs.last())
                .map(|tab| tab.id);
        }
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
    fn final_id_is_valid_and_exhaustion_does_not_wrap_or_change_state() {
        let mut manager = TabManager {
            next_id: u64::from(u32::MAX),
            ..TabManager::new()
        };
        let last = manager.open("last ID");
        assert_eq!(last, TabId(u32::MAX));
        let before = manager.clone();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| manager.open("overflow")));
        assert!(result.is_err());
        assert_eq!(manager, before);
        manager.close(last);
        assert_eq!(manager.active(), None);
    }

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

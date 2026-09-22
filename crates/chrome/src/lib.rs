//! Browser chrome / UX: tabs, address bar, menus, settings, bookmarks.
//!
//! **Owning team**: Browser UX Team
//!
//! Platform-specific shells live under `platforms/`; this crate holds
//! shared UI logic that those shells host.

#![forbid(unsafe_code)]

/// Label for a newly opened tab before navigating to a page.
pub const DEFAULT_TAB_TITLE: &str = "New Tab";

// TODO(chrome): Implement browser UI and window management.

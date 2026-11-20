//! Public-contract tests for `chrome`.
//!
//! Run with `cargo test -p chrome --test contract`.

use chrome::{TabId, TabManager};

#[test]
fn opening_a_tab_makes_it_active() {
    let mut manager = TabManager::new();
    let id = manager.open("https://byui.instructure.com/");
    assert_eq!(manager.active(), Some(id));
    assert_eq!(manager.tab_count(), 1);
}

#[test]
fn closing_active_tab_activates_a_neighbor() {
    let mut manager = TabManager::new();
    let first = manager.open("about:blank");
    let second = manager.open("about:blank");
    manager.close(second);
    assert_eq!(manager.active(), Some(first));
}

#[test]
fn closing_last_tab_leaves_no_active_tab() {
    let mut manager = TabManager::new();
    let only = manager.open("about:blank");
    manager.close(only);
    assert_eq!(manager.active(), None);
    assert_eq!(manager.tab_count(), 0);
}

#[test]
fn opening_multiple_tabs_keeps_order_and_owned_urls() {
    let mut manager = TabManager::new();
    let mut url = String::from("byui://welcome");
    let first = manager.open(&url);
    url.clear();
    let second = manager.open("https://example.com/");
    let third = manager.open("https://example.com/");
    assert_eq!(
        manager.tabs().iter().map(|tab| tab.id).collect::<Vec<_>>(),
        [first, second, third]
    );
    assert_eq!(manager.tabs()[0].url, "byui://welcome");
    assert_eq!(manager.tabs()[1].url, "https://example.com/");
    assert_eq!(manager.tabs()[2].url, "https://example.com/");
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert_ne!(first, third);
    assert_eq!(manager.active(), Some(third));
}

#[test]
fn switching_tabs_preserves_urls_order_and_count() {
    let mut manager = TabManager::new();
    let first = manager.open("first");
    let second = manager.open("second");
    let before = manager.tabs().to_vec();
    assert!(manager.activate(first));
    assert_eq!(manager.active(), Some(first));
    assert!(manager.activate(first));
    assert!(manager.activate(second));
    assert_eq!(manager.active(), Some(second));
    assert_eq!(manager.tabs(), before);
    assert_eq!(manager.tab_count(), 2);
}

#[test]
fn closing_inactive_tabs_keeps_active_selection() {
    let mut manager = TabManager::new();
    let first = manager.open("first");
    let second = manager.open("second");
    let third = manager.open("third");
    manager.close(second);
    assert_eq!(manager.active(), Some(third));
    assert_eq!(
        manager.tabs().iter().map(|t| t.id).collect::<Vec<_>>(),
        [first, third]
    );
    manager.close(first);
    assert_eq!(manager.active(), Some(third));
}

#[test]
fn closing_active_middle_tab_prefers_right_neighbor() {
    let mut manager = TabManager::new();
    let first = manager.open("first");
    let middle = manager.open("middle");
    let last = manager.open("last");
    assert!(manager.activate(middle));
    manager.close(middle);
    assert_eq!(manager.active(), Some(last));
    assert_eq!(
        manager.tabs().iter().map(|t| t.id).collect::<Vec<_>>(),
        [first, last]
    );
    assert!(manager.activate(first));
    manager.close(first);
    assert_eq!(manager.active(), Some(last));
}

#[test]
fn invalid_and_closed_ids_leave_all_state_unchanged() {
    let mut manager = TabManager::new();
    let empty = manager.clone();
    assert!(!manager.activate(TabId(u32::MAX)));
    manager.close(TabId(u32::MAX));
    assert_eq!(manager, empty);
    let old = manager.open("old");
    manager.close(old);
    let live = manager.open("live");
    let before = manager.clone();
    for invalid in [old, TabId(u32::MAX)] {
        assert!(!manager.activate(invalid));
        manager.close(invalid);
        assert_eq!(manager, before);
    }
    assert_ne!(old, live);
}

#[test]
fn opening_after_closure_appends_without_reusing_ids() {
    let mut manager = TabManager::new();
    let first = manager.open("first");
    let second = manager.open("second");
    manager.close(first);
    let third = manager.open("third");
    assert_ne!(first, third);
    assert_ne!(second, third);
    assert_eq!(
        manager.tabs().iter().map(|t| t.id).collect::<Vec<_>>(),
        [second, third]
    );
    manager.close(second);
    manager.close(third);
    assert!(manager.tabs().is_empty());
    assert_eq!(manager.active(), None);
    let fourth = manager.open("fourth");
    assert!(![first, second, third].contains(&fourth));
    assert_eq!(manager.active(), Some(fourth));
    assert_eq!(manager.tab_count(), 1);
}

#[test]
fn address_metadata_updates_without_switching_or_reordering_tabs() {
    let mut manager = TabManager::new();
    let first = manager.open("first");
    let second = manager.open("second");
    assert!(manager.update_url(first, "updated"));
    assert_eq!(manager.active(), Some(second));
    assert_eq!(manager.tabs()[0].url, "updated");
    assert_eq!(manager.tabs()[1].url, "second");
    assert!(!manager.update_url(TabId(u32::MAX), "ignored"));
}

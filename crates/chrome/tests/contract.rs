//! Public-contract tests for `chrome`.
//!
//! Run ignored tests with `cargo test -p chrome -- --ignored` to see the backlog.

use chrome::TabManager;

#[test]
#[ignore = "TODO(chrome): tab management not implemented"]
fn opening_a_tab_makes_it_active() {
    let mut manager = TabManager::new();
    let id = manager.open("https://byui.instructure.com/");
    assert_eq!(manager.active(), Some(id));
    assert_eq!(manager.tab_count(), 1);
}

#[test]
#[ignore = "TODO(chrome): tab management not implemented"]
fn closing_active_tab_activates_a_neighbor() {
    let mut manager = TabManager::new();
    let first = manager.open("about:blank");
    let second = manager.open("about:blank");
    manager.close(second);
    assert_eq!(manager.active(), Some(first));
}

#[test]
#[ignore = "TODO(chrome): tab management not implemented"]
fn closing_last_tab_leaves_no_active_tab() {
    let mut manager = TabManager::new();
    let only = manager.open("about:blank");
    manager.close(only);
    assert_eq!(manager.active(), None);
    assert_eq!(manager.tab_count(), 0);
}

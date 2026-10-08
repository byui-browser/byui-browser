//! Public-contract tests for `webapis`.
//!
//! Run ignored tests with `cargo test -p webapis -- --ignored` to see the backlog.

use webapis::TimerQueue;

#[test]
#[ignore = "TODO(webapis): timers not implemented"]
fn timers_fire_in_due_order_not_insertion_order() {
    let mut queue = TimerQueue::new();
    let late = queue.schedule(100);
    let early = queue.schedule(10);
    assert_eq!(queue.pending_count(), 2);
    assert_eq!(queue.pop_next_due(), Some(early));
    assert_eq!(queue.pop_next_due(), Some(late));
    assert_eq!(queue.pop_next_due(), None);
}

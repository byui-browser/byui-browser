//! Bindings between JavaScript and DOM/platform APIs.
//!
//! **Owning team**: JS APIs (Web APIs) Team
//!
//! Window, Document, Element, Fetch, timers, events, and related APIs.
//! DOM mutations from JS must go through a controlled mutation interface
//! so style/layout invalidation stays correct.

#![forbid(unsafe_code)]

use common::ids::NodeId;

/// Script-visible view of a DOM element.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub node: NodeId,
    /// Value of the `id` attribute, if any.
    pub id_attr: Option<String>,
}

/// Script-visible `document` object.
///
/// Wraps the HTML team's DOM; the real binding will hold a handle, not a copy.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Document {
    elements: Vec<Element>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// `document.getElementById(id)`.
    ///
    /// Currently only handles the empty document.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
    // signature however your crate's public API needs.
    pub fn get_element_by_id(&self, id: &str) -> Option<&Element> {
        if self.elements.is_empty() {
            return None;
        }
        todo!(
            "TODO(webapis): look up #{id} among {} elements",
            self.elements.len()
        )
    }
}

/// Handle returned by `setTimeout`.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u32);

/// Pending `setTimeout` / `setInterval` callbacks, ordered by due time.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimerQueue {
    pending: Vec<(TimerId, u64)>,
}

impl TimerQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of timers not yet fired.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// `setTimeout(callback, delay_ms)`: schedules a callback.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
    // signature however your crate's public API needs.
    pub fn schedule(&mut self, delay_ms: u64) -> TimerId {
        todo!("TODO(webapis): schedule timer at +{delay_ms}ms")
    }

    /// Removes and returns the timer due soonest, if any.
    ///
    /// Currently only handles the empty queue.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn pop_next_due(&mut self) -> Option<TimerId> {
        if self.pending.is_empty() {
            return None;
        }
        todo!(
            "TODO(webapis): pop earliest of {} timers",
            self.pending.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_no_element_by_id() {
        assert_eq!(Document::new().get_element_by_id("main"), None);
    }

    #[test]
    fn new_timer_queue_is_empty() {
        let mut queue = TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}

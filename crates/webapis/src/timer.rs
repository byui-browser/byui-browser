//! Timer handles for `setTimeout` and `setInterval`.

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
    fn new_timer_queue_is_empty() {
        let mut queue = TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}

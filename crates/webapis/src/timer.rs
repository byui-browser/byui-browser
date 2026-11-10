//! Timer handles for `setTimeout` and `setInterval`.

/// Positive integer handle returned by `setTimeout`, in the ID space shared
/// by timers within a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(
    /// Numeric timer handle, suitable for equality checks and cancellation.
    pub u32,
);

/// Timer handles ordered by their scheduled delay.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimerQueue {
    pending: Vec<(TimerId, u64)>,
    next_id: u32,
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

    /// Adds a timer with a delay in milliseconds and returns its handle.
    pub fn schedule(&mut self, delay_ms: u64) -> TimerId {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let id = TimerId(self.next_id);
        self.pending.push((id, delay_ms));
        id
    }

    /// Removes and returns the timer with the smallest scheduled delay.
    pub fn pop_next_due(&mut self) -> Option<TimerId> {
        let index = self
            .pending
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, due))| *due)
            .map(|(index, _)| index)?;
        Some(self.pending.remove(index).0)
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

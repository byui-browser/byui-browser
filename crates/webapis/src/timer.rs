//! Timer handles and scheduling state for Window timers.

use std::time::{Duration, Instant};

/// Handle returned by `setTimeout()` and `setInterval()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingTimer {
    id: TimerId,
    due: Instant,
    interval: Option<Duration>,
}

/// Pending timer deadlines, ordered by their next due time.
///
/// The queue owns scheduling only; the Window binding owns JavaScript
/// callbacks and invokes [`TimerQueue::pop_due`] from its event-loop pump.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimerQueue {
    next_id: u32,
    pending: Vec<PendingTimer>,
}

impl TimerQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            pending: Vec::new(),
        }
    }

    /// Returns the number of timers that have not been cancelled or fired.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Schedules a one-shot timer after `delay_ms` milliseconds.
    pub fn schedule(&mut self, delay_ms: u64) -> TimerId {
        self.schedule_kind(delay_ms, None)
    }

    /// Schedules a repeating timer after `delay_ms` milliseconds.
    pub fn schedule_interval(&mut self, delay_ms: u64) -> TimerId {
        self.schedule_kind(delay_ms, Some(Duration::from_millis(delay_ms)))
    }

    fn schedule_kind(&mut self, delay_ms: u64, interval: Option<Duration>) -> TimerId {
        let id = self.allocate_id();
        self.pending.push(PendingTimer {
            id,
            due: Instant::now() + Duration::from_millis(delay_ms),
            interval,
        });
        id
    }

    fn allocate_id(&mut self) -> TimerId {
        loop {
            let id = TimerId(self.next_id.max(1));
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if !self.pending.iter().any(|timer| timer.id == id) {
                return id;
            }
        }
    }

    /// Cancels a timer. Returns whether an active timer was removed.
    pub fn cancel(&mut self, id: TimerId) -> bool {
        let old_len = self.pending.len();
        self.pending.retain(|timer| timer.id != id);
        self.pending.len() != old_len
    }

    /// Removes the earliest timer, regardless of whether its deadline arrived.
    pub fn pop_next_due(&mut self) -> Option<TimerId> {
        let index = self
            .pending
            .iter()
            .enumerate()
            .min_by_key(|(_, timer)| timer.due)
            .map(|(index, _)| index)?;
        Some(self.pending.remove(index).id)
    }

    /// Removes the earliest timer whose deadline is no later than `now`.
    /// Repeating timers are re-queued for their next interval before return.
    pub fn pop_due(&mut self, now: Instant) -> Option<TimerId> {
        let index = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, timer)| timer.due <= now)
            .min_by_key(|(_, timer)| timer.due)
            .map(|(index, _)| index)?;
        let timer = self.pending.remove(index);
        if let Some(interval) = timer.interval {
            self.pending.push(PendingTimer {
                id: timer.id,
                due: now + interval,
                interval: Some(interval),
            });
        }
        Some(timer.id)
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

    #[test]
    fn timers_fire_in_due_order_and_intervals_requeue() {
        let mut queue = TimerQueue::new();
        let late = queue.schedule(100);
        let early = queue.schedule(10);
        let repeating = queue.schedule_interval(10);
        let now = Instant::now() + Duration::from_millis(20);
        assert_eq!(queue.pop_due(now), Some(early));
        assert_eq!(queue.pop_due(now), Some(repeating));
        assert_eq!(queue.pending_count(), 2);
        assert!(queue.cancel(repeating));
        assert_eq!(queue.pop_next_due(), Some(late));
        assert_eq!(queue.pop_next_due(), None);
    }
}

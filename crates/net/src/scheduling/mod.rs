//! Request concurrency admission.

mod scheduler;

pub(crate) use scheduler::{RequestPriority, RequestScheduler};

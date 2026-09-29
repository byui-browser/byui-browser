//! Network admission and concurrency control.
//!
//! The scheduler is intentionally behind the controller API. This lets the
//! browser add document-aware priorities, cancellation, or per-host limits
//! without changing callers of [`RequestController::fetch`](crate::RequestController::fetch).

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{error::RequestError, request::Request, response::Response, transport::Transport};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RequestPriority {
    Background,
    Low,
    #[default]
    Normal,
    High,
    Highest,
}

#[derive(Clone)]
pub(crate) struct RequestScheduler {
    transport: Arc<dyn Transport>,
    permits: Arc<Semaphore>,
}

impl RequestScheduler {
    /// Creates an admission controller around one shared transport.
    ///
    /// A zero configured limit is treated as one so a misconfigured browser
    /// cannot deadlock every request permanently.
    pub(crate) fn new(transport: Arc<dyn Transport>, max_in_flight: usize) -> Self {
        Self {
            transport,
            permits: Arc::new(Semaphore::new(max_in_flight.max(1))),
        }
    }

    pub(crate) async fn submit(
        &self,
        request: Request,
        _priority: RequestPriority,
    ) -> Result<Response, RequestError> {
        // Holding the permit for the entire transport future bounds active
        // network work, rather than merely bounding task submission.
        let _permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RequestError::SchedulerClosed)?;
        self.transport.send(request).await
    }
}

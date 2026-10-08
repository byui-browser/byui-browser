//! Network admission and concurrency control.
//!
//! The scheduler is intentionally behind the controller API. This lets the
//! browser add document-aware priorities, cancellation, or per-host limits
//! without changing callers of [`RequestController::fetch`](crate::RequestController::fetch).

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{
    error::RequestError, request::PreparedRequest, response::StreamingResponse,
    transport::Transport,
};

/// Relative importance assigned to a request by the network scheduler.
///
/// The ordering leaves room for document-aware scheduling. The current
/// scheduler records the value but does not yet use it to reorder requests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RequestPriority {
    /// Background work that may yield to user-visible requests.
    Background,
    /// Low-urgency work.
    Low,
    #[default]
    /// Ordinary network work.
    Normal,
    /// User-visible work that should be preferred when scheduling is added.
    High,
    /// The most urgent request class.
    Highest,
}

#[derive(Clone)]
/// Limits the number of requests actively using the shared transport.
///
/// Clones share both the transport and semaphore, so the limit applies to the
/// controller as a whole rather than separately to each cloned handle.
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

    /// Admits a request and keeps its permit with the response body.
    ///
    /// The permit is deliberately transferred into [`ResponseBody`](crate::ResponseBody)
    /// after response headers arrive. It is released only when the body is
    /// fully consumed or dropped.
    pub(crate) async fn submit(
        &self,
        request: PreparedRequest,
        _priority: RequestPriority,
    ) -> Result<StreamingResponse, RequestError> {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| RequestError::SchedulerClosed)?;
        let mut response = self.transport.send(request).await?;
        response.body.attach_permit(permit);
        Ok(response)
    }
}

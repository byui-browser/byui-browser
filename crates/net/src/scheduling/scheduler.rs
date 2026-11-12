//! Network admission and concurrency control.
//!
//! The scheduler is intentionally behind the controller API. This lets the
//! browser add document-aware priorities, cancellation, or per-host limits
//! without changing callers of [`RequestController::fetch`](crate::RequestController::fetch).

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{
    api::{
        diagnostics::{NetworkEvent, NetworkPhase, SharedNetworkObserver},
        error::RequestError,
        request::PreparedRequest,
        response::InternalResponse,
    },
    transport::Transport,
};

/// Relative importance assigned to a request by the network scheduler.
///
/// The current scheduler records the value but does not yet use it to reorder
/// requests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RequestPriority {
    /// Low-urgency work.
    Low,
    #[default]
    /// Ordinary network work.
    Normal,
    /// User-visible work that should be preferred when scheduling is added.
    High,
}

#[derive(Clone)]
/// Limits the number of requests actively using the shared transport.
///
/// Clones share both the transport and semaphore, so the limit applies to the
/// controller as a whole rather than separately to each cloned handle.
pub(crate) struct RequestScheduler {
    transport: Arc<dyn Transport>,
    permits: Arc<Semaphore>,
    observer: SharedNetworkObserver,
}

impl RequestScheduler {
    /// Creates an admission controller around one shared transport.
    ///
    /// A zero configured limit is treated as one so a misconfigured browser
    /// cannot deadlock every request permanently.
    pub(crate) fn new(
        transport: Arc<dyn Transport>,
        max_in_flight: usize,
        observer: SharedNetworkObserver,
    ) -> Self {
        Self {
            transport,
            permits: Arc::new(Semaphore::new(max_in_flight.max(1))),
            observer,
        }
    }

    /// Admits a request and keeps its permit with the response body.
    ///
    /// The permit is deliberately transferred into [`Body`](crate::Body)
    /// after response headers arrive. It is released only when the body is
    /// fully consumed or dropped.
    pub(crate) async fn submit(
        &self,
        request: PreparedRequest,
        _priority: RequestPriority,
    ) -> Result<InternalResponse, RequestError> {
        if request.request.signal.is_aborted() {
            return Err(RequestError::Aborted);
        }
        self.observe(&request, NetworkPhase::QueueStarted, None);
        log::trace!(
            "scheduler admitting {} {} ({} permits available)",
            request.request.method,
            crate::log_url::loggable(request.url.as_str()),
            self.permits.available_permits()
        );
        let permit = self.permits.clone().acquire_owned();
        let permit = tokio::select! {
            _ = request.request.signal.cancelled() => return Err(RequestError::Aborted),
            permit = permit => permit.map_err(|_| {
                log::error!("request scheduler is closed");
                RequestError::SchedulerClosed
            })?,
        };
        if request.request.signal.is_aborted() {
            return Err(RequestError::Aborted);
        }
        self.observe(&request, NetworkPhase::QueueAdmitted, None);
        self.observe(&request, NetworkPhase::TransportStarted, None);
        let request_id = request.request.id;
        let url = request.url.to_string();
        let redirect_count = request.request.redirect_count;
        let mut response = self.transport.send(request).await?;
        self.observe_parts(
            request_id,
            &url,
            redirect_count,
            NetworkPhase::ResponseHeaders,
            Some(response.status.as_u16()),
        );
        response.body.attach_permit(permit);
        Ok(response)
    }

    fn observe(&self, request: &PreparedRequest, phase: NetworkPhase, status: Option<u16>) {
        self.observe_parts(
            request.request.id,
            request.url.as_str(),
            request.request.redirect_count,
            phase,
            status,
        );
    }

    fn observe_parts(
        &self,
        request_id: u64,
        url: &str,
        redirect_count: usize,
        phase: NetworkPhase,
        status: Option<u16>,
    ) {
        if let Some(observer) = &self.observer {
            observer.observe(NetworkEvent::now(
                request_id,
                phase,
                url,
                status,
                false,
                redirect_count,
            ));
        }
    }
}

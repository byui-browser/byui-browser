//! Typed state for the DevTools Network panel.

/// The broad resource category shown by the Network panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Stylesheet,
    Script,
    Image,
    Other,
}

/// One completed resource request suitable for rendering in a network table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestEntry {
    /// Redacted request URL.
    pub url: String,
    /// HTTP status, or `None` when the request failed before headers arrived.
    pub status: Option<u16>,
    /// Resource class assigned by the browser loader.
    pub resource_type: ResourceType,
    /// Total time from controller acceptance through response return, in milliseconds.
    pub duration_ms: u64,
    /// Time waiting for a shared transport slot, in milliseconds.
    pub queue_ms: Option<u64>,
    /// Time from transport start until response headers, in milliseconds.
    pub headers_ms: Option<u64>,
}

impl RequestEntry {
    pub fn new(
        url: impl Into<String>,
        status: Option<u16>,
        resource_type: ResourceType,
        duration_ms: u64,
    ) -> Self {
        Self {
            url: url.into(),
            status,
            resource_type,
            duration_ms,
            queue_ms: None,
            headers_ms: None,
        }
    }

    /// Adds measured scheduler-queue and transport-header timings.
    pub fn with_network_timing(mut self, queue_ms: Option<u64>, headers_ms: Option<u64>) -> Self {
        self.queue_ms = queue_ms;
        self.headers_ms = headers_ms;
        self
    }
}

/// Session-local request history for the Network panel.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NetworkLog {
    entries: Vec<RequestEntry>,
}

impl NetworkLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, entry: RequestEntry) {
        self.entries.push(entry);
    }

    pub fn entries(&self) -> &[RequestEntry] {
        &self.entries
    }

    pub fn entries_of_type(&self, resource_type: ResourceType) -> Vec<&RequestEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.resource_type == resource_type)
            .collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{NetworkLog, RequestEntry, ResourceType};

    #[test]
    fn records_requests_in_completion_order_and_filters_by_type() {
        let mut log = NetworkLog::new();
        log.record(RequestEntry::new(
            "https://example.test/app.js",
            Some(200),
            ResourceType::Script,
            12,
        ));
        log.record(RequestEntry::new(
            "https://example.test/style.css",
            Some(200),
            ResourceType::Stylesheet,
            7,
        ));
        log.record(RequestEntry::new(
            "https://example.test/app.js",
            None,
            ResourceType::Script,
            40,
        ));

        assert_eq!(log.entries().len(), 3);
        assert_eq!(log.entries_of_type(ResourceType::Script).len(), 2);
        assert_eq!(log.entries_of_type(ResourceType::Script)[0].duration_ms, 12);
    }

    #[test]
    fn clear_removes_all_requests() {
        let mut log = NetworkLog::new();
        log.record(RequestEntry::new(
            "byui://welcome",
            Some(200),
            ResourceType::Document,
            1,
        ));
        log.clear();
        assert!(log.entries().is_empty());
    }
}

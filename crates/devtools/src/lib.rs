//! DevTools protocol and tooling: inspector, debugger, profiler, network panel, console.
//!
//! **Owning team**: Devtools Team
//!
//! Runs in the DevTools process and coordinates with other teams' hooks.

#![forbid(unsafe_code)]

/// Severity of a console entry, mirroring `console.log` / `warn` / `error`.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Log,
    Warn,
    Error,
}

/// One line in the console panel.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: Level,
    pub message: String,
}

/// Console panel backing store. Other teams push entries; the panel reads them.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Console {
    entries: Vec<LogEntry>,
}

impl Console {
    /// An empty console.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one entry.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn log(&mut self, level: Level, message: &str) {
        self.entries.push(LogEntry {
            level,
            message: message.to_owned(),
        });
    }

    /// All entries in the order they were logged.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Entries at exactly `level`, in order.
    ///
    /// Currently only handles the empty console.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn entries_at(&self, level: Level) -> Vec<&LogEntry> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        todo!(
            "TODO(devtools): filter {} entries by {level:?}",
            self.entries.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_console_is_empty() {
        let console = Console::new();
        assert!(console.entries().is_empty());
        assert!(console.entries_at(Level::Error).is_empty());
    }

    #[test]
    fn log_preserves_order_and_level() {
        let mut console = Console::new();
        console.log(Level::Log, "first");
        console.log(Level::Error, "second");
        let entries = console.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, Level::Log);
        assert_eq!(entries[0].message, "first");
        assert_eq!(entries[1].level, Level::Error);
        assert_eq!(entries[1].message, "second");
    }
}

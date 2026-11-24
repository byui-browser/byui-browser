//! Browser-owned completion history, independent of each tab's Back/Forward list.
//!
//! Retains at most 1,000 distinct HTTP(S) addresses, newest last. Internal pages,
//! failed loads, and URLs containing credentials are not persisted. This is a
//! single-process snapshot; concurrent browser instances do not merge history.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_VISITS: usize = 1_000;
const MAX_URL_BYTES: usize = 8_192;
const MAX_FILE_BYTES: u64 = (MAX_VISITS * (MAX_URL_BYTES + 1)) as u64;

/// Owned completion addresses in recency order, optionally backed by a file.
#[derive(Default)]
pub(crate) struct AddressHistory {
    urls: Vec<String>,
    path: Option<PathBuf>,
}

impl AddressHistory {
    /// Loads a browser-owned file; missing files start empty. Read failures
    /// preserve in-memory history support and are reported through the logger.
    pub(crate) fn open(path: PathBuf) -> Self {
        let mut history = Self {
            path: Some(path.clone()),
            ..Self::default()
        };
        match read(&path) {
            Ok(text) => {
                for url in text.lines() {
                    history.remember(url);
                }
            }
            Err(error) => log::warn!("could not load address history: {error}"),
        }
        history
    }

    /// Borrows distinct normalized addresses, oldest first.
    pub(crate) fn urls(&self) -> &[String] {
        &self.urls
    }

    /// Records a completed visit, promoting duplicates to most recent. A failed
    /// disk write leaves the previous file intact and retains the visit in memory.
    pub(crate) fn record(&mut self, url: &str) {
        if !self.remember(url) {
            return;
        }
        if let Some(path) = &self.path
            && let Err(error) = self.save(path)
        {
            log::warn!("could not save address history: {error}");
        }
    }

    fn remember(&mut self, address: &str) -> bool {
        if address.len() > MAX_URL_BYTES || address.chars().any(char::is_control) {
            return false;
        }
        let Ok(url) = net::Url::parse(address) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        let url = url.to_string();
        if url.len() > MAX_URL_BYTES {
            return false;
        }
        self.urls.retain(|entry| entry != &url);
        self.urls.push(url);
        if self.urls.len() > MAX_VISITS {
            self.urls.remove(0);
        }
        true
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        let pending = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&pending)?;
        let result = (|| {
            for url in &self.urls {
                writeln!(file, "{url}")?;
            }
            file.sync_all()
        })();
        drop(file);
        let result = result.and_then(|()| fs::rename(&pending, path));
        if result.is_err() {
            let _ = fs::remove_file(pending);
        }
        result
    }
}

fn read(path: &Path) -> io::Result<String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error),
    };
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "address history exceeds size limit",
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visits_survive_reopening_and_duplicates_become_recent() {
        let path =
            std::env::temp_dir().join(format!("byui-address-history-{}.txt", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut history = AddressHistory::open(path.clone());
        history.record("https://byui.instructure.com");
        history.record("https://www.byui.edu/");
        history.record("https://byui.instructure.com/");
        let reopened = AddressHistory::open(path.clone());
        assert_eq!(
            reopened.urls(),
            ["https://www.byui.edu/", "https://byui.instructure.com/"]
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_invalid_internal_and_credential_urls_and_bounds_history() {
        let mut history = AddressHistory::default();
        for url in [
            "invalid",
            "byui://welcome",
            "https://user:secret@example.com/",
            "https://example.com/\nother",
        ] {
            history.record(url);
        }
        assert!(history.urls().is_empty());
        for i in 0..=MAX_VISITS {
            history.record(&format!("https://example.com/{i}"));
        }
        assert_eq!(history.urls().len(), MAX_VISITS);
        assert_eq!(history.urls()[0], "https://example.com/1");
    }

    #[test]
    fn failed_write_keeps_visit_available_in_memory() {
        let mut history = AddressHistory {
            path: Some(
                std::env::temp_dir()
                    .join(format!("byui-missing-history-dir-{}", std::process::id()))
                    .join("history"),
            ),
            ..AddressHistory::default()
        };
        history.record("https://example.com/");
        assert_eq!(history.urls(), ["https://example.com/"]);
    }
}

//! Origin-scoped browser storage.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use security::{StorageKey, TrustedStorageKey};

const FILE_MAGIC: &[u8] = b"BYUI-LS\0";
const FILE_VERSION: u8 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: u32 = 100_000;

/// Storage limits measured in UTF-8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageLimits {
    max_item_bytes: usize,
    max_origin_bytes: usize,
}

impl StorageLimits {
    /// Creates limits for one key/value pair and one origin area.
    #[must_use]
    pub const fn new(max_item_bytes: usize, max_origin_bytes: usize) -> Self {
        Self {
            max_item_bytes,
            max_origin_bytes,
        }
    }

    /// Returns the maximum combined key/value size.
    #[must_use]
    pub const fn max_item_bytes(self) -> usize {
        self.max_item_bytes
    }

    /// Returns the maximum total size for one origin.
    #[must_use]
    pub const fn max_origin_bytes(self) -> usize {
        self.max_origin_bytes
    }
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self::new(1024 * 1024, 5 * 1024 * 1024)
    }
}

/// Failure while reading or writing browser storage.
#[derive(Debug)]
pub enum StorageError {
    /// The operating system rejected a file operation.
    Io(io::Error),
    /// A storage record failed validation.
    CorruptRecord(String),
    /// A write exceeded one of the configured limits.
    QuotaExceeded {
        /// Maximum permitted size in bytes.
        limit_bytes: usize,
        /// Requested size in bytes.
        requested_bytes: usize,
    },
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("storage I/O failed"),
            Self::CorruptRecord(reason) => write!(formatter, "corrupt storage record: {reason}"),
            Self::QuotaExceeded {
                limit_bytes,
                requested_bytes,
            } => write!(
                formatter,
                "storage quota exceeded: {requested_bytes} of {limit_bytes} bytes"
            ),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::CorruptRecord(_) | Self::QuotaExceeded { .. } => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// In-memory `localStorage` for one origin.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalStorage {
    items: HashMap<String, String>,
}

impl LocalStorage {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value for `key`.
    #[must_use]
    pub fn get_item(&self, key: &str) -> Option<&str> {
        self.items.get(key).map(String::as_str)
    }

    /// Stores `value` under `key`.
    pub fn set_item(&mut self, key: &str, value: &str) {
        self.items.insert(key.to_owned(), value.to_owned());
    }

    /// Removes and returns the value for `key`.
    pub fn remove_item(&mut self, key: &str) -> Option<String> {
        self.items.remove(key)
    }

    /// Returns the number of stored keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn size_bytes(&self) -> usize {
        self.items.iter().fold(0, |total, (key, value)| {
            total.saturating_add(key.len()).saturating_add(value.len())
        })
    }
}

/// In-memory storage areas partitioned by [`StorageKey`].
///
/// Operations require a [`TrustedStorageKey`] minted by a
/// [`security::RendererSecurityContext`]. This prevents callers at the
/// renderer boundary from selecting an arbitrary origin.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OriginStorage {
    areas: HashMap<StorageKey, LocalStorage>,
}

impl OriginStorage {
    /// Creates an empty origin-partitioned store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value for `key` in the authorized origin area.
    #[must_use]
    pub fn get_item(&self, storage_key: &TrustedStorageKey<'_>, key: &str) -> Option<&str> {
        self.get_item_for_key(storage_key.storage_key(), key)
    }

    /// Stores `value` under `key` in the authorized origin area.
    pub fn set_item(&mut self, storage_key: &TrustedStorageKey<'_>, key: &str, value: &str) {
        self.set_item_for_key(storage_key.storage_key(), key, value);
    }

    fn get_item_for_key(&self, storage_key: &StorageKey, key: &str) -> Option<&str> {
        self.areas
            .get(storage_key)
            .and_then(|area| area.get_item(key))
    }

    fn set_item_for_key(&mut self, storage_key: &StorageKey, key: &str, value: &str) {
        if !self.areas.contains_key(storage_key) {
            log::debug!("storage area opened for {}", storage_label(storage_key));
        }
        log::trace!(
            "set_item key {key:?} ({} bytes) in {}",
            value.len(),
            storage_label(storage_key)
        );
        self.areas
            .entry(storage_key.clone())
            .or_default()
            .set_item(key, value);
    }

    /// Removes and returns a value from the authorized origin area.
    pub fn remove_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
    ) -> Option<String> {
        self.remove_item_for_key(storage_key.storage_key(), key)
    }

    fn remove_item_for_key(&mut self, storage_key: &StorageKey, key: &str) -> Option<String> {
        let value = self.areas.get_mut(storage_key)?.remove_item(key);
        log::trace!(
            "remove_item key {key:?} in {} (present: {})",
            storage_label(storage_key),
            value.is_some()
        );
        if self
            .areas
            .get(storage_key)
            .is_some_and(LocalStorage::is_empty)
        {
            self.areas.remove(storage_key);
        }
        value
    }

    /// Returns the number of keys in the authorized origin area.
    #[must_use]
    pub fn len(&self, storage_key: &TrustedStorageKey<'_>) -> usize {
        self.len_for_key(storage_key.storage_key())
    }

    /// Removes every key in the authorized origin area.
    pub fn clear(&mut self, storage_key: &TrustedStorageKey<'_>) {
        self.clear_for_key(storage_key.storage_key());
    }

    fn len_for_key(&self, storage_key: &StorageKey) -> usize {
        self.areas.get(storage_key).map_or(0, LocalStorage::len)
    }

    fn clear_for_key(&mut self, storage_key: &StorageKey) {
        log::debug!("clearing storage area for {}", storage_label(storage_key));
        self.areas.remove(storage_key);
    }

    /// Returns whether the authorized origin area is empty.
    #[must_use]
    pub fn is_empty(&self, storage_key: &TrustedStorageKey<'_>) -> bool {
        self.len_for_key(storage_key.storage_key()) == 0
    }
}

/// Storage operations used by JavaScript bindings and browser processes.
///
/// The backend accepts only a [`TrustedStorageKey`], so a renderer cannot
/// select a different origin by passing a raw [`StorageKey`].
pub trait StorageBackend: fmt::Debug + Send {
    /// Reads a value for the authorized origin and item key.
    fn read_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
    ) -> Result<Option<String>, StorageError>;

    /// Writes a value for the authorized origin and item key.
    fn write_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError>;
}

/// Shared storage handle for renderer bindings.
pub type SharedStorage = Arc<Mutex<Box<dyn StorageBackend>>>;

/// Wraps a storage backend for use by multiple bindings.
#[must_use]
pub fn shared_storage<B>(backend: B) -> SharedStorage
where
    B: StorageBackend + 'static,
{
    Arc::new(Mutex::new(Box::new(backend)))
}

impl StorageBackend for OriginStorage {
    fn read_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(self.get_item(storage_key, key).map(str::to_owned))
    }

    fn write_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        self.set_item(storage_key, key, value);
        Ok(())
    }
}

/// File-backed storage with one snapshot sequence per origin.
#[derive(Debug)]
pub struct PersistentOriginStorage {
    root: PathBuf,
    limits: StorageLimits,
    memory: OriginStorage,
    loaded: HashSet<StorageKey>,
    generations: HashMap<StorageKey, u64>,
}

impl PersistentOriginStorage {
    /// Opens storage under the browser-owned directory `root`.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::open_with_limits(root, StorageLimits::default())
    }

    /// Opens storage with explicit byte limits.
    pub fn open_with_limits(
        root: impl AsRef<Path>,
        limits: StorageLimits,
    ) -> Result<Self, StorageError> {
        let root = root.as_ref().to_path_buf();
        if let Err(error) = fs::create_dir_all(&root) {
            log::error!("could not open storage at {}: {error}", root.display());
            return Err(error.into());
        }
        log::info!("storage opened at {}", root.display());
        log::debug!(
            "storage limits: {} bytes per item, {} bytes per origin",
            limits.max_item_bytes,
            limits.max_origin_bytes
        );

        Ok(Self {
            root,
            limits,
            memory: OriginStorage::new(),
            loaded: HashSet::new(),
            generations: HashMap::new(),
        })
    }

    /// Reads a value, loading the authorized origin's latest valid snapshot first.
    pub fn read_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        let storage_key = storage_key.storage_key();
        self.ensure_loaded(storage_key)?;
        let value = self
            .memory
            .get_item_for_key(storage_key, key)
            .map(str::to_owned);
        log::debug!(
            "read key {key:?} from {} (found: {}, {} bytes)",
            storage_label(storage_key),
            value.is_some(),
            value.as_ref().map_or(0, String::len)
        );
        Ok(value)
    }

    /// Writes a value and publishes a complete new snapshot for the authorized origin.
    pub fn write_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        let storage_key = storage_key.storage_key();
        self.ensure_loaded(storage_key)?;
        log::debug!(
            "write key {key:?} ({} bytes) to {}",
            value.len(),
            storage_label(storage_key)
        );

        let item_size = key.len().saturating_add(value.len());
        if item_size > self.limits.max_item_bytes {
            log::warn!(
                "storage quota exceeded for {}: item of {item_size} bytes exceeds {}",
                storage_label(storage_key),
                self.limits.max_item_bytes
            );
            return Err(StorageError::QuotaExceeded {
                limit_bytes: self.limits.max_item_bytes,
                requested_bytes: item_size,
            });
        }

        let previous = self.memory.areas.get(storage_key).cloned();
        self.memory.set_item_for_key(storage_key, key, value);
        let area = self
            .memory
            .areas
            .get(storage_key)
            .expect("set_item creates the origin area");

        if area.size_bytes() > self.limits.max_origin_bytes {
            let requested_bytes = area.size_bytes();
            log::warn!(
                "storage quota exceeded for {}: origin would use {requested_bytes} of {} bytes",
                storage_label(storage_key),
                self.limits.max_origin_bytes
            );
            self.restore_area(storage_key, previous);
            return Err(StorageError::QuotaExceeded {
                limit_bytes: self.limits.max_origin_bytes,
                requested_bytes,
            });
        }

        let record = match encode_area(storage_key, area, self.limits) {
            Ok(record) => record,
            Err(error) => {
                log::warn!(
                    "could not encode storage for {}: {error}",
                    storage_label(storage_key)
                );
                self.restore_area(storage_key, previous);
                return Err(error);
            }
        };

        let next_generation = self
            .generations
            .get(storage_key)
            .copied()
            .unwrap_or_default()
            .checked_add(1)
            .ok_or_else(|| StorageError::CorruptRecord("snapshot generation overflow".into()))?;
        let generation = match self.write_snapshot(storage_key, next_generation, &record) {
            Ok(generation) => generation,
            Err(error) => {
                match std::error::Error::source(&error) {
                    Some(source) => log::error!(
                        "could not write storage snapshot for {}: {error} (cause: {source})",
                        storage_label(storage_key)
                    ),
                    None => log::error!(
                        "could not write storage snapshot for {}: {error}",
                        storage_label(storage_key)
                    ),
                }
                self.restore_area(storage_key, previous);
                return Err(error);
            }
        };

        log::debug!(
            "storage snapshot generation {generation} written for {}",
            storage_label(storage_key)
        );
        self.generations.insert(storage_key.clone(), generation);
        self.remove_old_snapshots(storage_key, generation);
        Ok(())
    }

    fn ensure_loaded(&mut self, storage_key: &StorageKey) -> Result<(), StorageError> {
        if self.loaded.contains(storage_key) {
            return Ok(());
        }

        log::debug!("loading storage area for {}", storage_label(storage_key));
        let snapshots = self.snapshots(storage_key).inspect_err(|error| {
            log::error!(
                "could not list storage snapshots for {}: {error}",
                storage_label(storage_key)
            );
        })?;
        let newest_generation = snapshots.first().map_or(0, |(generation, _)| *generation);
        let mut saw_corrupt = false;

        for (generation, path) in snapshots {
            match read_snapshot(&path, storage_key, self.limits) {
                Ok(area) => {
                    log::debug!(
                        "loaded snapshot generation {generation} for {} ({} keys)",
                        storage_label(storage_key),
                        area.len()
                    );
                    self.memory.areas.insert(storage_key.clone(), area);
                    self.generations
                        .insert(storage_key.clone(), newest_generation);
                    self.loaded.insert(storage_key.clone());
                    return Ok(());
                }
                Err(StorageError::CorruptRecord(reason)) => {
                    log::warn!(
                        "skipping corrupt snapshot {} for {}: {reason}",
                        path.display(),
                        storage_label(storage_key)
                    );
                    saw_corrupt = true;
                }
                Err(error) => {
                    log::error!(
                        "could not read snapshot {} for {}: {error}",
                        path.display(),
                        storage_label(storage_key)
                    );
                    return Err(error);
                }
            }
            self.generations.insert(storage_key.clone(), generation);
        }

        if saw_corrupt {
            log::error!(
                "storage for {} is corrupt: no valid snapshot was found",
                storage_label(storage_key)
            );
            return Err(StorageError::CorruptRecord(
                "no valid snapshot was found".into(),
            ));
        }

        log::debug!(
            "no snapshots found for {}; starting empty",
            storage_label(storage_key)
        );
        self.loaded.insert(storage_key.clone());
        self.generations.insert(storage_key.clone(), 0);
        Ok(())
    }

    fn snapshots(&self, storage_key: &StorageKey) -> Result<Vec<(u64, PathBuf)>, StorageError> {
        let prefix = origin_file_prefix(storage_key);
        let mut snapshots = Vec::new();

        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() {
                continue;
            }

            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(generation) = name
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".storage"))
                .and_then(|generation| generation.parse::<u64>().ok())
            else {
                continue;
            };
            snapshots.push((generation, path));
        }

        snapshots.sort_unstable_by_key(|snapshot| std::cmp::Reverse(snapshot.0));
        Ok(snapshots)
    }

    fn write_snapshot(
        &self,
        storage_key: &StorageKey,
        mut generation: u64,
        record: &[u8],
    ) -> Result<u64, StorageError> {
        let prefix = origin_file_prefix(storage_key);

        loop {
            let path = self.root.join(format!("{prefix}{generation}.storage"));
            let file = OpenOptions::new().write(true).create_new(true).open(&path);

            let mut file = match file {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    generation = generation.checked_add(1).ok_or_else(|| {
                        StorageError::CorruptRecord("snapshot generation overflow".into())
                    })?;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            let result = file.write_all(record).and_then(|()| file.sync_all());
            if result.is_err()
                && let Err(error) = fs::remove_file(&path)
            {
                log::warn!(
                    "could not remove partial snapshot {}: {error}",
                    path.display()
                );
            }
            result?;
            return Ok(generation);
        }
    }

    fn remove_old_snapshots(&self, storage_key: &StorageKey, newest: u64) {
        match self.snapshots(storage_key) {
            Ok(snapshots) => {
                for (generation, path) in snapshots {
                    if generation.saturating_add(1) < newest
                        && let Err(error) = fs::remove_file(&path)
                    {
                        log::warn!(
                            "could not remove stale snapshot {}: {error}",
                            path.display()
                        );
                    }
                }
            }
            Err(error) => log::debug!(
                "could not list snapshots for {}; stale snapshots kept: {error}",
                storage_label(storage_key)
            ),
        }
    }

    fn restore_area(&mut self, storage_key: &StorageKey, previous: Option<LocalStorage>) {
        match previous {
            Some(area) => {
                self.memory.areas.insert(storage_key.clone(), area);
            }
            None => {
                self.memory.areas.remove(storage_key);
            }
        }
    }
}

impl StorageBackend for PersistentOriginStorage {
    fn read_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        Self::read_item(self, storage_key, key)
    }

    fn write_item(
        &mut self,
        storage_key: &TrustedStorageKey<'_>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        Self::write_item(self, storage_key, key, value)
    }
}

/// Formats a storage key's origin as `scheme://host:port` for log messages.
fn storage_label(storage_key: &StorageKey) -> String {
    let origin = storage_key.get();
    format!("{}://{}:{}", origin.scheme(), origin.host(), origin.port())
}

fn origin_file_prefix(storage_key: &StorageKey) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        storage_key.get().scheme(),
        storage_key.get().host(),
        storage_key.get().port()
    );
    format!("{}-", hex_encode(identity.as_bytes()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

fn encode_area(
    storage_key: &StorageKey,
    area: &LocalStorage,
    limits: StorageLimits,
) -> Result<Vec<u8>, StorageError> {
    let origin_id = origin_file_prefix(storage_key).into_bytes();
    let mut entries: Vec<_> = area.items.iter().collect();
    entries.sort_unstable_by_key(|(key, _)| (*key).clone());

    let entry_count = u32::try_from(entries.len())
        .map_err(|_| StorageError::CorruptRecord("too many storage entries".into()))?;
    let mut record = Vec::new();
    record.extend_from_slice(FILE_MAGIC);
    record.push(FILE_VERSION);
    write_u32(&mut record, origin_id.len())?;
    record.extend_from_slice(&origin_id);
    write_u32(&mut record, entry_count as usize)?;

    for (key, value) in entries {
        let item_size = key.len().saturating_add(value.len());
        if item_size > limits.max_item_bytes {
            return Err(StorageError::QuotaExceeded {
                limit_bytes: limits.max_item_bytes,
                requested_bytes: item_size,
            });
        }
        write_string(&mut record, key)?;
        write_string(&mut record, value)?;
    }

    Ok(record)
}

fn read_snapshot(
    path: &Path,
    storage_key: &StorageKey,
    limits: StorageLimits,
) -> Result<LocalStorage, StorageError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(StorageError::CorruptRecord("invalid snapshot file".into()));
    }
    let bytes = fs::read(path)?;
    decode_area(&bytes, storage_key, limits)
}

fn decode_area(
    bytes: &[u8],
    storage_key: &StorageKey,
    limits: StorageLimits,
) -> Result<LocalStorage, StorageError> {
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(FILE_MAGIC.len())? != FILE_MAGIC {
        return Err(StorageError::CorruptRecord("invalid file header".into()));
    }
    if reader.byte()? != FILE_VERSION {
        return Err(StorageError::CorruptRecord(
            "unsupported file version".into(),
        ));
    }

    let origin_id = reader.bytes()?;
    if origin_id != origin_file_prefix(storage_key).as_bytes() {
        return Err(StorageError::CorruptRecord("origin key mismatch".into()));
    }

    let entry_count = reader.u32()?;
    if entry_count > MAX_ENTRIES {
        return Err(StorageError::CorruptRecord(
            "too many storage entries".into(),
        ));
    }

    let mut area = LocalStorage::new();
    for _ in 0..entry_count {
        let key = reader.string()?;
        let value = reader.string()?;
        let item_size = key.len().saturating_add(value.len());
        if item_size > limits.max_item_bytes {
            return Err(StorageError::CorruptRecord("item exceeds quota".into()));
        }
        if area.items.contains_key(&key) {
            return Err(StorageError::CorruptRecord("duplicate storage key".into()));
        }
        area.set_item(&key, &value);
    }

    if area.size_bytes() > limits.max_origin_bytes {
        return Err(StorageError::CorruptRecord("origin exceeds quota".into()));
    }
    if reader.remaining() != 0 {
        return Err(StorageError::CorruptRecord("trailing record data".into()));
    }
    Ok(area)
}

fn write_u32(buffer: &mut Vec<u8>, value: usize) -> Result<(), StorageError> {
    let value = u32::try_from(value)
        .map_err(|_| StorageError::CorruptRecord("storage record is too large".into()))?;
    buffer.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_string(buffer: &mut Vec<u8>, value: &str) -> Result<(), StorageError> {
    write_u32(buffer, value.len())?;
    buffer.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], StorageError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| StorageError::CorruptRecord("record length overflow".into()))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| StorageError::CorruptRecord("truncated storage record".into()))?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, StorageError> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u32(&mut self) -> Result<u32, StorageError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn bytes(&mut self) -> Result<&'a [u8], StorageError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| StorageError::CorruptRecord("record length overflow".into()))?;
        self.take(length)
    }

    fn string(&mut self) -> Result<String, StorageError> {
        String::from_utf8(self.bytes()?.to_vec())
            .map_err(|_| StorageError::CorruptRecord("invalid UTF-8".into()))
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use security::{Origin, RendererSecurityContext, StorageKey, StoragePermission};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn storage_key(scheme: &str, host: &str, port: u16) -> StorageKey {
        StorageKey::new(Origin::with_port(scheme, host, port))
    }

    fn context_for(storage_key: &StorageKey) -> RendererSecurityContext {
        RendererSecurityContext::new(storage_key.get().clone(), StoragePermission::Allowed)
    }

    fn test_directory(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("byui-storage-{name}-{suffix}"))
    }

    fn remove_test_directory(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    #[test]
    fn missing_key_is_none() {
        assert_eq!(LocalStorage::new().get_item("token"), None);
    }

    #[test]
    fn set_then_get_round_trips() {
        let mut store = LocalStorage::new();
        store.set_item("token", "abc");
        assert_eq!(store.get_item("token"), Some("abc"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn set_overwrites_existing_value() {
        let mut store = LocalStorage::new();
        store.set_item("k", "1");
        store.set_item("k", "2");
        assert_eq!(store.get_item("k"), Some("2"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_returns_old_value_and_clears_key() {
        let mut store = LocalStorage::new();
        store.set_item("k", "v");
        assert_eq!(store.remove_item("k"), Some("v".into()));
        assert!(store.is_empty());
        assert_eq!(store.remove_item("k"), None);
    }

    #[test]
    fn origin_areas_do_not_share_values() {
        let first = storage_key("https", "example.com", 443);
        let second = storage_key("https", "other.example", 443);
        let first_context = context_for(&first);
        let second_context = context_for(&second);
        let first = first_context.trusted_storage_key();
        let second = second_context.trusted_storage_key();
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "token", "first");
        storage.set_item(&second, "token", "second");

        assert_eq!(storage.get_item(&first, "token"), Some("first"));
        assert_eq!(storage.get_item(&second, "token"), Some("second"));
    }

    #[test]
    fn same_origin_keys_share_an_area() {
        let first = storage_key("https", "example.com", 443);
        let equivalent = storage_key("HTTPS", "EXAMPLE.COM", 443);
        let first_context = context_for(&first);
        let equivalent_context = context_for(&equivalent);
        let first = first_context.trusted_storage_key();
        let equivalent = equivalent_context.trusted_storage_key();
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "theme", "dark");

        assert_eq!(storage.get_item(&equivalent, "theme"), Some("dark"));
        assert_eq!(storage.len(&equivalent), 1);
    }

    #[test]
    fn clear_only_removes_the_selected_origin() {
        let first = storage_key("https", "example.com", 443);
        let second = storage_key("http", "example.com", 80);
        let first_context = context_for(&first);
        let second_context = context_for(&second);
        let first = first_context.trusted_storage_key();
        let second = second_context.trusted_storage_key();
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "name", "first");
        storage.set_item(&second, "name", "second");
        storage.clear(&first);

        assert!(storage.is_empty(&first));
        assert_eq!(storage.get_item(&second, "name"), Some("second"));
    }

    #[test]
    fn dropping_storage_discards_all_values() {
        let key = storage_key("https", "example.com", 443);
        let context = context_for(&key);
        let key = context.trusted_storage_key();
        {
            let mut storage = OriginStorage::new();
            storage.set_item(&key, "session", "temporary");
        }

        let storage = OriginStorage::new();
        assert!(storage.is_empty(&key));
    }

    #[test]
    fn persistent_values_survive_reopen() {
        let path = test_directory("reopen");
        let key = storage_key("https", "example.com", 443);
        let context = context_for(&key);
        let key = context.trusted_storage_key();

        {
            let mut storage = PersistentOriginStorage::open(&path).unwrap();
            storage.write_item(&key, "theme", "dark").unwrap();
        }

        let mut reopened = PersistentOriginStorage::open(&path).unwrap();
        assert_eq!(
            reopened.read_item(&key, "theme").unwrap(),
            Some("dark".into())
        );
        remove_test_directory(&path);
    }

    #[test]
    fn persistent_storage_keeps_origins_separate() {
        let path = test_directory("isolation");
        let first = storage_key("https", "example.com", 443);
        let second = storage_key("https", "other.example", 443);
        let first_context = context_for(&first);
        let second_context = context_for(&second);
        let first = first_context.trusted_storage_key();
        let second = second_context.trusted_storage_key();
        let mut storage = PersistentOriginStorage::open(&path).unwrap();

        storage.write_item(&first, "token", "first").unwrap();
        storage.write_item(&second, "token", "second").unwrap();
        drop(storage);

        let mut reopened = PersistentOriginStorage::open(&path).unwrap();
        assert_eq!(
            reopened.read_item(&first, "token").unwrap(),
            Some("first".into())
        );
        assert_eq!(
            reopened.read_item(&second, "token").unwrap(),
            Some("second".into())
        );
        remove_test_directory(&path);
    }

    #[test]
    fn persistent_files_do_not_use_raw_origin_text() {
        let path = test_directory("filenames");
        let key = storage_key("https", "example.com/path", 443);
        let context = context_for(&key);
        let key = context.trusted_storage_key();
        let mut storage = PersistentOriginStorage::open(&path).unwrap();
        storage.write_item(&key, "name", "value").unwrap();

        for entry in fs::read_dir(&path).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            let stem = name.strip_suffix(".storage").unwrap();
            let (origin_id, generation) = stem.rsplit_once('-').unwrap();
            assert!(
                origin_id
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
            );
            assert!(
                generation
                    .chars()
                    .all(|character| character.is_ascii_digit())
            );
            assert!(!name.contains("example.com/path"));
        }
        remove_test_directory(&path);
    }

    #[test]
    fn corrupt_snapshot_is_rejected() {
        let path = test_directory("corrupt");
        let key = storage_key("https", "example.com", 443);
        let context = context_for(&key);
        let key = context.trusted_storage_key();
        fs::create_dir_all(&path).unwrap();
        let file_name = format!("{}1.storage", origin_file_prefix(key.storage_key()));
        fs::write(path.join(file_name), b"not a storage record").unwrap();

        let mut storage = PersistentOriginStorage::open(&path).unwrap();
        assert!(matches!(
            storage.read_item(&key, "name"),
            Err(StorageError::CorruptRecord(_))
        ));
        remove_test_directory(&path);
    }

    #[test]
    fn quota_rejects_large_values() {
        let path = test_directory("quota");
        let key = storage_key("https", "example.com", 443);
        let context = context_for(&key);
        let key = context.trusted_storage_key();
        let limits = StorageLimits::new(4, 8);
        let mut storage = PersistentOriginStorage::open_with_limits(&path, limits).unwrap();

        storage.write_item(&key, "a", "123").unwrap();
        assert!(matches!(
            storage.write_item(&key, "long", "value"),
            Err(StorageError::QuotaExceeded { .. })
        ));
        assert_eq!(storage.read_item(&key, "a").unwrap(), Some("123".into()));
        remove_test_directory(&path);
    }
}

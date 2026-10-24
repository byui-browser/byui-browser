use std::{
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use net::{AbortSignal, RequestError};
use security::file_access::{FileAccessLease, OpenedFile};
use tokio::sync::mpsc;

use super::{
    policy::{provider_error, provider_io_error, provider_resource_exhausted_error},
    session::{FileAccessCancellation, FileSessionByteBudget},
};

pub(super) const FILE_READ_CHUNK_SIZE: usize = 64 * 1024;
pub(super) const FILE_READ_CHANNEL_CAPACITY: usize = 2;
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Shared byte reservation for chunks waiting in provider body channels.
#[derive(Clone, Debug)]
pub(super) struct FileBufferBudget {
    available: Arc<AtomicU64>,
    maximum: u64,
}

impl FileBufferBudget {
    pub(super) fn new(maximum: u64) -> Self {
        Self {
            available: Arc::new(AtomicU64::new(maximum)),
            maximum,
        }
    }

    pub(super) fn try_reserve(&self, bytes: u64) -> bool {
        let mut available = self.available.load(Ordering::Acquire);
        loop {
            let Some(next) = available.checked_sub(bytes) else {
                return false;
            };
            match self.available.compare_exchange_weak(
                available,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(current) => available = current,
            }
        }
    }

    fn release(&self, bytes: u64) {
        let previous = self.available.fetch_add(bytes, Ordering::AcqRel);
        debug_assert!(previous <= self.maximum && bytes <= self.maximum - previous);
    }

    pub(super) fn chunk_size(&self) -> usize {
        usize::try_from(self.maximum.min(FILE_READ_CHUNK_SIZE as u64))
            .expect("file buffer limit fits in usize")
    }

    #[cfg(test)]
    pub(super) fn available(&self) -> u64 {
        self.available.load(Ordering::Acquire)
    }
}

/// A queued chunk which returns its aggregate buffer reservation when dropped.
pub(super) struct BufferedChunk {
    bytes: Vec<u8>,
    reserved: u64,
    budget: FileBufferBudget,
}

impl BufferedChunk {
    pub(super) fn new(bytes: Vec<u8>, reserved: u64, budget: FileBufferBudget) -> Self {
        Self {
            bytes,
            reserved,
            budget,
        }
    }

    pub(super) fn into_bytes(mut self) -> Vec<u8> {
        self.release();
        mem::take(&mut self.bytes)
    }

    fn release(&mut self) {
        if self.reserved != 0 {
            self.budget.release(self.reserved);
            self.reserved = 0;
        }
    }
}

impl Drop for BufferedChunk {
    fn drop(&mut self) {
        self.release();
    }
}

/// State owned by the blocking task that reads an opened local file.
pub(super) struct FileReadTask {
    pub(super) opened: OpenedFile,
    pub(super) sender: mpsc::Sender<Result<BufferedChunk, RequestError>>,
    pub(super) _permit: tokio::sync::OwnedSemaphorePermit,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) session_cancellation: FileAccessCancellation,
    pub(super) lease: FileAccessLease,
    pub(super) signal: AbortSignal,
    pub(super) expected_length: u64,
    pub(super) deadline: Instant,
    pub(super) session_budget: FileSessionByteBudget,
    pub(super) buffer_budget: FileBufferBudget,
    #[cfg(test)]
    pub(super) before_read_hook: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Reads an opened file on Tokio's blocking pool and sends bounded body chunks.
pub(super) fn read_file_stream(mut task: FileReadTask) {
    let mut buffer = [0; FILE_READ_CHUNK_SIZE];
    let mut remaining = task.expected_length;
    loop {
        if task.cancelled.load(Ordering::Acquire)
            || task.session_cancellation.is_cancelled()
            || !task.lease.is_current()
            || task.signal.is_aborted()
        {
            return;
        }
        if Instant::now() >= task.deadline {
            let _ = send_chunk(
                &task.sender,
                Err(provider_resource_exhausted_error()),
                &task.cancelled,
                &task.session_cancellation,
                &task.lease,
                &task.signal,
                task.deadline,
            );
            return;
        }
        if remaining == 0 {
            match task
                .opened
                .read_chunk(&mut buffer[..1])
                .map_err(provider_error)
            {
                Ok(0) => return,
                Ok(_) => {
                    let _ = send_chunk(
                        &task.sender,
                        Err(provider_io_error()),
                        &task.cancelled,
                        &task.session_cancellation,
                        &task.lease,
                        &task.signal,
                        task.deadline,
                    );
                    return;
                }
                Err(error) => {
                    let _ = send_chunk(
                        &task.sender,
                        Err(error),
                        &task.cancelled,
                        &task.session_cancellation,
                        &task.lease,
                        &task.signal,
                        task.deadline,
                    );
                    return;
                }
            }
        }
        let permitted = usize::try_from(remaining.min(task.buffer_budget.chunk_size() as u64))
            .expect("file read chunk size fits in usize");
        if !reserve_buffer(&task, permitted as u64) {
            return;
        }
        #[cfg(test)]
        if let Some(hook) = &task.before_read_hook {
            hook();
        }
        match task
            .opened
            .read_chunk(&mut buffer[..permitted])
            .map_err(provider_error)
        {
            Ok(0) => {
                task.buffer_budget.release(permitted as u64);
                let _ = send_chunk(
                    &task.sender,
                    Err(provider_io_error()),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                    task.deadline,
                );
                return;
            }
            Ok(count) => {
                task.buffer_budget.release((permitted - count) as u64);
                remaining -= count as u64;
                if !task.session_budget.try_consume(count as u64) {
                    task.buffer_budget.release(count as u64);
                    let _ = send_chunk(
                        &task.sender,
                        Err(provider_resource_exhausted_error()),
                        &task.cancelled,
                        &task.session_cancellation,
                        &task.lease,
                        &task.signal,
                        task.deadline,
                    );
                    return;
                }
                if !send_chunk(
                    &task.sender,
                    Ok(BufferedChunk::new(
                        buffer[..count].to_vec(),
                        count as u64,
                        task.buffer_budget.clone(),
                    )),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                    task.deadline,
                ) {
                    return;
                }
            }
            Err(error) => {
                task.buffer_budget.release(permitted as u64);
                let _ = send_chunk(
                    &task.sender,
                    Err(error),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                    task.deadline,
                );
                return;
            }
        }
    }
}

/// Sends one chunk while honoring cancellation and bounded-channel backpressure.
fn send_chunk(
    sender: &mpsc::Sender<Result<BufferedChunk, RequestError>>,
    mut chunk: Result<BufferedChunk, RequestError>,
    cancelled: &AtomicBool,
    session_cancellation: &FileAccessCancellation,
    lease: &FileAccessLease,
    signal: &AbortSignal,
    deadline: Instant,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire)
            || session_cancellation.is_cancelled()
            || !lease.is_current()
            || signal.is_aborted()
        {
            return false;
        }
        if Instant::now() >= deadline {
            return false;
        }
        match sender.try_send(chunk) {
            Ok(()) => return true,
            Err(mpsc::error::TrySendError::Closed(_)) => return false,
            Err(mpsc::error::TrySendError::Full(returned)) => {
                chunk = returned;
                std::thread::sleep(BACKPRESSURE_POLL_INTERVAL);
            }
        }
    }
}

/// Waits for enough shared channel capacity without reading another byte.
fn reserve_buffer(task: &FileReadTask, bytes: u64) -> bool {
    loop {
        if task.cancelled.load(Ordering::Acquire)
            || task.session_cancellation.is_cancelled()
            || !task.lease.is_current()
            || task.signal.is_aborted()
            || Instant::now() >= task.deadline
        {
            return false;
        }
        if task.buffer_budget.try_reserve(bytes) {
            return true;
        }
        std::thread::sleep(BACKPRESSURE_POLL_INTERVAL);
    }
}

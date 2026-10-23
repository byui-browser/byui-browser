use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use net::{AbortSignal, RequestError};
use security::file_access::{FileAccessLease, OpenedFile};
use tokio::sync::mpsc;

use super::{
    policy::{provider_error, provider_io_error},
    session::FileAccessCancellation,
};

pub(super) const FILE_READ_CHUNK_SIZE: usize = 64 * 1024;
pub(super) const FILE_READ_CHANNEL_CAPACITY: usize = 2;
const BACKPRESSURE_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// State owned by the blocking task that reads an opened local file.
pub(super) struct FileReadTask {
    pub(super) opened: OpenedFile,
    pub(super) sender: mpsc::Sender<Result<Vec<u8>, RequestError>>,
    pub(super) _permit: tokio::sync::OwnedSemaphorePermit,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) session_cancellation: FileAccessCancellation,
    pub(super) lease: FileAccessLease,
    pub(super) signal: AbortSignal,
    pub(super) expected_length: u64,
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
                    );
                    return;
                }
            }
        }
        let permitted = usize::try_from(remaining.min(FILE_READ_CHUNK_SIZE as u64))
            .expect("file read chunk size fits in usize");
        match task
            .opened
            .read_chunk(&mut buffer[..permitted])
            .map_err(provider_error)
        {
            Ok(0) => {
                let _ = send_chunk(
                    &task.sender,
                    Err(provider_io_error()),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                );
                return;
            }
            Ok(count) => {
                remaining -= count as u64;
                if !send_chunk(
                    &task.sender,
                    Ok(buffer[..count].to_vec()),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                ) {
                    return;
                }
            }
            Err(error) => {
                let _ = send_chunk(
                    &task.sender,
                    Err(error),
                    &task.cancelled,
                    &task.session_cancellation,
                    &task.lease,
                    &task.signal,
                );
                return;
            }
        }
    }
}

/// Sends one chunk while honoring cancellation and bounded-channel backpressure.
fn send_chunk(
    sender: &mpsc::Sender<Result<Vec<u8>, RequestError>>,
    mut chunk: Result<Vec<u8>, RequestError>,
    cancelled: &AtomicBool,
    session_cancellation: &FileAccessCancellation,
    lease: &FileAccessLease,
    signal: &AbortSignal,
) -> bool {
    loop {
        if cancelled.load(Ordering::Acquire)
            || session_cancellation.is_cancelled()
            || !lease.is_current()
            || signal.is_aborted()
        {
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

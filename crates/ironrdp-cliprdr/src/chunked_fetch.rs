//! Chunked, receiver-driven fetch of one file's contents.
//!
//! [MS-RDPECLIP]'s file-contents protocol is receiver-driven: the side that
//! wants the data sends a [`FileContentsRequest`] naming a byte range
//! (`flags: RANGE`, `position`, `requested_size`), and the other side answers
//! with exactly that range in a [`FileContentsResponse`]. Fetching a whole
//! file therefore means issuing a sequence of these requests and
//! reassembling the responses; [`ChunkedFetch`] is that sequence as a small
//! state machine, so each [`crate::backend::CliprdrBackend`] implementation
//! does not have to write its own.
//!
//! # What this does not do
//!
//! `ChunkedFetch` only tracks one file's own progress (next offset, buffered
//! bytes so far). It does not send anything itself, does not allocate its
//! `stream_id`, and does not touch [`crate::Cliprdr`]'s lock or pending-request
//! state: [`crate::Cliprdr::request_file_contents`] already validates every
//! request against the negotiated capabilities and the remote file list, and
//! already renews the active lock's activity timestamp on every call that
//! carries a `data_id` (including an auto-filled one), so there is nothing
//! left for this type to do on that front. Driving one looks like:
//!
//! ```ignore
//! let request = fetch.next_request().expect("fetch not finished");
//! let messages = cliprdr.request_file_contents(request)?;
//! // ... send `messages`, then later, in `on_file_contents_response`:
//! match fetch.on_response(&response) {
//!     ChunkedFetchProgress::InProgress => { /* call next_request() again */ }
//!     ChunkedFetchProgress::Complete => { let data = fetch.into_data(); }
//!     ChunkedFetchProgress::Failed => { /* abandon the fetch */ }
//! }
//! ```
//!
//! Since more than one fetch can be outstanding at once, each needs its own
//! `stream_id`; [`ChunkedFetch::stream_id`] is the key a backend should use
//! to route an incoming response to the right instance.

use crate::pdu::{FileContentsFlags, FileContentsRequest, FileContentsResponse};

/// Result of feeding a [`FileContentsResponse`] to [`ChunkedFetch::on_response`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkedFetchProgress {
    /// More data remains; call [`ChunkedFetch::next_request`] again.
    InProgress,
    /// The fetch is complete. Call [`ChunkedFetch::into_data`] to take the assembled bytes.
    Complete,
    /// The fetch failed: either the response reported `CB_RESPONSE_FAIL`, a `SIZE` response
    /// was malformed, or the peer stopped sending data before the file was complete.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitingSize,
    Fetching,
    Complete,
    Failed,
}

/// Drives the request/response sequence to fetch one file's contents.
///
/// See the [module documentation](self) for what this does and does not handle.
#[derive(Debug)]
pub struct ChunkedFetch {
    stream_id: u32,
    file_index: i32,
    chunk_size: u32,
    total_size: Option<u64>,
    received: Vec<u8>,
    next_offset: u64,
    state: State,
    awaiting_response: bool,
}

impl ChunkedFetch {
    /// Start a fetch for a file whose size is already known.
    ///
    /// This is the common case: the size is usually already available from an earlier
    /// `FileGroupDescriptorW` exchange, so a `SIZE` round-trip is unnecessary overhead.
    /// `chunk_size` must be greater than 0.
    pub fn new(stream_id: u32, file_index: i32, total_size: u64, chunk_size: u32) -> Self {
        Self {
            stream_id,
            file_index,
            chunk_size: chunk_size.max(1),
            total_size: Some(total_size),
            received: Vec::new(),
            next_offset: 0,
            state: if total_size == 0 {
                State::Complete
            } else {
                State::Fetching
            },
            awaiting_response: false,
        }
    }

    /// Start a fetch that must first learn the file's size via a `SIZE` request.
    ///
    /// `chunk_size` must be greater than 0.
    pub fn new_with_size_query(stream_id: u32, file_index: i32, chunk_size: u32) -> Self {
        Self {
            stream_id,
            file_index,
            chunk_size: chunk_size.max(1),
            total_size: None,
            received: Vec::new(),
            next_offset: 0,
            state: State::AwaitingSize,
            awaiting_response: false,
        }
    }

    /// The stream ID every request/response in this fetch's sequence uses.
    ///
    /// A caller driving more than one `ChunkedFetch` concurrently should use this as the key
    /// to route an incoming [`crate::backend::CliprdrBackend::on_file_contents_response`] to
    /// the right instance.
    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    /// Whether the fetch finished, successfully or not.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Complete | State::Failed)
    }

    /// The next request to send via [`crate::Cliprdr::request_file_contents`].
    ///
    /// Returns `None` if the fetch has finished, or if a request from a previous call is
    /// still outstanding (call [`Self::on_response`] first).
    pub fn next_request(&mut self) -> Option<FileContentsRequest> {
        if self.awaiting_response {
            return None;
        }

        let request = match self.state {
            State::AwaitingSize => FileContentsRequest {
                stream_id: self.stream_id,
                index: self.file_index,
                flags: FileContentsFlags::SIZE,
                position: 0,
                requested_size: 8,
                data_id: None,
            },
            State::Fetching => {
                let total_size = self.total_size?;
                let remaining = total_size.saturating_sub(self.next_offset);
                let requested_size = remaining.min(u64::from(self.chunk_size));
                // Infallible: bounded above by self.chunk_size (a u32) via the min() just above.
                let requested_size = u32::try_from(requested_size).unwrap_or(self.chunk_size);

                FileContentsRequest {
                    stream_id: self.stream_id,
                    index: self.file_index,
                    flags: FileContentsFlags::RANGE,
                    position: self.next_offset,
                    requested_size,
                    data_id: None,
                }
            }
            State::Complete | State::Failed => return None,
        };

        self.awaiting_response = true;
        Some(request)
    }

    /// Feed the response to the request most recently returned by [`Self::next_request`].
    pub fn on_response(&mut self, response: &FileContentsResponse<'_>) -> ChunkedFetchProgress {
        self.awaiting_response = false;

        if response.is_error() {
            self.state = State::Failed;
            return ChunkedFetchProgress::Failed;
        }

        match self.state {
            State::AwaitingSize => match response.data_as_size() {
                Ok(total_size) => {
                    self.total_size = Some(total_size);
                    self.state = if total_size == 0 {
                        State::Complete
                    } else {
                        State::Fetching
                    };
                }
                Err(_) => self.state = State::Failed,
            },
            State::Fetching => {
                let data = response.data();

                // A compliant peer never sends an empty range response before the file is
                // complete (MS-RDPECLIP defines no "not ready yet" signal here). Treating
                // one as failure rather than InProgress avoids looping forever re-requesting
                // the same range against a peer that keeps answering with nothing.
                if data.is_empty() {
                    self.state = State::Failed;
                } else {
                    // Clamp rather than trust the peer's byte count: a response longer than
                    // what's left would otherwise grow the buffer past total_size and desync
                    // next_offset from what was actually requested.
                    let data_len = u64::try_from(data.len()).unwrap_or(u64::MAX);
                    let remaining = self
                        .total_size
                        .unwrap_or(0)
                        .saturating_sub(self.next_offset)
                        .min(data_len);
                    // Infallible: just clamped to `data.len()` via `data_len`/`min()` above.
                    let take = usize::try_from(remaining).unwrap_or(data.len());

                    self.received.extend_from_slice(&data[..take]);
                    self.next_offset = self.next_offset.saturating_add(remaining);

                    if self.next_offset >= self.total_size.unwrap_or(0) {
                        self.state = State::Complete;
                    }
                }
            }
            State::Complete | State::Failed => {}
        }

        match self.state {
            State::Complete => ChunkedFetchProgress::Complete,
            State::Failed => ChunkedFetchProgress::Failed,
            State::AwaitingSize | State::Fetching => ChunkedFetchProgress::InProgress,
        }
    }

    /// Take the bytes assembled so far.
    ///
    /// Meaningful once [`Self::on_response`] has returned [`ChunkedFetchProgress::Complete`];
    /// returns whatever has been accumulated so far if called earlier.
    pub fn into_data(self) -> Vec<u8> {
        self.received
    }
}

//! Serializes the CLIPRDR request/response stream and OS clipboard events.

use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::loop_detector::{ClipboardSource, LoopDetector};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_core::IntoOwned as _;
use tracing::{debug, warn};

use super::os::{OsClipboard, PNG, TEXT, Transfer};
use crate::data_control::find_mime_match;

pub const PASTE_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_TRANSFER_BYTES: usize = 100 * 1024 * 1024;
const MAX_WAITERS: usize = 16;

#[derive(Debug)]
pub enum Command {
    AdvertiseLocal,
    LocalChanged(Vec<String>),
    RemoteCopy(Vec<ClipboardFormat>),
    RemoteData(Option<Vec<u8>>),
    RenderLocal(ClipboardFormatId),
    Paste(Transfer),
    Reset,
    Shutdown,
}

struct Pending {
    format: ClipboardFormatId,
    generation: u64,
    deadline: Instant,
    expired: bool,
    waiters: Vec<Transfer>,
}

struct Queued {
    deadline: Instant,
    transfer: Transfer,
}

pub struct Worker<O, P> {
    os: O,
    proxy: P,
    ready: bool,
    local_mimes: Vec<String>,
    remote: Vec<ClipboardFormat>,
    owns_remote: bool,
    generation: u64,
    pending: Option<Pending>,
    queue: VecDeque<Queued>,
    cache: HashMap<ClipboardFormatId, Vec<u8>>,
    loops: LoopDetector,
}

impl<O: OsClipboard, P: ClipboardMessageProxy> Worker<O, P> {
    pub fn new(os: O, proxy: P) -> Self {
        let local_mimes = os.mime_types();
        Self {
            os,
            proxy,
            local_mimes,
            ready: false,
            remote: Vec::new(),
            owns_remote: false,
            generation: 0,
            pending: None,
            queue: VecDeque::new(),
            cache: HashMap::new(),
            loops: LoopDetector::new(),
        }
    }

    pub fn run(mut self, receiver: &Receiver<Command>) {
        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.expire(Instant::now());
        }
        let _ = self.os.clear();
    }

    pub fn handle(&mut self, command: Command) {
        match command {
            Command::AdvertiseLocal => {
                self.ready = true;
                if !self.owns_remote {
                    self.local_mimes = self.os.mime_types();
                    self.advertise_local();
                }
            }
            Command::LocalChanged(_) => {
                // An OS event queued before offer() acknowledged our ownership
                // must not invalidate the selection that was just installed.
                if self.os.is_owner() {
                    return;
                }
                self.invalidate();
                self.owns_remote = false;
                self.remote.clear();
                self.local_mimes = self.os.mime_types();
                if self.ready {
                    self.advertise_local();
                }
            }
            Command::RemoteCopy(formats) => {
                self.invalidate();
                self.remote = formats;
                self.owns_remote = true;
                let mut mimes = Vec::new();
                if self.remote.iter().any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT) {
                    mimes.extend([TEXT.to_owned(), "text/plain".to_owned()]);
                }
                if self
                    .remote
                    .iter()
                    .any(|f| matches!(f.id(), ClipboardFormatId::CF_DIB | ClipboardFormatId::CF_DIBV5))
                {
                    mimes.push(PNG.to_owned());
                }
                if let Err(error) = self.os.offer(&mimes, self.generation) {
                    warn!(%error, "Could not advertise the remote clipboard");
                    let _ = self.os.clear();
                    self.owns_remote = false;
                    self.remote.clear();
                    self.local_mimes = self.os.mime_types();
                    if self.ready {
                        self.advertise_local();
                    }
                }
            }
            Command::Paste(transfer) => self.paste(transfer),
            Command::RemoteData(data) => self.remote_data(data),
            Command::RenderLocal(format) => self.render_local(format),
            Command::Reset => {
                self.invalidate();
                self.pending = None;
                self.ready = false;
                self.remote.clear();
                if self.owns_remote {
                    let _ = self.os.clear();
                }
                self.owns_remote = false;
                self.loops.clear();
            }
            Command::Shutdown => {}
        }
    }

    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.queue.clear();
        self.cache.clear();
        if let Some(pending) = &mut self.pending {
            pending.waiters.clear();
        }
    }

    fn advertise_local(&self) {
        let mut formats = Vec::new();
        if find_mime_match(TEXT, &self.local_mimes).is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        }
        if find_mime_match(PNG, &self.local_mimes).is_some() {
            formats.extend([
                ClipboardFormat::new(ClipboardFormatId::CF_DIB),
                ClipboardFormat::new(ClipboardFormatId::CF_DIBV5),
            ]);
        }
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn paste(&mut self, transfer: Transfer) {
        if !self.owns_remote || transfer.generation() != self.generation {
            return;
        }
        let has = |id| self.remote.iter().any(|f| f.id() == id);
        let format =
            if transfer.mime_type().split(';').next() == Some("text/plain") && has(ClipboardFormatId::CF_UNICODETEXT) {
                ClipboardFormatId::CF_UNICODETEXT
            } else if transfer.mime_type() == PNG && has(ClipboardFormatId::CF_DIBV5) {
                ClipboardFormatId::CF_DIBV5
            } else if transfer.mime_type() == PNG && has(ClipboardFormatId::CF_DIB) {
                ClipboardFormatId::CF_DIB
            } else {
                return;
            };
        if let Some(data) = self.cache.get(&format) {
            transfer.finish(Some(data.clone()));
            return;
        }
        if let Some(pending) = &mut self.pending {
            if pending.expired || MAX_WAITERS <= pending.waiters.len() + self.queue.len() {
                return;
            }
            if pending.generation == self.generation && pending.format == format {
                pending.waiters.push(transfer);
            } else {
                self.queue.push_back(Queued {
                    deadline: Instant::now() + PASTE_TIMEOUT,
                    transfer,
                });
            }
        } else {
            self.start(format, transfer);
        }
    }

    fn start(&mut self, format: ClipboardFormatId, transfer: Transfer) {
        self.pending = Some(Pending {
            format,
            generation: self.generation,
            deadline: Instant::now() + PASTE_TIMEOUT,
            expired: false,
            waiters: vec![transfer],
        });
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
    }

    /// Expire OS waiters, but keep the outstanding wire request until its reply arrives.
    /// [MS-RDPECLIP 2.2.5.2] has no response identifier, so starting another request
    /// after a timeout would misinterpret the late reply as the new clipboard content.
    ///
    /// [MS-RDPECLIP 2.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/28c193b8-4cec-413e-a07b-9235e5e15f6b
    pub fn expire(&mut self, now: Instant) {
        if let Some(pending) = &mut self.pending {
            if pending.deadline <= now {
                pending.expired = true;
                pending.waiters.clear();
            }
        }
        self.queue.retain(|queued| now < queued.deadline);
    }

    fn remote_data(&mut self, data: Option<Vec<u8>>) {
        let Some(pending) = self.pending.take() else {
            debug!("Discarding an unsolicited clipboard response");
            return;
        };
        if !pending.expired
            && Instant::now() < pending.deadline
            && pending.generation == self.generation
            && self.owns_remote
        {
            let decoded = data.filter(|data| data.len() <= MAX_TRANSFER_BYTES).and_then(|data| {
                let result = match pending.format {
                    ClipboardFormatId::CF_UNICODETEXT => FormatDataResponse::new_data(data.as_slice())
                        .to_unicode_string()
                        .map(String::into_bytes)
                        .map_err(|error| error.to_string()),
                    ClipboardFormatId::CF_DIB => bitmap::dib_to_png(&data).map_err(|error| error.to_string()),
                    ClipboardFormatId::CF_DIBV5 => bitmap::dibv5_to_png(&data).map_err(|error| error.to_string()),
                    _ => return None,
                };
                match result {
                    Ok(data) => Some(data),
                    Err(error) => {
                        warn!(%error, "Could not decode the remote clipboard");
                        None
                    }
                }
            });
            if let Some(data) = &decoded {
                self.loops
                    .record_content(data, ClipboardSource::Remote, crate::native_now_ms());
                self.cache.insert(pending.format, data.clone());
            }
            for transfer in pending.waiters {
                transfer.finish(decoded.clone());
            }
        }
        while self.pending.is_none() {
            let Some(queued) = self.queue.pop_front() else {
                break;
            };
            if Instant::now() < queued.deadline {
                self.paste(queued.transfer);
            }
        }
    }

    fn render_local(&mut self, format: ClipboardFormatId) {
        let result = (|| -> Result<Vec<u8>, String> {
            if self.owns_remote {
                return Err("local clipboard selection was replaced".into());
            }
            let mime = match format {
                ClipboardFormatId::CF_UNICODETEXT => TEXT,
                ClipboardFormatId::CF_DIB | ClipboardFormatId::CF_DIBV5 => PNG,
                _ => return Err("unsupported clipboard format".into()),
            };
            let data = self.os.read(mime)?.ok_or("clipboard format is no longer available")?;
            if MAX_TRANSFER_BYTES < data.len() {
                return Err("clipboard data exceeds the size limit".into());
            }
            // Native APIs suppress our own selection events. This also stops a
            // clipboard manager that republishes recently received content.
            if self
                .loops
                .would_cause_content_loop(&data, ClipboardSource::Local, crate::native_now_ms())
            {
                return Err("clipboard content would echo a recent remote paste".into());
            }
            self.loops
                .record_content(&data, ClipboardSource::Local, crate::native_now_ms());
            match format {
                ClipboardFormatId::CF_UNICODETEXT => {
                    let text = core::str::from_utf8(&data).map_err(|error| error.to_string())?;
                    let units = text.encode_utf16().count().saturating_add(1);
                    if MAX_TRANSFER_BYTES / 2 < units {
                        return Err("encoded clipboard text exceeds the size limit".into());
                    }
                    Ok(FormatDataResponse::new_unicode_string(text).into_data().into_owned())
                }
                ClipboardFormatId::CF_DIB => bitmap::png_to_cf_dib(&data).map_err(|error| error.to_string()),
                ClipboardFormatId::CF_DIBV5 => bitmap::png_to_cf_dibv5(&data).map_err(|error| error.to_string()),
                _ => Err("unsupported clipboard format".into()),
            }
        })();
        let response = match result {
            Ok(data) => FormatDataResponse::new_data(data).into_owned(),
            Err(error) => {
                debug!(%error, "Could not render the local clipboard");
                FormatDataResponse::new_error()
            }
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }
}

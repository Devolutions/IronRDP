//! The state machine behind [`LinuxClipboard`](super::LinuxClipboard).
//!
//! Runs on its own thread so the `CLIPRDR` callbacks, which arrive on the RDP
//! session task, never wait on the OS clipboard. The backend forwards each
//! callback as a [`Command`]; the worker owns the [`OsClipboard`], the
//! [`ClipboardMessageProxy`] and every piece of state.

use core::time::Duration;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy};
use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId, FormatDataResponse, OwnedFormatDataResponse};
use ironrdp_cliprdr_format::bitmap;
use ironrdp_core::IntoOwned as _;
use tracing::{debug, warn};

use super::image;
use super::os::{Content, OsClipboard};

/// How often the OS clipboard is re-read for changes. Neither X11 nor the
/// Wayland data-control protocol tells a non-owner when the selection changes,
/// so the clipboard is polled; text is read first and an image only when there
/// is no text, which keeps the common case cheap.
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// How long an outstanding paste request is waited for before a newer remote
/// copy replaces it. The remote may never answer if the application that owned
/// its clipboard went away.
const PENDING_PASTE_TIMEOUT: Duration = Duration::from_secs(5);

/// A `CLIPRDR` callback, forwarded from the backend.
#[derive(Debug)]
pub(super) enum Command {
    /// The channel is up: advertise whatever the OS clipboard holds.
    AdvertiseLocal,
    /// The remote copied something and offers these formats.
    RemoteCopy(Vec<ClipboardFormat>),
    /// The remote wants our clipboard in this format.
    RenderLocal(ClipboardFormatId),
    /// The remote answered our paste request: the bytes, or `None` on failure.
    RemoteData(Option<Vec<u8>>),
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingPaste {
    Text,
    Image { dibv5: bool },
}

pub(super) struct Worker<C, P> {
    os: C,
    proxy: P,
    /// What the remote has been told our clipboard holds and what
    /// [`Command::RenderLocal`] serves.
    local: Option<Content>,
    /// Polling must wait for the server to request the initial format list.
    initialized: bool,
    /// A local copy or newer remote offer superseded the outstanding response.
    discard_pending: bool,
    /// The last thing we wrote to the OS clipboard on the remote's behalf, so
    /// reading it back is not mistaken for a local copy.
    last_written: Option<Content>,
    pending: Option<(PendingPaste, Instant)>,
    /// A remote copy that arrived while `pending` was outstanding.
    desired: Option<(PendingPaste, ClipboardFormatId)>,
}

impl<C: OsClipboard, P: ClipboardMessageProxy> Worker<C, P> {
    pub(super) fn new(os: C, proxy: P) -> Self {
        Self {
            os,
            proxy,
            local: None,
            initialized: false,
            discard_pending: false,
            last_written: None,
            pending: None,
            desired: None,
        }
    }

    pub(super) fn run(mut self, commands: &Receiver<Command>) {
        loop {
            match commands.recv_timeout(POLL_INTERVAL) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(command) => self.handle(command),
                Err(RecvTimeoutError::Timeout) => self.poll(),
            }
        }
    }

    pub(super) fn handle(&mut self, command: Command) {
        match command {
            Command::AdvertiseLocal => {
                self.initialized = true;
                self.local = self.os.read();
                self.advertise();
            }
            Command::RemoteCopy(formats) => self.remote_copy(&formats),
            Command::RenderLocal(format) => self.render_local(format),
            Command::RemoteData(data) => self.remote_data(data),
            Command::Shutdown => {}
        }
    }

    /// Re-reads the OS clipboard and advertises it if it changed.
    pub(super) fn poll(&mut self) {
        if !self.initialized {
            return;
        }
        // A stalled owner must not strand a newer copy indefinitely. Check on
        // the timer, rather than requiring a third copy to trigger recovery.
        if self
            .pending
            .is_some_and(|(_, since)| since.elapsed() >= PENDING_PASTE_TIMEOUT)
        {
            if let Some(next) = self.desired.take() {
                debug!("Pending paste timed out; requesting the newer remote copy");
                self.issue_paste(next);
            }
        }
        let Some(content) = self.os.read() else {
            return;
        };
        if self.last_written.as_ref() == Some(&content) || self.local.as_ref() == Some(&content) {
            return;
        }
        // Something new was copied locally, so the echo guard is stale.
        self.last_written = None;
        self.discard_pending = self.pending.is_some();
        self.desired = None;
        self.local = Some(content);
        self.advertise();
    }

    fn advertise(&self) {
        let formats = self.local.as_ref().map_or_else(Vec::new, advertised_formats);
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(formats));
    }

    fn remote_copy(&mut self, formats: &[ClipboardFormat]) {
        let Some(target) = paste_target(formats) else {
            self.pending = None;
            self.desired = None;
            return;
        };

        if let Some((_, since)) = self.pending {
            if since.elapsed() < PENDING_PASTE_TIMEOUT {
                self.desired = Some(target);
                return;
            }
            debug!("Pending paste timed out; abandoning it for the newer remote copy");
        }

        self.issue_paste(target);
    }

    fn issue_paste(&mut self, (paste, format): (PendingPaste, ClipboardFormatId)) {
        self.pending = Some((paste, Instant::now()));
        self.discard_pending = false;
        self.desired = None;
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
    }

    fn remote_data(&mut self, data: Option<Vec<u8>>) {
        let superseded = self.discard_pending || self.desired.is_some();
        self.discard_pending = false;
        match (self.pending.take(), data) {
            (Some(_), _) if superseded => debug!("Ignoring superseded remote clipboard content"),
            (Some((paste, _)), Some(bytes)) => match decode_remote(paste, &bytes) {
                Ok(content) => self.store_remote(content),
                Err(error) => warn!(%error, "Could not decode the remote clipboard content"),
            },
            (Some(_), None) => debug!("The remote could not provide the requested clipboard format"),
            (None, _) => debug!("Unexpected format data response"),
        }

        if let Some(next) = self.desired.take() {
            self.issue_paste(next);
        }
    }

    fn store_remote(&mut self, content: Content) {
        match self.os.write(&content) {
            Ok(()) => {
                self.last_written = Some(content.clone());
                self.local = Some(content);
            }
            Err(error) => warn!(
                error,
                "Could not write the remote clipboard content to the OS clipboard"
            ),
        }
    }

    fn render_local(&self, format: ClipboardFormatId) {
        let response = match (format, self.local.as_ref()) {
            (ClipboardFormatId::CF_UNICODETEXT, Some(Content::Text(text))) => {
                FormatDataResponse::new_unicode_string(text).into_owned()
            }
            (ClipboardFormatId::CF_DIB, Some(Content::Image(image))) => {
                render_image(image, bitmap::png_to_cf_dib, "CF_DIB")
            }
            (ClipboardFormatId::CF_DIBV5, Some(Content::Image(image))) => {
                render_image(image, bitmap::png_to_cf_dibv5, "CF_DIBV5")
            }
            _ => FormatDataResponse::new_error(),
        };
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendFormatData(response));
    }
}

fn render_image(
    image: &image::Rgba,
    to_dib: fn(&[u8]) -> Result<Vec<u8>, bitmap::BitmapError>,
    name: &str,
) -> OwnedFormatDataResponse {
    let dib = image::rgba_to_png(image)
        .map_err(|error| error.to_string())
        .and_then(|png| to_dib(&png).map_err(|error| error.to_string()));
    match dib {
        Ok(dib) => FormatDataResponse::new_data(dib).into_owned(),
        Err(error) => {
            warn!(%error, format = name, "Could not convert the local image for the remote");
            FormatDataResponse::new_error()
        }
    }
}

/// The formats advertised for one piece of local content.
fn advertised_formats(content: &Content) -> Vec<ClipboardFormat> {
    match content {
        Content::Text(_) => vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
        // DIBV5 carries alpha; DIB is for peers that do not understand it.
        Content::Image(_) => vec![
            ClipboardFormat::new(ClipboardFormatId::CF_DIB),
            ClipboardFormat::new(ClipboardFormatId::CF_DIBV5),
        ],
    }
}

/// Which remote format to fetch: text first, then the richest image.
fn paste_target(formats: &[ClipboardFormat]) -> Option<(PendingPaste, ClipboardFormatId)> {
    let has = |id: ClipboardFormatId| formats.iter().any(|format| format.id() == id);

    if has(ClipboardFormatId::CF_UNICODETEXT) {
        Some((PendingPaste::Text, ClipboardFormatId::CF_UNICODETEXT))
    } else if has(ClipboardFormatId::CF_DIBV5) {
        Some((PendingPaste::Image { dibv5: true }, ClipboardFormatId::CF_DIBV5))
    } else if has(ClipboardFormatId::CF_DIB) {
        Some((PendingPaste::Image { dibv5: false }, ClipboardFormatId::CF_DIB))
    } else {
        None
    }
}

fn decode_remote(paste: PendingPaste, bytes: &[u8]) -> Result<Content, String> {
    match paste {
        PendingPaste::Text => FormatDataResponse::new_data(bytes)
            .to_unicode_string()
            .map(Content::Text)
            .map_err(|error| error.to_string()),
        PendingPaste::Image { dibv5 } => {
            let png = if dibv5 {
                bitmap::dibv5_to_png(bytes)
            } else {
                bitmap::dib_to_png(bytes)
            }
            .map_err(|error| error.to_string())?;
            image::png_to_rgba(&png)
                .map(Content::Image)
                .map_err(|error| error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::linux::image::Rgba;

    #[derive(Default)]
    struct FakeOs {
        content: Option<Content>,
        written: Vec<Content>,
    }

    impl OsClipboard for Arc<Mutex<FakeOs>> {
        fn read(&mut self) -> Option<Content> {
            self.lock().expect("fake os").content.clone()
        }

        fn write(&mut self, content: &Content) -> Result<(), String> {
            let mut os = self.lock().expect("fake os");
            os.content = Some(content.clone());
            os.written.push(content.clone());
            Ok(())
        }
    }

    #[derive(Debug, Default, Clone)]
    struct Recorder(Arc<Mutex<Vec<ClipboardMessage>>>);

    impl ClipboardMessageProxy for Recorder {
        fn send_clipboard_message(&self, message: ClipboardMessage) {
            self.0.lock().expect("recorder").push(message);
        }
    }

    impl Recorder {
        fn take(&self) -> Vec<ClipboardMessage> {
            core::mem::take(&mut *self.0.lock().expect("recorder"))
        }
    }

    type TestWorker = Worker<Arc<Mutex<FakeOs>>, Recorder>;

    fn worker() -> (TestWorker, Arc<Mutex<FakeOs>>, Recorder) {
        let os = Arc::new(Mutex::new(FakeOs::default()));
        let recorder = Recorder::default();
        (Worker::new(Arc::clone(&os), recorder.clone()), os, recorder)
    }

    fn set_local(os: &Arc<Mutex<FakeOs>>, content: Content) {
        os.lock().expect("fake os").content = Some(content);
    }

    fn format_ids(messages: &[ClipboardMessage]) -> Vec<Vec<ClipboardFormatId>> {
        messages
            .iter()
            .filter_map(|message| match message {
                ClipboardMessage::SendInitiateCopy(formats) => Some(formats.iter().map(|f| f.id()).collect()),
                _ => None,
            })
            .collect()
    }

    fn sample_image() -> Rgba {
        Rgba {
            width: 2,
            height: 1,
            bytes: vec![0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0xFF],
        }
    }

    #[test]
    fn polling_before_monitor_ready_does_not_send_clipboard_pdus() {
        let (mut worker, os, recorder) = worker();
        set_local(&os, Content::Text("preexisting local text".to_owned()));
        worker.poll();
        assert!(recorder.take().is_empty());
        worker.handle(Command::AdvertiseLocal);
        assert_eq!(
            format_ids(&recorder.take()),
            vec![vec![ClipboardFormatId::CF_UNICODETEXT]]
        );
    }

    #[test]
    fn a_newer_remote_copy_resumes_on_timeout_without_a_third_copy() {
        let (mut worker, _os, recorder) = worker();
        worker.handle(Command::AdvertiseLocal);
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_UNICODETEXT,
        )]));
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_DIB,
        )]));
        recorder.take();
        worker.pending.as_mut().expect("pending paste").1 = Instant::now() - PENDING_PASTE_TIMEOUT;
        worker.poll();
        assert!(matches!(
            recorder.take().as_slice(),
            [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_DIB)]
        ));
    }

    #[test]
    fn a_local_copy_is_not_overwritten_by_an_older_remote_response() {
        let (mut worker, os, recorder) = worker();
        worker.handle(Command::AdvertiseLocal);
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_UNICODETEXT,
        )]));
        set_local(&os, Content::Text("new local copy".to_owned()));
        worker.poll();
        recorder.take();
        let old = FormatDataResponse::new_unicode_string("old remote copy").into_owned();
        worker.handle(Command::RemoteData(Some(old.data().to_vec())));
        assert!(os.lock().expect("fake os").written.is_empty());
        assert_eq!(
            os.lock().expect("fake os").content,
            Some(Content::Text("new local copy".to_owned()))
        );
    }

    #[test]
    fn channel_ready_advertises_the_current_clipboard() {
        let (mut worker, os, recorder) = worker();
        set_local(&os, Content::Text("hello".to_owned()));

        worker.handle(Command::AdvertiseLocal);

        assert_eq!(
            format_ids(&recorder.take()),
            vec![vec![ClipboardFormatId::CF_UNICODETEXT]]
        );
    }

    #[test]
    fn channel_ready_with_an_empty_clipboard_advertises_nothing() {
        let (mut worker, _os, recorder) = worker();

        worker.handle(Command::AdvertiseLocal);

        assert_eq!(format_ids(&recorder.take()), vec![Vec::<ClipboardFormatId>::new()]);
    }

    #[test]
    fn a_local_change_is_advertised_once() {
        let (mut worker, os, recorder) = worker();
        worker.handle(Command::AdvertiseLocal);
        recorder.take();

        set_local(&os, Content::Text("copied locally".to_owned()));
        worker.poll();
        worker.poll();

        assert_eq!(
            format_ids(&recorder.take()),
            vec![vec![ClipboardFormatId::CF_UNICODETEXT]]
        );

        set_local(&os, Content::Image(sample_image()));
        worker.poll();
        assert_eq!(
            format_ids(&recorder.take()),
            vec![vec![ClipboardFormatId::CF_DIB, ClipboardFormatId::CF_DIBV5]]
        );
    }

    #[test]
    fn remote_text_lands_on_the_os_clipboard_without_echoing_back() {
        let (mut worker, os, recorder) = worker();

        worker.handle(Command::RemoteCopy(vec![
            ClipboardFormat::new(ClipboardFormatId::CF_DIB),
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]));
        let messages = recorder.take();
        assert!(
            matches!(
                messages.as_slice(),
                [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_UNICODETEXT)]
            ),
            "text is preferred over an image: {messages:?}"
        );

        let wire = FormatDataResponse::new_unicode_string("from the remote").into_owned();
        worker.handle(Command::RemoteData(Some(wire.data().to_vec())));

        assert_eq!(
            os.lock().expect("fake os").written,
            vec![Content::Text("from the remote".to_owned())]
        );

        // Reading our own write back must not re-advertise it to the remote.
        worker.poll();
        assert!(recorder.take().is_empty());
    }

    #[test]
    fn remote_image_round_trips_through_dib() {
        let (mut worker, os, recorder) = worker();
        let image = sample_image();

        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_DIB,
        )]));
        assert!(matches!(
            recorder.take().as_slice(),
            [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_DIB)]
        ));

        let png = image::rgba_to_png(&image).expect("png");
        let dib = bitmap::png_to_cf_dib(&png).expect("dib");
        worker.handle(Command::RemoteData(Some(dib)));

        assert_eq!(os.lock().expect("fake os").written, vec![Content::Image(image)]);
    }

    #[test]
    fn a_failed_remote_paste_leaves_the_os_clipboard_alone() {
        let (mut worker, os, _recorder) = worker();
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_UNICODETEXT,
        )]));
        worker.handle(Command::RemoteData(None));
        assert!(os.lock().expect("fake os").written.is_empty());
    }

    #[test]
    fn a_second_remote_copy_waits_for_the_outstanding_paste() {
        let (mut worker, os, recorder) = worker();
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_UNICODETEXT,
        )]));
        worker.handle(Command::RemoteCopy(vec![ClipboardFormat::new(
            ClipboardFormatId::CF_DIB,
        )]));
        assert_eq!(recorder.take().len(), 1, "only one paste request in flight");

        let wire = FormatDataResponse::new_unicode_string("first").into_owned();
        worker.handle(Command::RemoteData(Some(wire.data().to_vec())));
        assert!(
            os.lock().expect("fake os").written.is_empty(),
            "superseded response must not replace the clipboard"
        );
        assert!(matches!(
            recorder.take().as_slice(),
            [ClipboardMessage::SendInitiatePaste(ClipboardFormatId::CF_DIB)]
        ));
    }

    #[test]
    fn local_content_is_rendered_in_the_requested_format() {
        let (mut worker, os, recorder) = worker();
        set_local(&os, Content::Text("render me".to_owned()));
        worker.handle(Command::AdvertiseLocal);
        recorder.take();

        worker.handle(Command::RenderLocal(ClipboardFormatId::CF_UNICODETEXT));
        let messages = recorder.take();
        let [ClipboardMessage::SendFormatData(response)] = messages.as_slice() else {
            panic!("expected one format data response, got {messages:?}");
        };
        assert_eq!(response.to_unicode_string().expect("utf-16"), "render me");

        worker.handle(Command::RenderLocal(ClipboardFormatId::CF_DIB));
        let messages = recorder.take();
        let [ClipboardMessage::SendFormatData(response)] = messages.as_slice() else {
            panic!("expected one format data response, got {messages:?}");
        };
        assert!(response.is_error(), "text cannot be rendered as an image");

        set_local(&os, Content::Image(sample_image()));
        worker.poll();
        recorder.take();
        worker.handle(Command::RenderLocal(ClipboardFormatId::CF_DIBV5));
        let messages = recorder.take();
        let [ClipboardMessage::SendFormatData(response)] = messages.as_slice() else {
            panic!("expected one format data response, got {messages:?}");
        };
        assert!(!response.is_error());
        bitmap::validate_dibv5(response.data()).expect("a well-formed DIBV5");
    }
}

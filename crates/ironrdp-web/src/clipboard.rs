//! This module implements browser-based clipboard backend for CLIPRDR SVC
//!
//! # Implementation notes
//!
//! A catch with web browsers, is that there is no support for delayed clipboard rendering.
//! We can’t know which format will be requested by the target application ultimately.
//! Because of that, we need to fetch optimistically all available formats.
//!
//! For instance, we query both "text/plain" and "text/html". Indeed, depending on the
//! target application in which the user performs the paste operation, either one could be
//! requested: when pasting into notepad, which does not support "text/html", "text/plain"
//! will be requested, and when pasting into WordPad, "text/html" will be requested.

use std::collections::HashMap;

use futures_channel::mpsc;
use iron_remote_desktop::ClipboardData as _;
use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, FileContentsFlags,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr_format::bitmap::{dib_to_png, dibv5_to_png, png_to_cf_dibv5};
use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html};
use ironrdp_core::{IntoOwned as _, impl_as_any};
use tracing::{error, info, trace, warn};
use wasm_bindgen::prelude::*;

use crate::session::RdpInputEvent;

const MIME_TEXT: &str = "text/plain";
const MIME_HTML: &str = "text/html";
const MIME_PNG: &str = "image/png";

#[derive(Clone, Copy)]
struct ClientFormatDescriptor {
    id: ClipboardFormatId,
    name: &'static str,
}

impl ClientFormatDescriptor {
    const fn new(id: ClipboardFormatId, name: &'static str) -> Self {
        Self { id, name }
    }
}

impl From<ClientFormatDescriptor> for ClipboardFormat {
    fn from(descriptor: ClientFormatDescriptor) -> Self {
        ClipboardFormat::new(descriptor.id).with_name(ClipboardFormatName::new_static(descriptor.name))
    }
}

const FORMAT_WIN_HTML_ID: ClipboardFormatId = ClipboardFormatId(0xC001);
const FORMAT_MIME_HTML_ID: ClipboardFormatId = ClipboardFormatId(0xC002);
const FORMAT_PNG_ID: ClipboardFormatId = ClipboardFormatId(0xC003);
const FORMAT_MIME_PNG_ID: ClipboardFormatId = ClipboardFormatId(0xC004);

const FORMAT_WIN_HTML_NAME: &str = "HTML Format";
const FORMAT_MIME_HTML_NAME: &str = "text/html";
const FORMAT_PNG_NAME: &str = "PNG";
const FORMAT_MIME_PNG_NAME: &str = "image/png";

const FORMAT_WIN_HTML: ClientFormatDescriptor = ClientFormatDescriptor::new(FORMAT_WIN_HTML_ID, FORMAT_WIN_HTML_NAME);
const FORMAT_MIME_HTML: ClientFormatDescriptor =
    ClientFormatDescriptor::new(FORMAT_MIME_HTML_ID, FORMAT_MIME_HTML_NAME);
const FORMAT_PNG: ClientFormatDescriptor = ClientFormatDescriptor::new(FORMAT_PNG_ID, FORMAT_PNG_NAME);
const FORMAT_MIME_PNG: ClientFormatDescriptor = ClientFormatDescriptor::new(FORMAT_MIME_PNG_ID, FORMAT_MIME_PNG_NAME);

/// Message proxy used to send clipboard-related messages to the application main event loop
#[derive(Debug, Clone)]
pub(crate) struct WasmClipboardMessageProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl WasmClipboardMessageProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }

    /// Send messages which require action on CLIPRDR SVC
    pub(crate) fn send_cliprdr_message(&self, message: ClipboardMessage) {
        if self.tx.unbounded_send(RdpInputEvent::Cliprdr(message)).is_err() {
            error!("Failed to send os clipboard message, receiver is closed");
        }
    }

    /// Send messages which require action on wasm clipboard backend
    pub(crate) fn send_backend_message(&self, message: WasmClipboardBackendMessage) {
        if self
            .tx
            .unbounded_send(RdpInputEvent::ClipboardBackend(message))
            .is_err()
        {
            error!("Failed to send os clipboard message, receiver is closed");
        }
    }
}

/// Messages sent by the JS code or CLIPRDR to the backend implementation.
#[derive(Debug)]
pub(crate) enum WasmClipboardBackendMessage {
    LocalClipboardChanged(ClipboardData),
    RemoteDataRequest(ClipboardFormatId),

    RemoteClipboardChanged(Vec<ClipboardFormat>),
    RemoteDataResponse(FormatDataResponse<'static>),

    ForceClipboardUpdate,

    // File transfer messages
    /// [MS-RDPECLIP] 2.2.5.2 File list available from remote for download.
    ///
    /// Sent when remote copies files - contains file metadata (names, sizes, timestamps).
    /// The lock ID is included so JS can use it in FileContentsRequest calls.
    FileListAdvertise {
        files: Vec<FileMetadata>,
        clip_data_id: Option<u32>,
    },
    /// [MS-RDPECLIP] 2.2.5.3.1 Remote requests file contents from client (upload).
    ///
    /// Forwarded from remote when user pastes files on remote. JS should read the
    /// requested file chunk and respond via submit_file_contents().
    ///
    /// flags indicates SIZE (return 8-byte u64) or RANGE (return byte range).
    /// position/size define the byte range for RANGE requests.
    /// data_id associates request with locked clipboard (from LockClipData PDU).
    FileContentsRequest {
        stream_id: u32,
        /// Per MS-RDPECLIP 2.2.5.3, lindex is a signed i32.
        /// Must be non-negative (validated by protocol layer).
        index: i32,
        flags: FileContentsFlags,
        position: u64,
        size: u32,
        /// Optional clipboard data ID from LockClipData PDU
        data_id: Option<u32>,
    },
    /// [MS-RDPECLIP] 2.2.5.3.2 Remote sends file contents to client (download).
    ///
    /// Forwarded from remote in response to client's FileContentsRequest.
    /// If is_error is true, the request failed (data unavailable/access denied).
    /// For SIZE requests, data contains 8-byte little-endian u64.
    /// For DATA requests, data contains the requested byte range.
    FileContentsResponse {
        stream_id: u32,
        /// If true, the request failed and data should be ignored
        is_error: bool,
        data: Vec<u8>,
    },
    /// [MS-RDPECLIP] 2.2.4.1 Remote locked their clipboard for file transfer.
    ///
    /// Sent when remote locks their clipboard before we request file contents.
    /// The data_id associates subsequent FileContentsRequest/Response cycles.
    /// Clipboard remains locked until UnlockClipData PDU received.
    Lock {
        data_id: LockDataId,
    },
    /// [MS-RDPECLIP] 2.2.4.2 Remote unlocked their clipboard.
    ///
    /// Sent when remote unlocks clipboard after file transfer completes or
    /// when new clipboard content is copied (auto-unlock per spec).
    Unlock {
        data_id: LockDataId,
    },
    /// Client-side locks expired due to inactivity timeout.
    ///
    /// Sent when automatic cleanup removes locks that have been inactive
    /// for too long or exceeded maximum lifetime. The locks have already
    /// been unlocked by the time this notification is sent.
    ///
    /// JS should clear any references to these lock IDs and abort any
    /// associated file transfers.
    LocksExpired {
        clip_data_ids: Vec<u32>,
    },

    // JS-initiated file transfer operations
    /// JS requests file contents from remote (download).
    ///
    /// Sends FileContentsRequest PDU to remote to request file size or data.
    FileContentsRequestSend {
        stream_id: u32,
        /// Per MS-RDPECLIP 2.2.5.3, lindex is a signed i32.
        /// Must be non-negative (validated by protocol layer).
        index: i32,
        flags: FileContentsFlags,
        position: u64,
        size: u32,
        clip_data_id: Option<u32>,
    },
    /// JS sends file contents to remote (upload response).
    ///
    /// Sends FileContentsResponse PDU to remote with requested file data.
    FileContentsResponseSend {
        stream_id: u32,
        is_error: bool,
        data: Vec<u8>,
    },
    /// JS advertises local files for copy (upload).
    ///
    /// Sends FormatList with FileGroupDescriptorW containing file metadata.
    InitiateFileCopy {
        files: Vec<FileMetadata>,
    },
}

/// Clipboard backend implementation for web. This object should be created once per session and
/// kept alive until session is terminated.
pub(crate) struct WasmClipboard {
    local_clipboard: Option<ClipboardData>,
    remote_clipboard: ClipboardData,

    remote_mapping: HashMap<ClipboardFormatId, String>,
    remote_formats_to_read: Vec<ClipboardFormatId>,

    /// Deferred file list paste: when the remote FormatList includes FileGroupDescriptorW,
    /// we store its format ID here and trigger `SendInitiatePaste` only after all text/image
    /// formats have been fetched (or immediately if no text/image formats are present).
    /// This sequences the file list request after other format requests so that the cliprdr
    /// layer can correctly correlate each FormatDataResponse with its FormatDataRequest.
    pending_file_list_paste: Option<ClipboardFormatId>,

    proxy: WasmClipboardMessageProxy,
    js_callbacks: JsClipboardCallbacks,
}

/// Callbacks, required to interact with JS code from within the backend.
pub(crate) struct JsClipboardCallbacks {
    pub(crate) on_remote_clipboard_changed: js_sys::Function,
    pub(crate) on_force_clipboard_update: Option<js_sys::Function>,
    // File transfer callbacks
    pub(crate) on_files_available: Option<js_sys::Function>,
    pub(crate) on_file_contents_request: Option<js_sys::Function>,
    pub(crate) on_file_contents_response: Option<js_sys::Function>,
    pub(crate) on_lock: Option<js_sys::Function>,
    pub(crate) on_unlock: Option<js_sys::Function>,
    pub(crate) on_locks_expired: Option<js_sys::Function>,
}

impl WasmClipboard {
    pub(crate) fn new(message_proxy: WasmClipboardMessageProxy, js_callbacks: JsClipboardCallbacks) -> Self {
        Self {
            local_clipboard: None,
            remote_clipboard: ClipboardData::new(),
            proxy: message_proxy,
            js_callbacks,

            remote_mapping: HashMap::new(),
            remote_formats_to_read: Vec::new(),
            pending_file_list_paste: None,
        }
    }

    /// Returns CLIPRDR backend implementation
    pub(crate) fn backend(&self) -> WasmClipboardBackend {
        WasmClipboardBackend {
            proxy: self.proxy.clone(),
        }
    }

    fn handle_local_clipboard_changed(
        &mut self,
        clipboard_data: ClipboardData,
    ) -> anyhow::Result<Vec<ClipboardFormat>> {
        let mut formats = Vec::new();
        clipboard_data.items().iter().for_each(|item| {
            match item.mime_type.as_str() {
                MIME_TEXT => formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)),
                MIME_HTML => {
                    formats.extend([
                        // We don't provide CF_TEXT, because it could be synthesized from
                        // CF_UNICODETEXT on the remote side.
                        ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
                        FORMAT_WIN_HTML.into(),
                        FORMAT_MIME_HTML.into(),
                    ]);
                }
                MIME_PNG => {
                    formats.extend([
                        // We don't provide CF_DIB, because it could be synthesized from
                        // CF_DIBV5 on the remote side.
                        ClipboardFormat::new(ClipboardFormatId::CF_DIBV5),
                        FORMAT_PNG.into(),
                        FORMAT_MIME_PNG.into(),
                    ]);
                }
                _ => {}
            };
        });

        self.local_clipboard = Some(clipboard_data);

        trace!(?formats, "Sending clipboard formats");

        Ok(formats)
    }

    fn process_remote_data_request(
        &mut self,
        format: ClipboardFormatId,
    ) -> anyhow::Result<FormatDataResponse<'static>> {
        // Transaction is not set, bail!
        let clipboard_data = if let Some(clipboard_data) = &self.local_clipboard {
            clipboard_data
        } else {
            anyhow::bail!("Local clipboard is empty");
        };

        let find_content_by_mime = |mime: &str| {
            clipboard_data
                .items()
                .iter()
                .find(|item| item.mime_type.as_str() == mime)
        };

        let find_text_content_by_mime = |mime: &str| {
            find_content_by_mime(mime)
                .and_then(|item| {
                    if let ClipboardItemValue::Text(text) = &item.value {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("Failed to find `{mime}` in client clipboard"))
        };

        let find_binary_content_by_mime = |mime: &str| {
            find_content_by_mime(mime)
                .and_then(|item| {
                    if let ClipboardItemValue::Binary(binary) = &item.value {
                        Some(binary.as_slice())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("Failed to find `{mime}` in client clipboard"))
        };

        let response = match format {
            ClipboardFormatId::CF_UNICODETEXT => {
                let text = find_text_content_by_mime(MIME_TEXT)?;
                FormatDataResponse::new_unicode_string(text)
            }
            FORMAT_WIN_HTML_ID => {
                let html_text = find_text_content_by_mime(MIME_HTML)?;
                let cf_html = plain_html_to_cf_html(html_text);
                FormatDataResponse::new_data(cf_html.into_bytes())
            }
            FORMAT_MIME_HTML_ID => {
                let html_text = find_text_content_by_mime(MIME_HTML)?;
                FormatDataResponse::new_string(html_text)
            }
            ClipboardFormatId::CF_DIBV5 => {
                let png_data = find_binary_content_by_mime(MIME_PNG)?;
                let buffer = png_to_cf_dibv5(png_data)?;
                FormatDataResponse::new_data(buffer)
            }
            FORMAT_MIME_PNG_ID | FORMAT_PNG_ID => {
                let png_data = find_binary_content_by_mime(MIME_PNG)?;
                FormatDataResponse::new_data(png_data)
            }
            _ => {
                anyhow::bail!("Unknown format id requested: {}", format.value());
            }
        };

        Ok(response.into_owned())
    }

    fn process_remote_clipboard_changed(
        &mut self,
        formats: Vec<ClipboardFormat>,
    ) -> anyhow::Result<Option<ClipboardFormatId>> {
        self.remote_clipboard.clear();
        self.remote_mapping.clear();
        self.pending_file_list_paste = None;

        // We accumulate all formats in the `remote_formats_to_read` attribute.
        // Later, we loop over and fetch all of these (see `process_remote_data_response`).
        //
        // SAFETY (stale response concern): clearing both `remote_mapping` and
        // `remote_formats_to_read` here is safe because the WASM runtime is
        // single-threaded. Any in-flight `FormatDataResponse` for the previous
        // format list will be processed after this function returns. At that
        // point `remote_formats_to_read` is either empty (response dropped by
        // the guard in `process_remote_data_response`) or repopulated with the
        // new format list, so stale data cannot be misattributed to a new format.
        self.remote_formats_to_read.clear();

        // In this loop, we ignore some formats. There are two reasons for that:
        //
        // 1) Some formats require an extra conversion into the appropriate MIME format
        // prior to being written to the system clipboard.
        // E.g.: "image/png" format is preferred over "CF_DIB" because we'll convert the
        // uncompressed BMP into "image/png". "text/html" is preferred over Windows
        // "CF_HTML" because we'll convert it into "text/html".
        //
        // 2) A direct consequence of 1) is that some formats will end up being mapped
        // into the same MIME type. Fetching only one of these is enough, especially given
        // that delayed rendering is not an option.
        for format in &formats {
            if format.id().is_registered() {
                if let Some(name) = format.name() {
                    // [MS-RDPECLIP] 2.2.5.2 FileGroupDescriptorW: file transfer format.
                    // Handled separately from text/image because the cliprdr layer's
                    // `initiate_paste()` + `handle_format_data_response()` intercept path
                    // parses the file descriptors and calls `on_remote_file_list()`.
                    // We defer this paste until after all text/image formats are fetched
                    // to avoid conflicts with the FormatDataRequest/Response chain.
                    if name.value() == ClipboardFormatName::FILE_LIST.value() {
                        self.pending_file_list_paste = Some(format.id());
                        continue;
                    }

                    const SUPPORTED_FORMATS: &[&str] = &[
                        FORMAT_WIN_HTML.name,
                        FORMAT_MIME_HTML.name,
                        FORMAT_PNG.name,
                        FORMAT_MIME_PNG.name,
                    ];

                    if !SUPPORTED_FORMATS.iter().any(|supported| *supported == name.value()) {
                        // Unknown format
                        continue;
                    }

                    let skip_win_html = format_name_eq(format, FORMAT_WIN_HTML.name)
                        && formats
                            .iter()
                            .any(|format| format_name_eq(format, FORMAT_MIME_HTML.name));

                    let skip_mime_png = format_name_eq(format, FORMAT_MIME_PNG.name)
                        && formats.iter().any(|format| format_name_eq(format, FORMAT_PNG.name));

                    if skip_win_html || skip_mime_png {
                        continue;
                    }

                    self.remote_mapping.insert(format.id(), name.value().to_owned());
                }
            } else {
                const SUPPORTED_FORMATS: &[ClipboardFormatId] = &[
                    ClipboardFormatId::CF_UNICODETEXT,
                    ClipboardFormatId::CF_DIB,
                    ClipboardFormatId::CF_DIBV5,
                ];

                if !SUPPORTED_FORMATS.contains(&format.id()) {
                    // Unknown format
                    continue;
                }

                let skip_dib = format.id() == ClipboardFormatId::CF_DIB
                    && formats.iter().any(|format| {
                        format.id() == ClipboardFormatId::CF_DIBV5
                            || format_name_eq(format, FORMAT_MIME_PNG.name)
                            || format_name_eq(format, FORMAT_PNG.name)
                    });

                let skip_dibv5 = format.id() == ClipboardFormatId::CF_DIBV5
                    && formats.iter().any(|format| {
                        format_name_eq(format, FORMAT_MIME_PNG.name) || format_name_eq(format, FORMAT_PNG.name)
                    });

                if skip_dib || skip_dibv5 {
                    continue;
                }
            }

            self.remote_formats_to_read.push(format.id());
        }

        return Ok(self.remote_formats_to_read.last().copied());

        fn format_name_eq(format: &ClipboardFormat, name: &str) -> bool {
            format
                .name()
                .map(|actual: &ClipboardFormatName| actual.value() == name)
                .unwrap_or(false)
        }
    }

    fn process_remote_data_response(&mut self, response: FormatDataResponse<'_>) -> anyhow::Result<()> {
        let pending_format = match self.remote_formats_to_read.pop() {
            Some(format) => format,
            None => {
                warn!("Remote returned format data, but no formats were requested");
                return Ok(());
            }
        };

        if response.is_error() {
            // Format is not available anymore.
            return Ok(());
        }

        let item = match pending_format {
            ClipboardFormatId::CF_UNICODETEXT => match response.to_unicode_string() {
                Ok(text) => Some(ClipboardItem::new_text(MIME_TEXT, text)),
                Err(err) => {
                    error!(error = %err, "CF_UNICODETEXT decode error");
                    None
                }
            },
            ClipboardFormatId::CF_DIB => match dib_to_png(response.data()) {
                Ok(png) => Some(ClipboardItem::new_binary(MIME_PNG, png)),
                Err(err) => {
                    warn!(error = %err, "DIB decode error");
                    None
                }
            },
            ClipboardFormatId::CF_DIBV5 => match dibv5_to_png(response.data()) {
                Ok(png) => Some(ClipboardItem::new_binary(MIME_PNG, png)),
                Err(err) => {
                    warn!(error = %err, "DIBv5 decode error");
                    None
                }
            },
            registered => {
                let format_name = self.remote_mapping.get(&registered).map(|s| s.as_str());

                match format_name {
                    Some(FORMAT_WIN_HTML_NAME) => match cf_html_to_plain_html(response.data()) {
                        Ok(text) => Some(ClipboardItem::new_text(MIME_HTML, text.to_owned())),
                        Err(err) => {
                            warn!(error = %err, "CF_HTML decode error");
                            None
                        }
                    },
                    Some(FORMAT_MIME_HTML_NAME) => match response.to_string() {
                        Ok(text) => Some(ClipboardItem::new_text(MIME_HTML, text)),
                        Err(err) => {
                            warn!(error = %err, "text/html decode error");
                            None
                        }
                    },
                    Some(FORMAT_MIME_PNG_NAME) | Some(FORMAT_PNG_NAME) => {
                        Some(ClipboardItem::new_binary(MIME_PNG, response.data().to_owned()))
                    }
                    _ => {
                        // Not supported format
                        None
                    }
                }
            }
        };

        if let Some(item) = item {
            self.remote_clipboard.add(item);
        }

        if let Some(format) = self.remote_formats_to_read.last() {
            // Request next format.
            self.proxy
                .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(*format));
        } else {
            // All text/image formats were read, send clipboard data to JS.
            let clipboard_data = core::mem::take(&mut self.remote_clipboard);

            if !clipboard_data.is_empty() {
                if let Err(e) = self.js_callbacks.on_remote_clipboard_changed.call1(
                    &JsValue::NULL,
                    &JsValue::from(crate::wasm_bridge::ClipboardData::from(clipboard_data)),
                ) {
                    error!(error = ?e, "Failed to call remote clipboard changed callback");
                }
            }

            // Now trigger the deferred file list fetch if the FormatList included
            // FileGroupDescriptorW. The cliprdr layer handles parsing the file
            // descriptors and calling `on_remote_file_list()` automatically.
            if let Some(file_format) = self.pending_file_list_paste.take() {
                self.proxy
                    .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(file_format));
            }
        }

        Ok(())
    }

    /// Process backend event. This method should be called from the main event loop.
    pub(crate) fn process_event(&mut self, event: WasmClipboardBackendMessage) -> anyhow::Result<()> {
        match event {
            WasmClipboardBackendMessage::LocalClipboardChanged(clipboard_data) => {
                match self.handle_local_clipboard_changed(clipboard_data) {
                    Ok(formats) => {
                        self.proxy
                            .send_cliprdr_message(ClipboardMessage::SendInitiateCopy(formats));
                    }
                    Err(e) => {
                        // Not a critical error, we could skip single clipboard update.
                        error!(error = format!("{e:#}"), "Failed to handle local clipboard change");
                    }
                }
            }
            WasmClipboardBackendMessage::RemoteDataRequest(format) => {
                let message = match self.process_remote_data_request(format) {
                    Ok(message) => message,
                    Err(e) => {
                        // Not a critical error, but we should notify remote about it.
                        error!(error = format!("{e:#}"), "Failed to process remote data request");
                        FormatDataResponse::new_error()
                    }
                };
                self.proxy
                    .send_cliprdr_message(ClipboardMessage::SendFormatData(message));
            }
            WasmClipboardBackendMessage::RemoteClipboardChanged(formats) => {
                match self.process_remote_clipboard_changed(formats) {
                    Ok(Some(format)) => {
                        // We start querying text/image formats right away. This is due to
                        // absence of delay-rendering in web client.
                        // If a file list format is also pending, it will be triggered after
                        // all text/image formats are fetched (see process_remote_data_response).
                        self.proxy
                            .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(format));
                    }
                    Ok(None) => {
                        // No text/image formats to query. If a file list format was detected,
                        // trigger it immediately since there's no text/image fetch chain to
                        // wait for.
                        if let Some(file_format) = self.pending_file_list_paste.take() {
                            self.proxy
                                .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(file_format));
                        }
                    }
                    Err(e) => {
                        error!(error = format!("{e:#}"), "Failed to process remote clipboard change");
                    }
                }
            }
            WasmClipboardBackendMessage::RemoteDataResponse(formats) => {
                match self.process_remote_data_response(formats) {
                    Ok(()) => {}
                    Err(e) => {
                        error!(error = format!("{e:#}"), "Failed to process remote data response");
                    }
                }
            }
            WasmClipboardBackendMessage::ForceClipboardUpdate => {
                if let Some(callback) = self.js_callbacks.on_force_clipboard_update.as_mut() {
                    if let Err(e) = callback.call0(&JsValue::NULL) {
                        error!(error = ?e, "Failed to call JS force clipboard update callback");
                        return Ok(());
                    }
                } else {
                    // If no initial clipboard callback was set, send empty format list instead
                    return self
                        .process_event(WasmClipboardBackendMessage::LocalClipboardChanged(ClipboardData::new()));
                }
            }
            WasmClipboardBackendMessage::FileListAdvertise { files, clip_data_id } => {
                if let Some(callback) = self.js_callbacks.on_files_available.as_ref() {
                    // Convert FileMetadata vector to JS array.
                    // Reflect::set on a fresh Object practically never fails, but we
                    // log and skip rather than panicking the entire WASM module.
                    let js_array = js_sys::Array::new();
                    for file in &files {
                        let js_file = js_sys::Object::new();
                        if let Err(e) = js_sys::Reflect::set(&js_file, &"name".into(), &JsValue::from_str(&file.name)) {
                            error!(error = ?e, field = "name", file_name = %file.name, "Failed to set JS file metadata property");
                        }
                        // Set path if present (relative directory within the copied collection)
                        if let Some(path) = &file.path {
                            if let Err(e) = js_sys::Reflect::set(&js_file, &"path".into(), &JsValue::from_str(path)) {
                                error!(error = ?e, field = "path", file_name = %file.name, "Failed to set JS file metadata property");
                            }
                        }
                        #[expect(clippy::as_conversions, clippy::cast_precision_loss)]
                        let size_f64 = file.size as f64;
                        if let Err(e) = js_sys::Reflect::set(&js_file, &"size".into(), &JsValue::from_f64(size_f64)) {
                            error!(error = ?e, field = "size", file_name = %file.name, "Failed to set JS file metadata property");
                        }
                        #[expect(clippy::as_conversions, clippy::cast_precision_loss)]
                        let last_modified_f64 = file.last_modified as f64;
                        if let Err(e) = js_sys::Reflect::set(
                            &js_file,
                            &"lastModified".into(),
                            &JsValue::from_f64(last_modified_f64),
                        ) {
                            error!(error = ?e, field = "lastModified", file_name = %file.name, "Failed to set JS file metadata property");
                        }
                        if let Err(e) = js_sys::Reflect::set(
                            &js_file,
                            &"isDirectory".into(),
                            &JsValue::from_bool(file.is_directory),
                        ) {
                            error!(error = ?e, field = "isDirectory", file_name = %file.name, "Failed to set JS file metadata property");
                        }
                        js_array.push(&js_file);
                    }
                    let clip_data_id_js = match clip_data_id {
                        Some(id) => JsValue::from_f64(f64::from(id)),
                        None => JsValue::UNDEFINED,
                    };
                    if let Err(e) = callback.call2(&JsValue::NULL, &js_array, &clip_data_id_js) {
                        error!(error = ?e, file_count = files.len(), "Failed to call JS files available callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        file_count = files.len(),
                        "File list available but no JS callback registered"
                    );
                }
            }
            WasmClipboardBackendMessage::FileContentsRequest {
                stream_id,
                index,
                flags,
                position,
                size,
                data_id,
            } => {
                if let Some(callback) = self.js_callbacks.on_file_contents_request.as_ref() {
                    let js_request = js_sys::Object::new();
                    if let Err(e) = js_sys::Reflect::set(
                        &js_request,
                        &"streamId".into(),
                        &JsValue::from_f64(f64::from(stream_id)),
                    ) {
                        error!(error = ?e, field = "streamId", stream_id, "Failed to set JS file contents request property");
                    }
                    if let Err(e) =
                        js_sys::Reflect::set(&js_request, &"index".into(), &JsValue::from_f64(f64::from(index)))
                    {
                        error!(error = ?e, field = "index", stream_id, "Failed to set JS file contents request property");
                    }
                    if let Err(e) = js_sys::Reflect::set(
                        &js_request,
                        &"flags".into(),
                        &JsValue::from_f64(f64::from(flags.bits())),
                    ) {
                        error!(error = ?e, field = "flags", stream_id, "Failed to set JS file contents request property");
                    }
                    #[expect(clippy::as_conversions, clippy::cast_precision_loss)]
                    let position_f64 = position as f64;
                    if let Err(e) =
                        js_sys::Reflect::set(&js_request, &"position".into(), &JsValue::from_f64(position_f64))
                    {
                        error!(error = ?e, field = "position", stream_id, "Failed to set JS file contents request property");
                    }
                    if let Err(e) =
                        js_sys::Reflect::set(&js_request, &"size".into(), &JsValue::from_f64(f64::from(size)))
                    {
                        error!(error = ?e, field = "size", stream_id, "Failed to set JS file contents request property");
                    }
                    // data_id is optional - only set if present
                    if let Some(id) = data_id {
                        if let Err(e) =
                            js_sys::Reflect::set(&js_request, &"dataId".into(), &JsValue::from_f64(f64::from(id)))
                        {
                            error!(error = ?e, field = "dataId", stream_id, "Failed to set JS file contents request property");
                        }
                    }
                    if let Err(e) = callback.call1(&JsValue::NULL, &js_request) {
                        error!(error = ?e, stream_id, "Failed to call JS file contents request callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        stream_id,
                        index, "File contents request from remote but no JS callback registered"
                    );
                }
            }
            WasmClipboardBackendMessage::FileContentsResponse {
                stream_id,
                is_error,
                data,
            } => {
                if let Some(callback) = self.js_callbacks.on_file_contents_response.as_ref() {
                    let js_response = js_sys::Object::new();
                    if let Err(e) = js_sys::Reflect::set(
                        &js_response,
                        &"streamId".into(),
                        &JsValue::from_f64(f64::from(stream_id)),
                    ) {
                        error!(error = ?e, field = "streamId", stream_id, "Failed to set JS file contents response property");
                    }
                    if let Err(e) = js_sys::Reflect::set(&js_response, &"isError".into(), &JsValue::from_bool(is_error))
                    {
                        error!(error = ?e, field = "isError", stream_id, "Failed to set JS file contents response property");
                    }
                    if let Err(e) =
                        js_sys::Reflect::set(&js_response, &"data".into(), &js_sys::Uint8Array::from(data.as_slice()))
                    {
                        error!(error = ?e, field = "data", stream_id, data_len = data.len(), "Failed to set JS file contents response property");
                    }
                    if let Err(e) = callback.call1(&JsValue::NULL, &js_response) {
                        error!(error = ?e, stream_id, "Failed to call JS file contents response callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        stream_id,
                        is_error,
                        data_len = data.len(),
                        "File contents response from remote but no JS callback registered"
                    );
                }
            }
            WasmClipboardBackendMessage::Lock { data_id } => {
                if let Some(callback) = self.js_callbacks.on_lock.as_ref() {
                    if let Err(e) = callback.call1(&JsValue::NULL, &JsValue::from_f64(f64::from(data_id.0))) {
                        error!(error = ?e, data_id = data_id.0, "Failed to call JS lock callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        data_id = data_id.0,
                        "Clipboard lock received but no JS callback registered"
                    );
                }
            }
            WasmClipboardBackendMessage::Unlock { data_id } => {
                if let Some(callback) = self.js_callbacks.on_unlock.as_ref() {
                    if let Err(e) = callback.call1(&JsValue::NULL, &JsValue::from_f64(f64::from(data_id.0))) {
                        error!(error = ?e, data_id = data_id.0, "Failed to call JS unlock callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        data_id = data_id.0,
                        "Clipboard unlock received but no JS callback registered"
                    );
                }
            }
            WasmClipboardBackendMessage::LocksExpired { clip_data_ids } => {
                if let Some(callback) = self.js_callbacks.on_locks_expired.as_ref() {
                    let js_array = js_sys::Uint32Array::from(clip_data_ids.as_slice());
                    if let Err(e) = callback.call1(&JsValue::NULL, &js_array) {
                        error!(error = ?e, count = clip_data_ids.len(), "Failed to call JS locks expired callback");
                        return Ok(());
                    }
                } else {
                    warn!(
                        count = clip_data_ids.len(),
                        "Clipboard locks expired but no JS callback registered"
                    );
                }
            }
            // The following variants are handled directly in the event loop and should never reach here
            WasmClipboardBackendMessage::FileContentsRequestSend { .. }
            | WasmClipboardBackendMessage::FileContentsResponseSend { .. }
            | WasmClipboardBackendMessage::InitiateFileCopy { .. } => {
                error!("Outbound file transfer message should not reach WasmClipboard::process_event");
                anyhow::bail!("Unexpected outbound file transfer message in clipboard backend");
            }
        };

        Ok(())
    }
}

/// CLIPRDR backend implementation for web. This object could be instantiated via [`WasmClipboard`]
/// to pass it to CLIPRDR SVC constructor.
#[derive(Debug)]
pub(crate) struct WasmClipboardBackend {
    proxy: WasmClipboardMessageProxy,
}

impl WasmClipboardBackend {
    fn send_event(&self, event: WasmClipboardBackendMessage) {
        self.proxy.send_backend_message(event);
    }
}

impl_as_any!(WasmClipboardBackend);

impl CliprdrBackend for WasmClipboardBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // [MS-RDPECLIP] 2.2.2.1 General Capability Set (CLIPRDR_GENERAL_CAPABILITY)
        // Advertise file transfer support via CLIPRDR virtual channel:
        // - STREAM_FILECLIP_ENABLED: support stream-based file copy/paste via FileContentsRequest/Response PDUs
        // - FILECLIP_NO_FILE_PATHS: file descriptors must not include source paths (security)
        // - CAN_LOCK_CLIPDATA: support clipboard locking during file transfer via LockData/UnlockData PDUs
        // - HUGE_FILE_SUPPORT_ENABLED: support files >4GB (positions/sizes use 64-bit values)
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        // Initial clipboard is assumed to be empty on WASM (TODO: This is only relevant for Firefox?)
        self.send_event(WasmClipboardBackendMessage::ForceClipboardUpdate);
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        info!(?capabilities, "CLIPRDR negotiated capabilities");

        if !capabilities.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED) {
            warn!("CB_STREAM_FILECLIP_ENABLED not negotiated - file transfers will not work");
        }
        if !capabilities.contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA) {
            warn!("CB_CAN_LOCK_CLIPDATA not negotiated - file transfer reliability may be reduced");
        }
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send_event(WasmClipboardBackendMessage::RemoteClipboardChanged(
            available_formats.to_vec(),
        ));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send_event(WasmClipboardBackendMessage::RemoteDataRequest(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send_event(WasmClipboardBackendMessage::RemoteDataResponse(response.into_owned()));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        // Forward file contents request to JS to retrieve file data
        self.send_event(WasmClipboardBackendMessage::FileContentsRequest {
            stream_id: request.stream_id,
            index: request.index,
            flags: request.flags,
            position: request.position,
            size: request.requested_size,
            data_id: request.data_id,
        });
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        // Forward file contents response to JS (for downloads from remote)
        self.send_event(WasmClipboardBackendMessage::FileContentsResponse {
            stream_id: response.stream_id(),
            is_error: response.is_error(),
            data: response.data().to_owned(),
        });
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.send_event(WasmClipboardBackendMessage::Lock { data_id });
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.send_event(WasmClipboardBackendMessage::Unlock { data_id });
    }

    fn on_remote_file_list(&mut self, files: &[ironrdp::cliprdr::pdu::FileDescriptor], clip_data_id: Option<u32>) {
        let file_metadata: Vec<FileMetadata> = files.iter().map(FileMetadata::from_file_descriptor).collect();

        self.send_event(WasmClipboardBackendMessage::FileListAdvertise {
            files: file_metadata,
            clip_data_id,
        });
    }

    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        // Notify JS that locks expired due to inactivity timeout
        // JS should clear references and abort associated transfers
        if !clip_data_ids.is_empty() {
            self.send_event(WasmClipboardBackendMessage::LocksExpired {
                clip_data_ids: clip_data_ids.iter().map(|id| id.0).collect(),
            });
        }
    }

    fn now_ms(&self) -> u64 {
        // Prefer Performance.now() for a monotonic clock that won't jump
        // backwards on NTP adjustments. Fall back to Date.now() if the
        // Performance API is unavailable (e.g. in non-browser WASM runtimes).
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation, clippy::as_conversions)]
        {
            web_sys::window()
                .and_then(|w| w.performance())
                .map_or_else(|| js_sys::Date::now() as u64, |p| p.now() as u64)
        }
    }

    fn elapsed_ms(&self, since: u64) -> u64 {
        self.now_ms().saturating_sub(since)
    }
}

/// Object which represents complete clipboard transaction with multiple MIME types.
#[derive(Debug, Default, Clone)]
pub(crate) struct ClipboardData {
    items: Vec<ClipboardItem>,
}

impl ClipboardData {
    pub(crate) fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub(crate) fn add(&mut self, item: ClipboardItem) {
        self.items.push(item);
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }
}

impl iron_remote_desktop::ClipboardData for ClipboardData {
    type Item = ClipboardItem;

    fn create() -> Self {
        Self::new()
    }

    fn add_text(&mut self, mime_type: &str, text: &str) {
        self.items.push(ClipboardItem {
            mime_type: mime_type.to_owned(),
            value: ClipboardItemValue::Text(text.to_owned()),
        })
    }

    fn add_binary(&mut self, mime_type: &str, binary: &[u8]) {
        self.items.push(ClipboardItem {
            mime_type: mime_type.to_owned(),
            value: ClipboardItemValue::Binary(binary.to_owned()),
        })
    }

    fn items(&self) -> &[Self::Item] {
        &self.items
    }
}

impl FromIterator<ClipboardItem> for ClipboardData {
    fn from_iter<T: IntoIterator<Item = ClipboardItem>>(iter: T) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ClipboardItemValue {
    Text(String),
    Binary(Vec<u8>),
}

/// Object which represents single clipboard format represented standard MIME type.
#[derive(Debug, Clone)]
pub(crate) struct ClipboardItem {
    mime_type: String,
    value: ClipboardItemValue,
}

impl ClipboardItem {
    pub(crate) fn new_text(mime_type: impl Into<String>, text: String) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardItemValue::Text(text),
        }
    }

    pub(crate) fn new_binary(mime_type: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            mime_type: mime_type.into(),
            value: ClipboardItemValue::Binary(payload),
        }
    }
}

impl iron_remote_desktop::ClipboardItem for ClipboardItem {
    fn mime_type(&self) -> &str {
        &self.mime_type
    }

    #[expect(refining_impl_trait)]
    fn value(&self) -> JsValue {
        match &self.value {
            ClipboardItemValue::Text(text) => JsValue::from_str(text),
            ClipboardItemValue::Binary(binary) => JsValue::from(js_sys::Uint8Array::from(binary.as_slice())),
        }
    }
}

/// File metadata for JS interop.
///
/// Simplified representation of [FileDescriptor] for WASM/JS boundary.
/// When passed to JavaScript, u64 values are converted to f64 which may lose
/// precision for files larger than 2^53 bytes (~9 PB). In practice, this is
/// acceptable because:
/// - Files >9 PB are extremely rare
/// - JavaScript Number has ~15-16 decimal digits of precision
/// - File systems typically don't support files that large
///
/// ## [MS-RDPECLIP] Spec Notes
/// Per 2.2.5.2.3.1, file names must be ≤259 characters (leaving room for null
/// terminator in 260-character field). Names must not be empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileMetadata {
    /// File name (basename including extension, without directory path).
    /// Per [MS-RDPECLIP] 2.2.5.2.3.1: max 259 chars, non-empty, no path separators.
    pub(crate) name: String,
    /// Relative directory path within the copied collection, using `\` as separator.
    /// `None` for root-level files.
    /// Per [MS-RDPECLIP] 3.1.1.2, file lists use relative paths (e.g., `temp\file1.txt`).
    pub(crate) path: Option<String>,
    /// File size in bytes.
    ///
    /// When constructed via [`FileMetadata::from_file_descriptor`], `None`
    /// (unknown) is mapped to `0`. In [`FileMetadata::to_file_descriptor`],
    /// the size is always reported as known (`Some(self.size)`), which is
    /// correct for the JS-to-remote upload path where JS `File.size` always
    /// provides a concrete value.
    ///
    /// **Note**: When converted to JavaScript f64, precision loss occurs for sizes >2^53.
    pub(crate) size: u64,
    /// Last write time as a JavaScript timestamp (milliseconds since Unix epoch 1970-01-01).
    /// 0 indicates unknown or not applicable (valid for files without timestamps).
    /// Converted from/to Windows FILETIME at the WASM boundary so JS consumers can use
    /// the value directly (e.g., `new Date(lastModified)`).
    pub(crate) last_modified: u64,
    /// Whether this entry represents a directory rather than a file.
    pub(crate) is_directory: bool,
}

impl FileMetadata {
    pub(crate) fn from_file_descriptor(desc: &ironrdp::cliprdr::pdu::FileDescriptor) -> Self {
        use ironrdp::cliprdr::pdu::ClipboardFileAttributes;

        let is_directory = desc
            .attributes
            .map(|a| a.contains(ClipboardFileAttributes::DIRECTORY))
            .unwrap_or(false);

        // Convert Windows FILETIME (100-ns intervals since 1601-01-01) to
        // JavaScript timestamp (ms since Unix epoch 1970-01-01). This mirrors
        // the reverse conversion in session.rs for the upload path.
        const WINDOWS_EPOCH_DIFF: u64 = 116_444_736_000_000_000;
        const TICKS_PER_MS: u64 = 10_000;
        let last_modified = match desc.last_write_time {
            Some(ft) if ft >= WINDOWS_EPOCH_DIFF => (ft - WINDOWS_EPOCH_DIFF) / TICKS_PER_MS,
            _ => 0,
        };

        Self {
            name: desc.name.clone(),
            path: desc.relative_path.clone(),
            size: desc.file_size.unwrap_or(0),
            last_modified,
            is_directory,
        }
    }

    pub(crate) fn to_file_descriptor(&self) -> anyhow::Result<ironrdp::cliprdr::pdu::FileDescriptor> {
        use ironrdp::cliprdr::pdu::{ClipboardFileAttributes, FileDescriptor};

        // [MS-RDPECLIP] 2.2.5.2.3.1: File names must be <=259 characters (leaving room for null terminator).
        // Check the full wire name (relative_path + \ + name) since that's what goes on the wire.
        if self.name.is_empty() {
            anyhow::bail!("File name cannot be empty per MS-RDPECLIP 2.2.5.2.3.1");
        }
        let wire_len = match &self.path {
            Some(p) if !p.is_empty() => p.chars().count() + 1 + self.name.chars().count(),
            _ => self.name.chars().count(),
        };
        if wire_len > 259 {
            anyhow::bail!(
                "Wire file name exceeds 259 character limit per MS-RDPECLIP 2.2.5.2.3.1 (has {wire_len} chars)",
            );
        }

        let attributes = if self.is_directory {
            Some(ClipboardFileAttributes::DIRECTORY)
        } else {
            Some(ClipboardFileAttributes::NORMAL)
        };

        // Convert JavaScript timestamp (ms since Unix epoch) back to Windows
        // FILETIME (100-ns intervals since 1601-01-01) for the wire format.
        // This mirrors the reverse conversion in from_file_descriptor().
        const WINDOWS_EPOCH_DIFF: u64 = 116_444_736_000_000_000;
        const TICKS_PER_MS: u64 = 10_000;
        let last_write_time = if self.last_modified > 0 {
            Some(
                self.last_modified
                    .saturating_mul(TICKS_PER_MS)
                    .saturating_add(WINDOWS_EPOCH_DIFF),
            )
        } else {
            None
        };

        let mut desc = FileDescriptor::new(self.name.clone()).with_file_size(self.size);
        if let Some(attrs) = attributes {
            desc = desc.with_attributes(attrs);
        }
        if let Some(time) = last_write_time {
            desc = desc.with_last_write_time(time);
        }
        if let Some(path) = self.path.clone() {
            desc = desc.with_relative_path(path);
        }
        Ok(desc)
    }
}

#[cfg(test)]
mod tests {
    use ironrdp::cliprdr::pdu::FileDescriptor;
    use ironrdp_core::AsAny as _;

    use super::*;

    // Helper to create a test message proxy
    fn create_test_proxy() -> (WasmClipboardMessageProxy, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (tx, rx) = mpsc::unbounded();
        (WasmClipboardMessageProxy::new(tx), rx)
    }

    // Mock JS callbacks that don't panic
    fn create_test_callbacks() -> JsClipboardCallbacks {
        JsClipboardCallbacks {
            on_remote_clipboard_changed: js_sys::Function::new_no_args(""),
            on_force_clipboard_update: Some(js_sys::Function::new_no_args("")),
            on_files_available: Some(js_sys::Function::new_no_args("")),
            on_file_contents_request: Some(js_sys::Function::new_no_args("")),
            on_file_contents_response: Some(js_sys::Function::new_no_args("")),
            on_lock: Some(js_sys::Function::new_no_args("")),
            on_unlock: Some(js_sys::Function::new_no_args("")),
            on_locks_expired: Some(js_sys::Function::new_no_args("")),
        }
    }

    #[test]
    fn test_wasm_clipboard_backend_new() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let clipboard = WasmClipboard::new(proxy, callbacks);

        assert!(clipboard.local_clipboard.is_none());
        assert!(clipboard.remote_mapping.is_empty());
    }

    #[test]
    fn test_local_clipboard_text_formats() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/plain", "Hello World".to_owned()));

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        // Should contain CF_UNICODETEXT
        assert!(formats.iter().any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT));
    }

    #[test]
    fn test_local_clipboard_html_formats() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/html", "<p>Hello</p>".to_owned()));

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        // Should contain CF_UNICODETEXT, HTML Format, and text/html
        assert!(formats.iter().any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT));
        assert!(formats.iter().any(|f| f.id() == FORMAT_WIN_HTML_ID));
        assert!(formats.iter().any(|f| f.id() == FORMAT_MIME_HTML_ID));
    }

    #[test]
    fn test_local_clipboard_image_formats() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        let png_data = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]; // PNG magic
        data.items.push(ClipboardItem::new_binary("image/png", png_data));

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        // Should contain CF_DIBV5, PNG, and image/png
        assert!(formats.iter().any(|f| f.id() == ClipboardFormatId::CF_DIBV5));
        assert!(formats.iter().any(|f| f.id() == FORMAT_PNG_ID));
        assert!(formats.iter().any(|f| f.id() == FORMAT_MIME_PNG_ID));
    }

    #[test]
    fn test_process_remote_data_request_no_local_clipboard() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        // Request format when no local clipboard is set
        let result = clipboard.process_remote_data_request(ClipboardFormatId::CF_UNICODETEXT);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Local clipboard is empty"));
    }

    #[test]
    fn test_process_remote_data_request_text() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/plain", "Hello World".to_owned()));
        clipboard.handle_local_clipboard_changed(data).unwrap();

        let response = clipboard
            .process_remote_data_request(ClipboardFormatId::CF_UNICODETEXT)
            .unwrap();

        assert!(!response.is_error());
        let data = response.into_owned().into_data();
        // Should be UTF-16LE encoded with null terminator
        assert!(!data.is_empty());
    }

    #[test]
    fn test_message_proxy_send_cliprdr_message() {
        let (proxy, mut rx) = create_test_proxy();

        proxy.send_cliprdr_message(ClipboardMessage::SendInitiateCopy(vec![]));

        // Check message was sent
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(_)) => {}
            _ => panic!("Expected Cliprdr message"),
        }
    }

    #[test]
    fn test_message_proxy_send_backend_message() {
        let (proxy, mut rx) = create_test_proxy();

        proxy.send_backend_message(WasmClipboardBackendMessage::ForceClipboardUpdate);

        // Check message was sent
        match rx.try_recv() {
            Ok(RdpInputEvent::ClipboardBackend(_)) => {}
            _ => panic!("Expected ClipboardBackend message"),
        }
    }

    #[test]
    fn test_file_metadata_conversions() {
        let file_desc = FileDescriptor::new("test.txt")
            .with_last_write_time(132_000_000_000_000_000)
            .with_file_size(1024);

        let metadata = FileMetadata::from_file_descriptor(&file_desc);

        assert_eq!(metadata.name, "test.txt");
        assert_eq!(metadata.size, 1024);
        assert!(metadata.last_modified > 0); // Should be converted to Unix timestamp
    }

    #[test]
    fn test_file_metadata_from_js_timestamp() {
        let now = 1_700_000_000_000u64; // JavaScript timestamp (ms)

        let metadata = FileMetadata {
            name: "test.txt".to_owned(),
            path: None,
            size: 1024,
            last_modified: now,
            is_directory: false,
        };

        let file_desc = metadata.to_file_descriptor().unwrap();

        assert_eq!(file_desc.name, "test.txt");
        assert_eq!(file_desc.file_size, Some(1024));
        assert!(file_desc.last_write_time.is_some());
    }

    #[test]
    fn test_process_event_local_clipboard_changed() {
        let (proxy, mut rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/plain", "test".to_owned()));

        let result = clipboard.process_event(WasmClipboardBackendMessage::LocalClipboardChanged(data));

        assert!(result.is_ok());

        // Should have sent SendInitiateCopy message
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendInitiateCopy(_))) => {}
            _ => panic!("Expected SendInitiateCopy message"),
        }
    }

    #[test]
    fn test_process_event_remote_data_request() {
        let (proxy, mut rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        // Set up local clipboard first
        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/plain", "test".to_owned()));
        clipboard.handle_local_clipboard_changed(data).unwrap();

        // Clear the SendInitiateCopy message
        let _ = rx.try_recv();

        let result = clipboard.process_event(WasmClipboardBackendMessage::RemoteDataRequest(
            ClipboardFormatId::CF_UNICODETEXT,
        ));

        assert!(result.is_ok());

        // Should have sent SendFormatData message
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendFormatData(_))) => {}
            _ => panic!("Expected SendFormatData message"),
        }
    }

    #[test]
    fn test_process_event_remote_data_request_error() {
        let (proxy, mut rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        // Don't set up local clipboard - should result in error response
        let result = clipboard.process_event(WasmClipboardBackendMessage::RemoteDataRequest(
            ClipboardFormatId::CF_UNICODETEXT,
        ));

        assert!(result.is_ok()); // Event processing succeeds, but sends error response

        // Should have sent SendFormatData with error
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendFormatData(response))) => {
                assert!(response.is_error());
            }
            _ => panic!("Expected SendFormatData error message"),
        }
    }

    #[test]
    fn test_process_event_force_clipboard_update() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let result = clipboard.process_event(WasmClipboardBackendMessage::ForceClipboardUpdate);

        // With callback present, should succeed without panicking
        assert!(result.is_ok());
    }

    #[test]
    fn test_process_event_force_clipboard_update_fallback() {
        let (proxy, mut rx) = create_test_proxy();
        let mut callbacks = create_test_callbacks();
        callbacks.on_force_clipboard_update = None; // No callback
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let result = clipboard.process_event(WasmClipboardBackendMessage::ForceClipboardUpdate);

        assert!(result.is_ok());

        // Should fall back to sending empty LocalClipboardChanged
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendInitiateCopy(formats))) => {
                assert!(formats.is_empty()); // Empty clipboard
            }
            _ => panic!("Expected empty SendInitiateCopy message"),
        }
    }

    #[test]
    fn test_clipboard_backend_capabilities() {
        let (proxy, _rx) = create_test_proxy();
        let backend = WasmClipboardBackend { proxy };

        let caps = backend.client_capabilities();

        // Should include file transfer capabilities
        assert!(caps.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED));
        assert!(caps.contains(ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS));
        assert!(caps.contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA));
    }

    #[test]
    fn test_clipboard_backend_as_any() {
        let (proxy, _rx) = create_test_proxy();
        let mut backend = WasmClipboardBackend { proxy };

        // Test AsAny trait implementation
        let any_ref = backend.as_any();
        assert!(any_ref.is::<WasmClipboardBackend>());

        let any_mut = backend.as_any_mut();
        assert!(any_mut.is::<WasmClipboardBackend>());
    }

    #[test]
    fn test_multiple_clipboard_items() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_text("text/plain", "plain text".to_owned()));
        data.items
            .push(ClipboardItem::new_text("text/html", "<p>html</p>".to_owned()));

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        // Should contain formats for both text and HTML
        assert!(formats.len() >= 3); // CF_UNICODETEXT, HTML Format, text/html
    }

    #[test]
    fn test_empty_clipboard() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let data = ClipboardData::new(); // Empty

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        assert!(formats.is_empty());
    }

    #[test]
    fn test_unsupported_mime_type() {
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let mut data = ClipboardData::new();
        data.items
            .push(ClipboardItem::new_binary("application/octet-stream", vec![1, 2, 3]));

        let formats = clipboard.handle_local_clipboard_changed(data).unwrap();

        // Unsupported MIME type should not generate any formats
        assert!(formats.is_empty());
    }

    #[test]
    fn test_backend_temporary_directory() {
        let (proxy, _rx) = create_test_proxy();
        let backend = WasmClipboardBackend { proxy };

        let temp_dir = backend.temporary_directory();

        // Should return a valid path
        assert!(!temp_dir.is_empty());
    }

    #[test]
    fn test_file_descriptor_with_minimal_fields() {
        let file_desc = FileDescriptor::new("minimal.txt");

        let metadata = FileMetadata::from_file_descriptor(&file_desc);

        assert_eq!(metadata.name, "minimal.txt");
        assert_eq!(metadata.size, 0); // None maps to 0 (unknown size)
        assert_eq!(metadata.last_modified, 0); // None maps to 0 (unknown timestamp)
    }

    #[test]
    fn test_file_descriptor_round_trip() {
        let original = FileMetadata {
            name: "roundtrip.txt".to_owned(),
            path: None,
            size: 4096,
            last_modified: 1_700_000_000_000,
            is_directory: false,
        };

        let file_desc = original.to_file_descriptor().unwrap();
        let converted = FileMetadata::from_file_descriptor(&file_desc);

        assert_eq!(converted.name, original.name);
        assert_eq!(converted.path, original.path);
        assert_eq!(converted.size, original.size);
        assert_eq!(converted.is_directory, original.is_directory);
        // Note: timestamp conversion may lose some precision, so we check within tolerance
        let diff = converted.last_modified.abs_diff(original.last_modified);
        assert!(diff < 1000);
    }

    #[test]
    fn test_zero_size_file_reports_known_size() {
        // Zero-byte files (e.g. .gitkeep) should produce file_size: Some(0),
        // not None (which means "unknown" per MS-RDPECLIP 2.2.5.2.3.1).
        let metadata = FileMetadata {
            name: ".gitkeep".to_owned(),
            path: None,
            size: 0,
            last_modified: 0,
            is_directory: false,
        };

        let file_desc = metadata.to_file_descriptor().unwrap();
        assert_eq!(file_desc.file_size, Some(0));
    }

    // Helper: create a ClipboardFormat for FileGroupDescriptorW with a given registered ID.
    fn file_list_format(id: u32) -> ClipboardFormat {
        ClipboardFormat::new(ClipboardFormatId::new(id)).with_name(ClipboardFormatName::FILE_LIST)
    }

    #[test]
    fn test_remote_clipboard_changed_detects_file_format_in_mixed_list() {
        // A mixed FormatList with text and FileGroupDescriptorW should:
        // - Add text format to remote_formats_to_read
        // - Store file format in pending_file_list_paste (NOT in remote_formats_to_read)
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let formats = vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            file_list_format(0xC080),
        ];

        let result = clipboard.process_remote_clipboard_changed(formats).unwrap();

        // Should return CF_UNICODETEXT as the format to fetch
        assert_eq!(result, Some(ClipboardFormatId::CF_UNICODETEXT));
        // File format should NOT be in remote_formats_to_read
        assert!(
            !clipboard
                .remote_formats_to_read
                .contains(&ClipboardFormatId::new(0xC080)),
            "FileGroupDescriptorW should not be in remote_formats_to_read"
        );
        // File format should be stored for deferred paste
        assert_eq!(clipboard.pending_file_list_paste, Some(ClipboardFormatId::new(0xC080)));
    }

    #[test]
    fn test_remote_clipboard_changed_files_only() {
        // A FormatList with only FileGroupDescriptorW should return None
        // (no text/image formats to fetch) and store the file format for deferred paste.
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let formats = vec![file_list_format(0xC080)];

        let result = clipboard.process_remote_clipboard_changed(formats).unwrap();

        assert_eq!(result, None, "No text/image formats should be returned");
        assert_eq!(clipboard.pending_file_list_paste, Some(ClipboardFormatId::new(0xC080)));
    }

    #[test]
    fn test_process_event_files_only_triggers_immediate_paste() {
        // When FormatList contains only FileGroupDescriptorW, process_event should
        // immediately send SendInitiatePaste for the file format.
        let (proxy, mut rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        let formats = vec![file_list_format(0xC080)];
        clipboard
            .process_event(WasmClipboardBackendMessage::RemoteClipboardChanged(formats))
            .unwrap();

        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendInitiatePaste(format))) => {
                assert_eq!(format, ClipboardFormatId::new(0xC080));
            }
            other => panic!("Expected SendInitiatePaste for file format, got: {other:?}"),
        }
    }

    #[test]
    fn test_deferred_file_paste_after_text_formats() {
        // When FormatList has text + files, the file format paste should be deferred
        // until all text formats are fetched. Simulate the full chain:
        // 1. RemoteClipboardChanged with text + file
        // 2. First SendInitiatePaste for text
        // 3. RemoteDataResponse for text
        // 4. After text is done, SendInitiatePaste for file format should be sent
        let (proxy, mut rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        // Step 1: Remote clipboard changed with text + file
        let formats = vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            file_list_format(0xC080),
        ];
        clipboard
            .process_event(WasmClipboardBackendMessage::RemoteClipboardChanged(formats))
            .unwrap();

        // Step 2: Should get SendInitiatePaste for text first
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendInitiatePaste(format))) => {
                assert_eq!(format, ClipboardFormatId::CF_UNICODETEXT);
            }
            other => panic!("Expected SendInitiatePaste for CF_UNICODETEXT, got: {other:?}"),
        }

        // Step 3: Simulate FormatDataResponse for text (null-terminated UTF-16LE "hi")
        let text_data: Vec<u8> = vec![0x68, 0x00, 0x69, 0x00, 0x00, 0x00]; // "hi\0" in UTF-16LE
        let response = FormatDataResponse::new_data(text_data);
        clipboard
            .process_event(WasmClipboardBackendMessage::RemoteDataResponse(response.into_owned()))
            .unwrap();

        // Step 4: After text is done, should get SendInitiatePaste for file format
        match rx.try_recv() {
            Ok(RdpInputEvent::Cliprdr(ClipboardMessage::SendInitiatePaste(format))) => {
                assert_eq!(
                    format,
                    ClipboardFormatId::new(0xC080),
                    "Deferred file list paste should fire after text formats are done"
                );
            }
            other => panic!("Expected deferred SendInitiatePaste for file format, got: {other:?}"),
        }
    }

    #[test]
    fn test_new_format_list_clears_pending_file_paste() {
        // A new FormatList should clear any pending file list paste from a previous one.
        let (proxy, _rx) = create_test_proxy();
        let callbacks = create_test_callbacks();
        let mut clipboard = WasmClipboard::new(proxy, callbacks);

        // First FormatList with files
        let formats = vec![file_list_format(0xC080)];
        clipboard.process_remote_clipboard_changed(formats).unwrap();
        assert!(clipboard.pending_file_list_paste.is_some());

        // Second FormatList without files replaces the first
        let formats = vec![ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
        clipboard.process_remote_clipboard_changed(formats).unwrap();
        assert_eq!(
            clipboard.pending_file_list_paste, None,
            "New FormatList without files should clear pending file paste"
        );
    }
}

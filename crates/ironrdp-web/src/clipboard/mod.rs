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

mod transaction;

use std::collections::HashMap;

use futures_channel::mpsc;
use ironrdp::cliprdr::backend::{ClipboardMessage, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp_cliprdr_format::bitmap::{dib_to_png, dibv5_to_png, png_to_cf_dibv5};
use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html};
use ironrdp_core::{impl_as_any, IntoOwned};
use transaction::{ClipboardContent, ClipboardContentValue};
use wasm_bindgen::prelude::*;

use crate::session::RdpInputEvent;

#[rustfmt::skip]
pub(crate) use transaction::ClipboardTransaction;

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
    LocalClipboardChanged(ClipboardTransaction),
    RemoteDataRequest(ClipboardFormatId),

    RemoteClipboardChanged(Vec<ClipboardFormat>),
    RemoteDataResponse(FormatDataResponse<'static>),

    FormatListReceived,
    ForceClipboardUpdate,
}

/// Clipboard backend implementation for web. This object should be created once per session and
/// kept alive until session is terminated.
pub(crate) struct WasmClipboard {
    local_clipboard: Option<ClipboardTransaction>,
    remote_clipboard: ClipboardTransaction,

    remote_mapping: HashMap<ClipboardFormatId, String>,
    remote_formats_to_read: Vec<ClipboardFormatId>,

    proxy: WasmClipboardMessageProxy,
    js_callbacks: JsClipboardCallbacks,
}

/// Callbacks, required to interact with JS code from within the backend.
pub(crate) struct JsClipboardCallbacks {
    pub(crate) on_remote_clipboard_changed: js_sys::Function,
    pub(crate) on_remote_received_format_list: Option<js_sys::Function>,
    pub(crate) on_force_clipboard_update: Option<js_sys::Function>,
}

impl WasmClipboard {
    pub(crate) fn new(message_proxy: WasmClipboardMessageProxy, js_callbacks: JsClipboardCallbacks) -> Self {
        Self {
            local_clipboard: None,
            remote_clipboard: ClipboardTransaction::construct(),
            proxy: message_proxy,
            js_callbacks,

            remote_mapping: HashMap::new(),
            remote_formats_to_read: Vec::new(),
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
        transaction: ClipboardTransaction,
    ) -> anyhow::Result<Vec<ClipboardFormat>> {
        let mut formats = Vec::new();
        transaction.contents().iter().for_each(|content| {
            match content.mime_type() {
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

        self.local_clipboard = Some(transaction);

        trace!("Sending clipboard formats: {:?}", formats);

        Ok(formats)
    }

    fn process_remote_data_request(
        &mut self,
        format: ClipboardFormatId,
    ) -> anyhow::Result<FormatDataResponse<'static>> {
        // Transaction is not set, bail!
        let transaction = if let Some(transaction) = &self.local_clipboard {
            transaction
        } else {
            anyhow::bail!("Local clipboard is empty");
        };

        let find_content_by_mime = |mime: &str| {
            transaction
                .contents()
                .iter()
                .find(|content| content.mime_type() == mime)
        };

        let find_text_content_by_mime = |mime: &str| {
            find_content_by_mime(mime)
                .and_then(|content| {
                    if let ClipboardContentValue::Text(text) = content.value() {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("Failed to find `{mime}` in client clipboard"))
        };

        let find_binary_content_by_mime = |mime: &str| {
            find_content_by_mime(mime)
                .and_then(|content| {
                    if let ClipboardContentValue::Binary(binary) = content.value() {
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

        // We accumulate all formats in the `remote_formats_to_read` attribute.
        // Later, we loop over and fetch all of these (see `process_remote_data_response`).
        self.remote_formats_to_read.clear();

        // In this loop, we ignore some formats. There are two reasons for that:
        //
        // 1) Some formats require an extra conversion into the appropriate MIME format
        // prior to being written to the system clipboard.
        // E.g.: "image/png" format is preferred over "CF_DIB" because we’ll convert the
        // uncompressed BMP into "image/png". "text/html" is preferred over Windows
        // "CF_HTML" because we’ll convert it into "text/html".
        //
        // 2) A direct consequence of 1) is that some formats will end up being mapped
        // into the same MIME type. Fetching only one of these is enough, especially given
        // that delayed rendering is not an option.
        for format in &formats {
            if format.id().is_registered() {
                if let Some(name) = format.name() {
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

        let content = match pending_format {
            ClipboardFormatId::CF_UNICODETEXT => match response.to_unicode_string() {
                Ok(text) => Some(ClipboardContent::new_text(MIME_TEXT, &text)),
                Err(err) => {
                    error!("CF_UNICODETEXT decode error: {}", err);
                    None
                }
            },
            ClipboardFormatId::CF_DIB => match dib_to_png(response.data()) {
                Ok(png) => Some(ClipboardContent::new_binary(MIME_PNG, &png)),
                Err(err) => {
                    warn!("DIB decode error: {}", err);
                    None
                }
            },
            ClipboardFormatId::CF_DIBV5 => match dibv5_to_png(response.data()) {
                Ok(png) => Some(ClipboardContent::new_binary(MIME_PNG, &png)),
                Err(err) => {
                    warn!("DIBv5 decode error: {}", err);
                    None
                }
            },
            registered => {
                let format_name = self.remote_mapping.get(&registered).map(|s| s.as_str());
                match format_name {
                    Some(FORMAT_WIN_HTML_NAME) => match cf_html_to_plain_html(response.data()) {
                        Ok(text) => Some(ClipboardContent::new_text(MIME_HTML, text)),
                        Err(err) => {
                            warn!("CF_HTML decode error: {}", err);
                            None
                        }
                    },
                    Some(FORMAT_MIME_HTML_NAME) => match response.to_string() {
                        Ok(text) => Some(ClipboardContent::new_text(MIME_HTML, &text)),
                        Err(err) => {
                            warn!("text/html decode error: {}", err);
                            None
                        }
                    },
                    Some(FORMAT_MIME_PNG_NAME) | Some(FORMAT_PNG_NAME) => {
                        Some(ClipboardContent::new_binary(MIME_PNG, response.data()))
                    }
                    _ => {
                        // Not supported format
                        None
                    }
                }
            }
        };

        if let Some(content) = content {
            self.remote_clipboard.add_content(content);
        }

        if let Some(format) = self.remote_formats_to_read.last() {
            // Request next format.
            self.proxy
                .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(*format));
        } else {
            // All formats were read, send clipboard to JS
            let transaction = core::mem::take(&mut self.remote_clipboard);
            if transaction.is_empty() {
                return Ok(());
            }
            // Set clipboard when all formats were read
            self.js_callbacks
                .on_remote_clipboard_changed
                .call1(&JsValue::NULL, &JsValue::from(transaction))
                .expect("Failed to call JS callback");
        }

        Ok(())
    }

    /// Process backend event. This method should be called from the main event loop.
    pub(crate) fn process_event(&mut self, event: WasmClipboardBackendMessage) -> anyhow::Result<()> {
        match event {
            WasmClipboardBackendMessage::LocalClipboardChanged(transaction) => {
                match self.handle_local_clipboard_changed(transaction) {
                    Ok(formats) => {
                        self.proxy
                            .send_cliprdr_message(ClipboardMessage::SendInitiateCopy(formats));
                    }
                    Err(err) => {
                        // Not a critical error, we could skip single clipboard update
                        error!("Failed to handle local clipboard change: {}", err);
                    }
                }
            }
            WasmClipboardBackendMessage::RemoteDataRequest(format) => {
                let message = match self.process_remote_data_request(format) {
                    Ok(message) => message,
                    Err(err) => {
                        // Not a critical error, but we should notify remote about error
                        error!("Failed to process remote data request: {}", err);
                        FormatDataResponse::new_error()
                    }
                };
                self.proxy
                    .send_cliprdr_message(ClipboardMessage::SendFormatData(message));
            }
            WasmClipboardBackendMessage::RemoteClipboardChanged(formats) => {
                match self.process_remote_clipboard_changed(formats) {
                    Ok(Some(format)) => {
                        // We start querying formats right away. This is due absence of
                        // delay-rendering in web client.
                        self.proxy
                            .send_cliprdr_message(ClipboardMessage::SendInitiatePaste(format));
                    }
                    Ok(None) => {
                        // No formats to query
                    }
                    Err(err) => {
                        error!("Failed to process remote clipboard change: {}", err);
                    }
                }
            }
            WasmClipboardBackendMessage::RemoteDataResponse(formats) => {
                match self.process_remote_data_response(formats) {
                    Ok(()) => {}
                    Err(err) => {
                        error!("Failed to process remote data response: {}", err);
                    }
                }
            }
            WasmClipboardBackendMessage::FormatListReceived => {
                if let Some(callback) = self.js_callbacks.on_remote_received_format_list.as_mut() {
                    callback.call0(&JsValue::NULL).expect("Failed to call JS callback");
                }
            }
            WasmClipboardBackendMessage::ForceClipboardUpdate => {
                if let Some(callback) = self.js_callbacks.on_force_clipboard_update.as_mut() {
                    callback.call0(&JsValue::NULL).expect("Failed to call JS callback");
                } else {
                    // If no initial clipboard callback was set, send empty format list instead
                    return self.process_event(WasmClipboardBackendMessage::LocalClipboardChanged(
                        ClipboardTransaction::construct(),
                    ));
                }
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
        // No additional capabilities yet
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_request_format_list(&mut self) {
        // Initial clipboard is assumed to be empty on WASM (TODO: This is only relevant for Firefox?)
        self.send_event(WasmClipboardBackendMessage::ForceClipboardUpdate);
    }

    fn on_format_list_received(&mut self) {
        self.send_event(WasmClipboardBackendMessage::FormatListReceived);
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {
        // No additional capabilities yet
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

    fn on_file_contents_request(&mut self, _request: FileContentsRequest) {
        // File transfer not implemented yet
    }

    fn on_file_contents_response(&mut self, _response: FileContentsResponse<'_>) {
        // File transfer not implemented yet
    }

    fn on_lock(&mut self, _data_id: LockDataId) {
        // File transfer not implemented yet
    }

    fn on_unlock(&mut self, _data_id: LockDataId) {
        // File transfer not implemented yet
    }
}

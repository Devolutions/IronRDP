use wasm_bindgen::JsValue;
use web_sys::{HtmlCanvasElement, js_sys};

use crate::clipboard::ClipboardData;
use crate::error::IronError;
use crate::input::InputTransaction;
use crate::{DesktopSize, Extension};

pub trait SessionBuilder {
    type Session: Session;
    type Error: IronError;

    fn create() -> Self;

    #[must_use]
    fn username(&self, username: String) -> Self;

    #[must_use]
    fn destination(&self, destination: String) -> Self;

    #[must_use]
    fn server_domain(&self, server_domain: String) -> Self;

    #[must_use]
    fn password(&self, password: String) -> Self;

    #[must_use]
    fn proxy_address(&self, address: String) -> Self;

    #[must_use]
    fn auth_token(&self, token: String) -> Self;

    #[must_use]
    fn desktop_size(&self, desktop_size: DesktopSize) -> Self;

    #[must_use]
    fn render_canvas(&self, canvas: HtmlCanvasElement) -> Self;

    #[must_use]
    fn set_cursor_style_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn set_cursor_style_callback_context(&self, context: JsValue) -> Self;

    #[must_use]
    fn remote_clipboard_changed_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn force_clipboard_update_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn canvas_resized_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn files_available_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn file_contents_request_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn file_contents_response_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn lock_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn unlock_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn locks_expired_callback(&self, callback: js_sys::Function) -> Self;

    #[must_use]
    fn extension(&self, ext: Extension) -> Self;

    #[expect(async_fn_in_trait)]
    async fn connect(&self) -> Result<Self::Session, Self::Error>;
}

pub trait Session {
    type SessionTerminationInfo: SessionTerminationInfo;
    type InputTransaction: InputTransaction;
    type ClipboardData: ClipboardData;
    type Error: IronError;

    fn run(&self) -> impl Future<Output = Result<Self::SessionTerminationInfo, Self::Error>>;

    fn desktop_size(&self) -> DesktopSize;

    fn apply_inputs(&self, transaction: Self::InputTransaction) -> Result<(), Self::Error>;

    fn release_all_inputs(&self) -> Result<(), Self::Error>;

    fn synchronize_lock_keys(
        &self,
        scroll_lock: bool,
        num_lock: bool,
        caps_lock: bool,
        kana_lock: bool,
    ) -> Result<(), Self::Error>;

    fn shutdown(&self) -> Result<(), Self::Error>;

    fn on_clipboard_paste(&self, content: &Self::ClipboardData) -> impl Future<Output = Result<(), Self::Error>>;

    fn resize(
        &self,
        width: u32,
        height: u32,
        scale_factor: Option<u32>,
        physical_width: Option<u32>,
        physical_height: Option<u32>,
    );

    fn supports_unicode_keyboard_shortcuts(&self) -> bool;

    fn invoke_extension(&self, ext: Extension) -> Result<JsValue, Self::Error>;

    /// Requests file contents from the remote clipboard.
    ///
    /// Used to download files from the remote. Can request either file size or file data.
    /// The remote responds via the `fileContentsResponseCallback`.
    ///
    /// # Arguments
    /// * `stream_id` - Unique identifier for this file transfer stream
    /// * `file_index` - Index of the file in the file list (0-based)
    /// * `flags` - FileContentsFlags bitmask:
    ///   - `0x1` (SIZE): Request file size (response is 8-byte u64)
    ///   - `0x2` (RANGE): Request byte range
    /// * `position` - Byte offset for DATA requests (0 for SIZE requests)
    /// * `size` - Number of bytes requested for DATA requests (8 for SIZE requests)
    /// * `clip_data_id` - Optional clipboard lock ID (provided via `filesAvailableCallback`)
    ///
    /// # MS-RDPECLIP Reference
    /// [MS-RDPECLIP] 2.2.5.3 - File Contents Request PDU
    ///
    /// # Panics
    ///
    /// The default implementation panics. Implementors that do not support
    /// file transfer must not call this method.
    ///
    /// # Example: Download a file
    /// ```text
    /// // When filesAvailableCallback receives files and clipDataId:
    /// // First request size
    /// requestFileContents(1, 0, 0x1, 0, 8, clipDataId);
    /// // Wait for fileContentsResponseCallback with size
    /// // Then request data in chunks
    /// requestFileContents(1, 0, 0x2, 0, 4096, clipDataId);
    /// // Wait for fileContentsResponseCallback with data
    /// unlockClipboard(clipDataId);
    /// ```
    fn request_file_contents(
        &self,
        _stream_id: u32,
        _file_index: i32,
        _flags: u32,
        _position: u64,
        _size: u32,
        _clip_data_id: Option<u32>,
    ) -> Result<(), Self::Error> {
        unimplemented!("file transfer not supported by this session implementation")
    }

    /// Submits file contents to the remote in response to a file contents request.
    ///
    /// Called in response to `fileContentsRequestCallback` when the remote requests
    /// file upload from the client.
    ///
    /// # Arguments
    /// * `stream_id` - Stream ID from the request
    /// * `is_error` - `true` if the request failed (file unavailable/access denied)
    /// * `data` - File data:
    ///   - For SIZE requests: 8-byte little-endian u64 file size
    ///   - For DATA requests: requested byte range
    ///   - For errors: empty or error-specific data
    ///
    /// # MS-RDPECLIP Reference
    /// [MS-RDPECLIP] 2.2.5.4 - File Contents Response PDU
    ///
    /// # Panics
    ///
    /// The default implementation panics. Implementors that do not support
    /// file transfer must not call this method.
    ///
    /// # Example: Upload a file
    /// ```text
    /// // In fileContentsRequestCallback(request):
    /// if (request.flags & 0x1) {  // SIZE request
    ///   const size = file.size;
    ///   const sizeBytes = new Uint8Array(8);
    ///   new DataView(sizeBytes.buffer).setBigUint64(0, BigInt(size), true);
    ///   submitFileContents(request.streamId, false, sizeBytes);
    /// } else if (request.flags & 0x2) {  // DATA request
    ///   const chunk = await file.slice(request.position, request.position + request.size).arrayBuffer();
    ///   submitFileContents(request.streamId, false, new Uint8Array(chunk));
    /// }
    /// ```
    fn submit_file_contents(&self, _stream_id: u32, _is_error: bool, _data: Vec<u8>) -> Result<(), Self::Error> {
        unimplemented!("file transfer not supported by this session implementation")
    }

    /// Initiates a file copy operation by advertising local files to the remote.
    ///
    /// Sends a Format List with FileGroupDescriptorW to the remote, making local
    /// files available for the remote to download. The remote can then request
    /// file contents via `fileContentsRequestCallback`.
    ///
    /// # Arguments
    /// * `files` - JavaScript array of file metadata objects with properties:
    ///   - `name` (string): File name (max 259 characters)
    ///   - `size` (number): File size in bytes
    ///   - `lastModified` (number): Last modified timestamp as milliseconds
    ///     since the Unix epoch (1970-01-01), matching the JS `File.lastModified`
    ///     property. Converted to Windows FILETIME on the Rust side. Use 0 if unknown.
    ///
    /// # Design note
    ///
    /// This method accepts a raw `JsValue` rather than a typed Rust struct because
    /// the `iron-remote-desktop` crate defines the WASM-facing session trait. The
    /// file metadata array originates from JavaScript (e.g. from `File` objects or
    /// the `DataTransfer` API) and is destructured in the `ironrdp-web` session
    /// implementation. Using `JsValue` avoids requiring `wasm_bindgen` types in
    /// this crate's public API and keeps the trait implementable from pure Rust.
    ///
    /// # MS-RDPECLIP Reference
    /// [MS-RDPECLIP] 2.2.5.2.3 - File Descriptor (CLIPRDR_FILEDESCRIPTOR)
    ///
    /// # Panics
    ///
    /// The default implementation panics. Implementors that do not support
    /// file transfer must not call this method.
    ///
    /// # Example: Upload files to remote
    /// ```text
    /// const files = [
    ///   { name: "document.pdf", size: 102400, lastModified: 0 },
    ///   { name: "image.png", size: 51200, lastModified: 0 }
    /// ];
    /// await session.initiateFileCopy(files);
    /// // Remote will call fileContentsRequestCallback to download
    /// ```
    fn initiate_file_copy(&self, _files: JsValue) -> Result<(), Self::Error> {
        unimplemented!("file transfer not supported by this session implementation")
    }
}

pub trait SessionTerminationInfo {
    fn reason(&self) -> String;
}

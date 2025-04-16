mod clipboard;
mod cursor;
mod error;
mod input;
mod session;

pub use clipboard::{ClipboardContent, ClipboardTransaction};
pub use cursor::CursorStyle;
pub use error::{IronError, IronErrorKind};
pub use input::{DeviceEvent, InputTransaction};
pub use session::{Session, SessionBuilder, SessionTerminationInfo};
use wasm_bindgen::prelude::wasm_bindgen;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct DesktopSize {
    pub width: u16,
    pub height: u16,
}

#[wasm_bindgen]
impl DesktopSize {
    pub fn init(width: u16, height: u16) -> Self {
        DesktopSize { width, height }
    }
}

pub fn iron_init(log_level: &str) {
    // When the `console_error_panic_hook` feature is enabled, we can call the
    // `set_panic_hook` function at least once during initialization, and then
    // we will get better error messages if our code ever panics.
    //
    // For more details see
    // https://github.com/rustwasm/console_error_panic_hook#readme
    #[cfg(feature = "panic_hook")]
    console_error_panic_hook::set_once();

    if let Ok(level) = log_level.parse::<tracing::Level>() {
        set_logger_once(level);
    }
}

fn set_logger_once(level: tracing::Level) {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::fmt::time::UtcTime;
    use tracing_subscriber::prelude::*;
    use tracing_web::MakeConsoleWriter;

    static INIT: std::sync::Once = std::sync::Once::new();

    INIT.call_once(|| {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_timer(UtcTime::rfc_3339()) // std::time is not available in browsers
            .with_writer(MakeConsoleWriter);

        let level_filter = LevelFilter::from_level(level);

        tracing_subscriber::registry().with(fmt_layer).with(level_filter).init();
    })
}

#[macro_export]
macro_rules! export_wasm {
    ($iron_init_fn:path, $ty_session:ty, $ty_session_builder:ty, $ty_session_termination_info:ty, $ty_device_event:ty, $ty_input_transaction:ty, $ty_clipboard_transaction:ty, $ty_clipboard_content:ty, $ty_error:ty) => {
        mod __wasm_ffi {
            use wasm_bindgen::prelude::*;
            use web_sys::{js_sys, HtmlCanvasElement};
            use $crate::{
                ClipboardContent as _, ClipboardTransaction as _, DeviceEvent as _, InputTransaction as _,
                IronError as _, Session as _, SessionBuilder as _, SessionTerminationInfo as _,
            };

            #[wasm_bindgen]
            pub fn iron_init(log_level: &str) {
                $iron_init_fn(log_level);
            }

            #[wasm_bindgen]
            pub struct DeviceEvent($ty_device_event);

            #[wasm_bindgen]
            impl DeviceEvent {
                pub fn mouse_button_pressed(button: u8) -> Self {
                    Self(<$ty_device_event>::mouse_button_pressed(button))
                }

                pub fn mouse_button_released(button: u8) -> Self {
                    Self(<$ty_device_event>::mouse_button_released(button))
                }

                pub fn mouse_move(x: u16, y: u16) -> Self {
                    Self(<$ty_device_event>::mouse_move(x, y))
                }

                pub fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
                    Self(<$ty_device_event>::wheel_rotations(vertical, rotation_units))
                }

                pub fn key_pressed(scancode: u16) -> Self {
                    Self(<$ty_device_event>::key_pressed(scancode))
                }

                pub fn key_released(scancode: u16) -> Self {
                    Self(<$ty_device_event>::key_released(scancode))
                }

                pub fn unicode_pressed(unicode: char) -> Self {
                    Self(<$ty_device_event>::unicode_pressed(unicode))
                }

                pub fn unicode_released(unicode: char) -> Self {
                    Self(<$ty_device_event>::unicode_released(unicode))
                }
            }

            #[wasm_bindgen]
            pub struct InputTransaction($ty_input_transaction);

            #[wasm_bindgen]
            impl InputTransaction {
                pub fn init() -> Self {
                    Self(<$ty_input_transaction>::init())
                }

                pub fn add_event(&mut self, event: DeviceEvent) {
                    self.0.add_event(event.0);
                }
            }

            #[wasm_bindgen]
            pub struct IronError($ty_error);

            #[wasm_bindgen]
            impl IronError {
                pub fn backtrace(&self) -> String {
                    self.0.backtrace()
                }

                pub fn kind(&self) -> $crate::IronErrorKind {
                    self.0.kind()
                }
            }

            #[wasm_bindgen]
            pub struct Session($ty_session);

            #[wasm_bindgen]
            impl Session {
                pub async fn run(&self) -> Result<SessionTerminationInfo, IronError> {
                    self.0.run().await.map(SessionTerminationInfo).map_err(IronError)
                }

                pub fn desktop_size(&self) -> $crate::DesktopSize {
                    self.0.desktop_size()
                }

                pub fn apply_inputs(&self, transaction: InputTransaction) -> Result<(), IronError> {
                    self.0.apply_inputs(transaction.0).map_err(IronError)
                }

                pub fn release_all_inputs(&self) -> Result<(), IronError> {
                    self.0.release_all_inputs().map_err(IronError)
                }

                pub fn synchronize_lock_keys(
                    &self,
                    scroll_lock: bool,
                    num_lock: bool,
                    caps_lock: bool,
                    kana_lock: bool,
                ) -> Result<(), IronError> {
                    self.0
                        .synchronize_lock_keys(scroll_lock, num_lock, caps_lock, kana_lock)
                        .map_err(IronError)
                }

                pub fn shutdown(&self) -> Result<(), IronError> {
                    self.0.shutdown().map_err(IronError)
                }

                pub async fn on_clipboard_paste(&self, content: ClipboardTransaction) -> Result<(), IronError> {
                    self.0.on_clipboard_paste(content.0).await.map_err(IronError)
                }

                pub fn resize(
                    &self,
                    width: u32,
                    height: u32,
                    scale_factor: Option<u32>,
                    physical_width: Option<u32>,
                    physical_height: Option<u32>,
                ) {
                    self.0
                        .resize(width, height, scale_factor, physical_width, physical_height);
                }

                pub fn supports_unicode_keyboard_shortcuts(&self) -> bool {
                    self.0.supports_unicode_keyboard_shortcuts()
                }

                pub fn extension_call(value: JsValue) -> Result<JsValue, IronError> {
                    <$ty_session>::extension_call(value).map_err(IronError)
                }
            }

            #[wasm_bindgen]
            pub struct SessionBuilder($ty_session_builder);

            #[wasm_bindgen]
            impl SessionBuilder {
                pub fn init() -> Self {
                    Self(<$ty_session_builder>::init())
                }

                pub fn username(&self, username: String) -> Self {
                    Self(self.0.username(username))
                }

                pub fn destination(&self, destination: String) -> Self {
                    Self(self.0.destination(destination))
                }

                pub fn server_domain(&self, server_domain: String) -> Self {
                    Self(self.0.server_domain(server_domain))
                }

                pub fn password(&self, password: String) -> Self {
                    Self(self.0.password(password))
                }

                pub fn proxy_address(&self, address: String) -> Self {
                    Self(self.0.proxy_address(address))
                }

                pub fn auth_token(&self, token: String) -> Self {
                    Self(self.0.auth_token(token))
                }

                pub fn desktop_size(&self, desktop_size: $crate::DesktopSize) -> Self {
                    Self(self.0.desktop_size(desktop_size))
                }

                pub fn render_canvas(&self, canvas: HtmlCanvasElement) -> Self {
                    Self(self.0.render_canvas(canvas))
                }

                pub fn set_cursor_style_callback(&self, callback: js_sys::Function) -> Self {
                    Self(self.0.set_cursor_style_callback(callback))
                }

                pub fn set_cursor_style_callback_context(&self, context: JsValue) -> Self {
                    Self(self.0.set_cursor_style_callback_context(context))
                }

                pub fn remote_clipboard_changed_callback(&self, callback: js_sys::Function) -> Self {
                    Self(self.0.remote_clipboard_changed_callback(callback))
                }

                pub fn remote_received_format_list_callback(&self, callback: js_sys::Function) -> Self {
                    Self(self.0.remote_received_format_list_callback(callback))
                }

                pub fn force_clipboard_update_callback(&self, callback: js_sys::Function) -> Self {
                    Self(self.0.force_clipboard_update_callback(callback))
                }

                pub fn extension(&self, value: JsValue) -> Self {
                    Self(self.0.extension(value))
                }

                pub async fn connect(&self) -> Result<Session, IronError> {
                    self.0.connect().await.map(Session).map_err(IronError)
                }
            }

            #[wasm_bindgen]
            pub struct SessionTerminationInfo($ty_session_termination_info);

            #[wasm_bindgen]
            impl SessionTerminationInfo {
                pub fn reason(&self) -> String {
                    self.0.reason()
                }
            }

            #[wasm_bindgen]
            pub struct ClipboardTransaction($ty_clipboard_transaction);

            #[wasm_bindgen]
            impl ClipboardTransaction {
                pub fn init() -> Self {
                    Self(<$ty_clipboard_transaction>::init())
                }

                pub fn add_content(&mut self, content: ClipboardContent) {
                    self.0.add_content(content.0);
                }

                pub fn is_empty(&self) -> bool {
                    self.0.is_empty()
                }

                pub fn content(&self) -> js_sys::Array {
                    self.0.js_contents()
                }
            }

            #[wasm_bindgen]
            pub struct ClipboardContent($ty_clipboard_content);

            #[wasm_bindgen]
            impl ClipboardContent {
                pub fn new_text(mime_type: &str, text: &str) -> Self {
                    Self(<$ty_clipboard_content>::new_text(mime_type, text))
                }

                pub fn new_binary(mime_type: &str, binary: &[u8]) -> Self {
                    Self(<$ty_clipboard_content>::new_binary(mime_type, binary))
                }

                pub fn mime_type(&self) -> String {
                    self.0.js_mime_type()
                }

                pub fn value(&self) -> JsValue {
                    self.0.js_value()
                }
            }
        }
    };
}

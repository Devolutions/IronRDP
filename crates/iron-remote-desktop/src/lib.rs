#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

mod clipboard;
mod cursor;
mod desktop_size;
mod error;
mod extension;
mod input;
mod session;

pub use clipboard::{ClipboardData, ClipboardItem};
pub use cursor::CursorStyle;
pub use desktop_size::DesktopSize;
pub use error::{IronError, IronErrorKind};
pub use extension::Extension;
pub use input::{DeviceEvent, InputTransaction};
pub use session::{Session, SessionBuilder, SessionTerminationInfo};

pub trait RemoteDesktopApi {
    type Session: Session;
    type SessionBuilder: SessionBuilder;
    type SessionTerminationInfo: SessionTerminationInfo;
    type DeviceEvent: DeviceEvent;
    type InputTransaction: InputTransaction;
    type ClipboardData: ClipboardData;
    type ClipboardItem: ClipboardItem;
    type Error: IronError;

    /// Called before the logger is set.
    fn pre_setup() {}

    /// Called after the logger is set.
    fn post_setup() {}
}

#[macro_export]
macro_rules! make_bridge {
    ($api:ty) => {
        use wasm_bindgen::prelude::*;
        use web_sys::{js_sys, HtmlCanvasElement};
        use $crate::{
            ClipboardData as _, ClipboardItem as _, DeviceEvent as _, InputTransaction as _, IronError as _,
            RemoteDesktopApi, Session as _, SessionBuilder as _, SessionTerminationInfo as _,
        };

        #[wasm_bindgen]
        pub fn setup(log_level: &str) {
            <$api as RemoteDesktopApi>::pre_setup();
            $crate::internal::setup(log_level);
            <$api as RemoteDesktopApi>::post_setup();
        }

        #[wasm_bindgen]
        pub struct DeviceEvent(<$api as RemoteDesktopApi>::DeviceEvent);

        impl From<<$api as RemoteDesktopApi>::DeviceEvent> for DeviceEvent {
            fn from(value: <$api as RemoteDesktopApi>::DeviceEvent) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl DeviceEvent {
            pub fn mouse_button_pressed(button: u8) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::mouse_button_pressed(
                    button,
                ))
            }

            pub fn mouse_button_released(button: u8) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::mouse_button_released(
                    button,
                ))
            }

            pub fn mouse_move(x: u16, y: u16) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::mouse_move(x, y))
            }

            pub fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::wheel_rotations(
                    vertical,
                    rotation_units,
                ))
            }

            pub fn key_pressed(scancode: u16) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::key_pressed(scancode))
            }

            pub fn key_released(scancode: u16) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::key_released(scancode))
            }

            pub fn unicode_pressed(unicode: char) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::unicode_pressed(
                    unicode,
                ))
            }

            pub fn unicode_released(unicode: char) -> Self {
                Self(<<$api as RemoteDesktopApi>::DeviceEvent>::unicode_released(
                    unicode,
                ))
            }
        }

        #[wasm_bindgen]
        pub struct InputTransaction(<$api as RemoteDesktopApi>::InputTransaction);

        impl From<<$api as RemoteDesktopApi>::InputTransaction> for InputTransaction {
            fn from(value: <$api as RemoteDesktopApi>::InputTransaction) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl InputTransaction {
            pub fn init() -> Self {
                Self(<<$api as RemoteDesktopApi>::InputTransaction>::init())
            }

            pub fn add_event(&mut self, event: DeviceEvent) {
                self.0.add_event(event.0);
            }
        }

        #[wasm_bindgen]
        pub struct IronError(<$api as RemoteDesktopApi>::Error);

        impl From<<$api as RemoteDesktopApi>::Error> for IronError {
            fn from(value: <$api as RemoteDesktopApi>::Error) -> Self {
                Self(value)
            }
        }

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
        pub struct Session(<$api as RemoteDesktopApi>::Session);

        impl From<<$api as RemoteDesktopApi>::Session> for Session {
            fn from(value: <$api as RemoteDesktopApi>::Session) -> Self {
                Self(value)
            }
        }

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

            pub async fn on_clipboard_paste(&self, content: ClipboardData) -> Result<(), IronError> {
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

            pub fn extension_call(ext: $crate::Extension) -> Result<JsValue, IronError> {
                <<$api as RemoteDesktopApi>::Session>::extension_call(ext).map_err(IronError)
            }
        }

        #[wasm_bindgen]
        pub struct SessionBuilder(<$api as RemoteDesktopApi>::SessionBuilder);

        impl From<<$api as RemoteDesktopApi>::SessionBuilder> for SessionBuilder {
            fn from(value: <$api as RemoteDesktopApi>::SessionBuilder) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl SessionBuilder {
            pub fn init() -> Self {
                Self(<<$api as RemoteDesktopApi>::SessionBuilder>::init())
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

            pub fn extension(&self, ext: $crate::Extension) -> Self {
                Self(self.0.extension(ext))
            }

            pub async fn connect(&self) -> Result<Session, IronError> {
                self.0.connect().await.map(Session).map_err(IronError)
            }
        }

        #[wasm_bindgen]
        pub struct SessionTerminationInfo(<$api as RemoteDesktopApi>::SessionTerminationInfo);

        impl From<<$api as RemoteDesktopApi>::SessionTerminationInfo> for SessionTerminationInfo {
            fn from(value: <$api as RemoteDesktopApi>::SessionTerminationInfo) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl SessionTerminationInfo {
            pub fn reason(&self) -> String {
                self.0.reason()
            }
        }

        #[wasm_bindgen]
        pub struct ClipboardData(<$api as RemoteDesktopApi>::ClipboardData);

        impl From<<$api as RemoteDesktopApi>::ClipboardData> for ClipboardData {
            fn from(value: <$api as RemoteDesktopApi>::ClipboardData) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl ClipboardData {
            pub fn init() -> Self {
                Self(<<$api as RemoteDesktopApi>::ClipboardData>::init())
            }

            pub fn add_text(&mut self, mime_type: &str, text: &str) {
                self.0.add_text(mime_type, text);
            }

            pub fn add_binary(&mut self, mime_type: &str, binary: &[u8]) {
                self.0.add_binary(mime_type, binary);
            }

            pub fn items(&self) -> Vec<ClipboardItem> {
                self.0.items().into_iter().cloned().map(ClipboardItem).collect()
            }

            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        #[wasm_bindgen]
        pub struct ClipboardItem(<$api as RemoteDesktopApi>::ClipboardItem);

        impl From<<$api as RemoteDesktopApi>::ClipboardItem> for ClipboardItem {
            fn from(value: <$api as RemoteDesktopApi>::ClipboardItem) -> Self {
                Self(value)
            }
        }

        #[wasm_bindgen]
        impl ClipboardItem {
            pub fn mime_type(&self) -> String {
                self.0.mime_type().to_owned()
            }

            pub fn value(&self) -> JsValue {
                self.0.value().into()
            }
        }
    };
}

#[doc(hidden)]
pub mod internal {
    #[doc(hidden)]
    pub fn setup(log_level: &str) {
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
}

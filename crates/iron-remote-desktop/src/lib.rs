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
        use $crate::{
            ClipboardData as _, ClipboardItem as _, DeviceEvent as _, InputTransaction as _, IronError as _,
            Session as _, SessionBuilder as _, SessionTerminationInfo as _,
        };

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct Session(<$api as $crate::RemoteDesktopApi>::Session);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct SessionBuilder(<$api as $crate::RemoteDesktopApi>::SessionBuilder);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct SessionTerminationInfo(<$api as $crate::RemoteDesktopApi>::SessionTerminationInfo);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct DeviceEvent(<$api as $crate::RemoteDesktopApi>::DeviceEvent);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct InputTransaction(<$api as $crate::RemoteDesktopApi>::InputTransaction);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct ClipboardData(<$api as $crate::RemoteDesktopApi>::ClipboardData);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct ClipboardItem(<$api as $crate::RemoteDesktopApi>::ClipboardItem);

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        pub struct IronError(<$api as $crate::RemoteDesktopApi>::Error);

        impl From<<$api as $crate::RemoteDesktopApi>::Session> for Session {
            fn from(value: <$api as $crate::RemoteDesktopApi>::Session) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::SessionBuilder> for SessionBuilder {
            fn from(value: <$api as $crate::RemoteDesktopApi>::SessionBuilder) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::SessionTerminationInfo> for SessionTerminationInfo {
            fn from(value: <$api as $crate::RemoteDesktopApi>::SessionTerminationInfo) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::DeviceEvent> for DeviceEvent {
            fn from(value: <$api as $crate::RemoteDesktopApi>::DeviceEvent) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::InputTransaction> for InputTransaction {
            fn from(value: <$api as $crate::RemoteDesktopApi>::InputTransaction) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::ClipboardData> for ClipboardData {
            fn from(value: <$api as $crate::RemoteDesktopApi>::ClipboardData) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::ClipboardItem> for ClipboardItem {
            fn from(value: <$api as $crate::RemoteDesktopApi>::ClipboardItem) -> Self {
                Self(value)
            }
        }

        impl From<<$api as $crate::RemoteDesktopApi>::Error> for IronError {
            fn from(value: <$api as $crate::RemoteDesktopApi>::Error) -> Self {
                Self(value)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        pub fn setup(log_level: &str) {
            <$api as $crate::RemoteDesktopApi>::pre_setup();
            $crate::internal::setup(log_level);
            <$api as $crate::RemoteDesktopApi>::post_setup();
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl Session {
            pub async fn run(&self) -> Result<SessionTerminationInfo, IronError> {
                self.0.run().await.map(SessionTerminationInfo).map_err(IronError)
            }

            #[wasm_bindgen(js_name = desktopSize)]
            pub fn desktop_size(&self) -> $crate::DesktopSize {
                self.0.desktop_size()
            }

            #[wasm_bindgen(js_name = applyInputs)]
            pub fn apply_inputs(&self, transaction: InputTransaction) -> Result<(), IronError> {
                self.0.apply_inputs(transaction.0).map_err(IronError)
            }

            #[wasm_bindgen(js_name = releaseAllInputs)]
            pub fn release_all_inputs(&self) -> Result<(), IronError> {
                self.0.release_all_inputs().map_err(IronError)
            }

            #[wasm_bindgen(js_name = synchronizeLockKeys)]
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

            #[wasm_bindgen(js_name = onClipboardPaste)]
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

            #[wasm_bindgen(js_name = supportsUnicodeKeyboardShortcuts)]
            pub fn supports_unicode_keyboard_shortcuts(&self) -> bool {
                self.0.supports_unicode_keyboard_shortcuts()
            }

            #[wasm_bindgen(js_name = extensionCall)]
            pub fn extension_call(
                ext: $crate::Extension,
            ) -> Result<$crate::internal::wasm_bindgen::JsValue, IronError> {
                <<$api as $crate::RemoteDesktopApi>::Session>::extension_call(ext).map_err(IronError)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl SessionBuilder {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::SessionBuilder>::create())
            }

            pub fn username(&self, username: String) -> Self {
                Self(self.0.username(username))
            }

            pub fn destination(&self, destination: String) -> Self {
                Self(self.0.destination(destination))
            }

            #[wasm_bindgen(js_name = serverDomain)]
            pub fn server_domain(&self, server_domain: String) -> Self {
                Self(self.0.server_domain(server_domain))
            }

            pub fn password(&self, password: String) -> Self {
                Self(self.0.password(password))
            }

            #[wasm_bindgen(js_name = proxyAddress)]
            pub fn proxy_address(&self, address: String) -> Self {
                Self(self.0.proxy_address(address))
            }

            #[wasm_bindgen(js_name = authToken)]
            pub fn auth_token(&self, token: String) -> Self {
                Self(self.0.auth_token(token))
            }

            #[wasm_bindgen(js_name = desktopSize)]
            pub fn desktop_size(&self, desktop_size: $crate::DesktopSize) -> Self {
                Self(self.0.desktop_size(desktop_size))
            }

            #[wasm_bindgen(js_name = renderCanvas)]
            pub fn render_canvas(&self, canvas: $crate::internal::web_sys::HtmlCanvasElement) -> Self {
                Self(self.0.render_canvas(canvas))
            }

            #[wasm_bindgen(js_name = setCursorStyleCallback)]
            pub fn set_cursor_style_callback(&self, callback: $crate::internal::web_sys::js_sys::Function) -> Self {
                Self(self.0.set_cursor_style_callback(callback))
            }

            #[wasm_bindgen(js_name = setCursorStyleCallbackContext)]
            pub fn set_cursor_style_callback_context(&self, context: $crate::internal::wasm_bindgen::JsValue) -> Self {
                Self(self.0.set_cursor_style_callback_context(context))
            }

            #[wasm_bindgen(js_name = remoteClipboardChangedCallback)]
            pub fn remote_clipboard_changed_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self(self.0.remote_clipboard_changed_callback(callback))
            }

            #[wasm_bindgen(js_name = remoteReceivedFormatListCallback)]
            pub fn remote_received_format_list_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self(self.0.remote_received_format_list_callback(callback))
            }

            #[wasm_bindgen(js_name = forceClipboardUpdateCallback)]
            pub fn force_clipboard_update_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self(self.0.force_clipboard_update_callback(callback))
            }

            pub fn extension(&self, ext: $crate::Extension) -> Self {
                Self(self.0.extension(ext))
            }

            pub async fn connect(&self) -> Result<Session, IronError> {
                self.0.connect().await.map(Session).map_err(IronError)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl SessionTerminationInfo {
            pub fn reason(&self) -> String {
                self.0.reason()
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl DeviceEvent {
            #[wasm_bindgen(js_name = mouseButtonPressed)]
            pub fn mouse_button_pressed(button: u8) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::mouse_button_pressed(button))
            }

            #[wasm_bindgen(js_name = mouseButtonReleased)]
            pub fn mouse_button_released(button: u8) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::mouse_button_released(button))
            }

            #[wasm_bindgen(js_name = mouseMove)]
            pub fn mouse_move(x: u16, y: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::mouse_move(
                    x, y,
                ))
            }

            #[wasm_bindgen(js_name = wheelRotations)]
            pub fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::wheel_rotations(vertical, rotation_units))
            }

            #[wasm_bindgen(js_name = keyPressed)]
            pub fn key_pressed(scancode: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::key_pressed(
                    scancode,
                ))
            }

            #[wasm_bindgen(js_name = keyReleased)]
            pub fn key_released(scancode: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::key_released(
                    scancode,
                ))
            }

            #[wasm_bindgen(js_name = unicodePressed)]
            pub fn unicode_pressed(unicode: char) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::unicode_pressed(unicode))
            }

            #[wasm_bindgen(js_name = unicodeReleased)]
            pub fn unicode_released(unicode: char) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent>::unicode_released(unicode))
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl InputTransaction {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::InputTransaction>::create())
            }

            #[wasm_bindgen(js_name = addEvent)]
            pub fn add_event(&mut self, event: DeviceEvent) {
                self.0.add_event(event.0);
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl ClipboardData {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::ClipboardData>::create())
            }

            #[wasm_bindgen(js_name = addText)]
            pub fn add_text(&mut self, mime_type: &str, text: &str) {
                self.0.add_text(mime_type, text);
            }

            #[wasm_bindgen(js_name = addBinary)]
            pub fn add_binary(&mut self, mime_type: &str, binary: &[u8]) {
                self.0.add_binary(mime_type, binary);
            }

            pub fn items(&self) -> Vec<ClipboardItem> {
                self.0.items().into_iter().cloned().map(ClipboardItem).collect()
            }

            #[wasm_bindgen(js_name = isEmpty)]
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl ClipboardItem {
            #[wasm_bindgen(js_name = mimeType)]
            pub fn mime_type(&self) -> String {
                self.0.mime_type().to_owned()
            }

            pub fn value(&self) -> $crate::internal::wasm_bindgen::JsValue {
                self.0.value().into()
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl IronError {
            pub fn backtrace(&self) -> String {
                self.0.backtrace()
            }

            pub fn kind(&self) -> $crate::IronErrorKind {
                self.0.kind()
            }
        }
    };
}

#[doc(hidden)]
pub mod internal {
    #[doc(hidden)]
    pub use web_sys;

    #[doc(hidden)]
    pub use wasm_bindgen;

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

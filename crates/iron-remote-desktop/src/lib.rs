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
                $crate::Session::run(&self.0)
                    .await
                    .map(SessionTerminationInfo)
                    .map_err(IronError)
            }

            #[wasm_bindgen(js_name = desktopSize)]
            pub fn desktop_size(&self) -> $crate::DesktopSize {
                $crate::Session::desktop_size(&self.0)
            }

            #[wasm_bindgen(js_name = applyInputs)]
            pub fn apply_inputs(&self, transaction: InputTransaction) -> Result<(), IronError> {
                $crate::Session::apply_inputs(&self.0, transaction.0).map_err(IronError)
            }

            #[wasm_bindgen(js_name = releaseAllInputs)]
            pub fn release_all_inputs(&self) -> Result<(), IronError> {
                $crate::Session::release_all_inputs(&self.0).map_err(IronError)
            }

            #[wasm_bindgen(js_name = synchronizeLockKeys)]
            pub fn synchronize_lock_keys(
                &self,
                scroll_lock: bool,
                num_lock: bool,
                caps_lock: bool,
                kana_lock: bool,
            ) -> Result<(), IronError> {
                $crate::Session::synchronize_lock_keys(&self.0, scroll_lock, num_lock, caps_lock, kana_lock)
                    .map_err(IronError)
            }

            pub fn shutdown(&self) -> Result<(), IronError> {
                $crate::Session::shutdown(&self.0).map_err(IronError)
            }

            #[wasm_bindgen(js_name = onClipboardPaste)]
            pub async fn on_clipboard_paste(&self, content: ClipboardData) -> Result<(), IronError> {
                $crate::Session::on_clipboard_paste(&self.0, content.0)
                    .await
                    .map_err(IronError)
            }

            pub fn resize(
                &self,
                width: u32,
                height: u32,
                scale_factor: Option<u32>,
                physical_width: Option<u32>,
                physical_height: Option<u32>,
            ) {
                $crate::Session::resize(
                    &self.0,
                    width,
                    height,
                    scale_factor,
                    physical_width,
                    physical_height,
                );
            }

            #[wasm_bindgen(js_name = supportsUnicodeKeyboardShortcuts)]
            pub fn supports_unicode_keyboard_shortcuts(&self) -> bool {
                $crate::Session::supports_unicode_keyboard_shortcuts(&self.0)
            }

            #[wasm_bindgen(js_name = invokeExtension)]
            pub fn invoke_extension(
                &self,
                ext: $crate::Extension,
            ) -> Result<$crate::internal::wasm_bindgen::JsValue, IronError> {
                <<$api as $crate::RemoteDesktopApi>::Session as $crate::Session>::invoke_extension(&self.0, ext)
                    .map_err(IronError)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl SessionBuilder {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::SessionBuilder as $crate::SessionBuilder>::create())
            }

            pub fn username(&self, username: String) -> Self {
                Self($crate::SessionBuilder::username(&self.0, username))
            }

            pub fn destination(&self, destination: String) -> Self {
                Self($crate::SessionBuilder::destination(&self.0, destination))
            }

            #[wasm_bindgen(js_name = serverDomain)]
            pub fn server_domain(&self, server_domain: String) -> Self {
                Self($crate::SessionBuilder::server_domain(&self.0, server_domain))
            }

            pub fn password(&self, password: String) -> Self {
                Self($crate::SessionBuilder::password(&self.0, password))
            }

            #[wasm_bindgen(js_name = proxyAddress)]
            pub fn proxy_address(&self, address: String) -> Self {
                Self($crate::SessionBuilder::proxy_address(&self.0, address))
            }

            #[wasm_bindgen(js_name = authToken)]
            pub fn auth_token(&self, token: String) -> Self {
                Self($crate::SessionBuilder::auth_token(&self.0, token))
            }

            #[wasm_bindgen(js_name = desktopSize)]
            pub fn desktop_size(&self, desktop_size: $crate::DesktopSize) -> Self {
                Self($crate::SessionBuilder::desktop_size(&self.0, desktop_size))
            }

            #[wasm_bindgen(js_name = renderCanvas)]
            pub fn render_canvas(&self, canvas: $crate::internal::web_sys::HtmlCanvasElement) -> Self {
                Self($crate::SessionBuilder::render_canvas(&self.0, canvas))
            }

            #[wasm_bindgen(js_name = setCursorStyleCallback)]
            pub fn set_cursor_style_callback(&self, callback: $crate::internal::web_sys::js_sys::Function) -> Self {
                Self($crate::SessionBuilder::set_cursor_style_callback(
                    &self.0, callback,
                ))
            }

            #[wasm_bindgen(js_name = setCursorStyleCallbackContext)]
            pub fn set_cursor_style_callback_context(&self, context: $crate::internal::wasm_bindgen::JsValue) -> Self {
                Self($crate::SessionBuilder::set_cursor_style_callback_context(
                    &self.0, context,
                ))
            }

            #[wasm_bindgen(js_name = remoteClipboardChangedCallback)]
            pub fn remote_clipboard_changed_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self($crate::SessionBuilder::remote_clipboard_changed_callback(
                    &self.0, callback,
                ))
            }

            #[wasm_bindgen(js_name = remoteReceivedFormatListCallback)]
            pub fn remote_received_format_list_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self($crate::SessionBuilder::remote_received_format_list_callback(
                    &self.0, callback,
                ))
            }

            #[wasm_bindgen(js_name = forceClipboardUpdateCallback)]
            pub fn force_clipboard_update_callback(
                &self,
                callback: $crate::internal::web_sys::js_sys::Function,
            ) -> Self {
                Self($crate::SessionBuilder::force_clipboard_update_callback(
                    &self.0, callback,
                ))
            }

            pub fn extension(&self, ext: $crate::Extension) -> Self {
                Self($crate::SessionBuilder::extension(&self.0, ext))
            }

            pub async fn connect(&self) -> Result<Session, IronError> {
                $crate::SessionBuilder::connect(&self.0)
                    .await
                    .map(Session)
                    .map_err(IronError)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl SessionTerminationInfo {
            pub fn reason(&self) -> String {
                $crate::SessionTerminationInfo::reason(&self.0)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl DeviceEvent {
            #[wasm_bindgen(js_name = mouseButtonPressed)]
            pub fn mouse_button_pressed(button: u8) -> Self {
                Self(
                    <<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::mouse_button_pressed(
                        button,
                    ),
                )
            }

            #[wasm_bindgen(js_name = mouseButtonReleased)]
            pub fn mouse_button_released(button: u8) -> Self {
                Self(
                    <<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::mouse_button_released(
                        button,
                    ),
                )
            }

            #[wasm_bindgen(js_name = mouseMove)]
            pub fn mouse_move(x: u16, y: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::mouse_move(x, y))
            }

            #[wasm_bindgen(js_name = wheelRotations)]
            pub fn wheel_rotations(vertical: bool, rotation_units: i16) -> Self {
                Self(
                    <<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::wheel_rotations(
                        vertical,
                        rotation_units,
                    ),
                )
            }

            #[wasm_bindgen(js_name = keyPressed)]
            pub fn key_pressed(scancode: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::key_pressed(scancode))
            }

            #[wasm_bindgen(js_name = keyReleased)]
            pub fn key_released(scancode: u16) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::key_released(scancode))
            }

            #[wasm_bindgen(js_name = unicodePressed)]
            pub fn unicode_pressed(unicode: char) -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::unicode_pressed(unicode))
            }

            #[wasm_bindgen(js_name = unicodeReleased)]
            pub fn unicode_released(unicode: char) -> Self {
                Self(
                    <<$api as $crate::RemoteDesktopApi>::DeviceEvent as $crate::DeviceEvent>::unicode_released(unicode),
                )
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl InputTransaction {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::InputTransaction as $crate::InputTransaction>::create())
            }

            #[wasm_bindgen(js_name = addEvent)]
            pub fn add_event(&mut self, event: DeviceEvent) {
                $crate::InputTransaction::add_event(&mut self.0, event.0);
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl ClipboardData {
            #[wasm_bindgen(constructor)]
            pub fn create() -> Self {
                Self(<<$api as $crate::RemoteDesktopApi>::ClipboardData as $crate::ClipboardData>::create())
            }

            #[wasm_bindgen(js_name = addText)]
            pub fn add_text(&mut self, mime_type: &str, text: &str) {
                $crate::ClipboardData::add_text(&mut self.0, mime_type, text);
            }

            #[wasm_bindgen(js_name = addBinary)]
            pub fn add_binary(&mut self, mime_type: &str, binary: &[u8]) {
                $crate::ClipboardData::add_binary(&mut self.0, mime_type, binary);
            }

            pub fn items(&self) -> Vec<ClipboardItem> {
                $crate::ClipboardData::items(&self.0)
                    .into_iter()
                    .cloned()
                    .map(ClipboardItem)
                    .collect()
            }

            #[wasm_bindgen(js_name = isEmpty)]
            pub fn is_empty(&self) -> bool {
                $crate::ClipboardData::is_empty(&self.0)
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl ClipboardItem {
            #[wasm_bindgen(js_name = mimeType)]
            pub fn mime_type(&self) -> String {
                $crate::ClipboardItem::mime_type(&self.0).to_owned()
            }

            pub fn value(&self) -> $crate::internal::wasm_bindgen::JsValue {
                $crate::ClipboardItem::value(&self.0).into()
            }
        }

        #[$crate::internal::wasm_bindgen::prelude::wasm_bindgen]
        #[doc(hidden)]
        impl IronError {
            pub fn backtrace(&self) -> String {
                $crate::IronError::backtrace(&self.0)
            }

            pub fn kind(&self) -> $crate::IronErrorKind {
                $crate::IronError::kind(&self.0)
            }
        }
    };
}

#[doc(hidden)]
pub mod internal {
    #[doc(hidden)]
    pub use wasm_bindgen;
    #[doc(hidden)]
    pub use web_sys;

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

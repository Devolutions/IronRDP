use wasm_bindgen::JsValue;
use web_sys::{js_sys, HtmlCanvasElement};

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
    fn extension(&self, ext: Extension) -> Self;

    #[expect(async_fn_in_trait)]
    async fn connect(&self) -> Result<Self::Session, Self::Error>;
}

pub trait Session {
    type SessionTerminationInfo: SessionTerminationInfo;
    type InputTransaction: InputTransaction;
    type ClipboardData: ClipboardData;
    type Error: IronError;

    fn run(&self) -> impl core::future::Future<Output = Result<Self::SessionTerminationInfo, Self::Error>>;

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

    fn on_clipboard_paste(
        &self,
        content: &Self::ClipboardData,
    ) -> impl core::future::Future<Output = Result<(), Self::Error>>;

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
}

pub trait SessionTerminationInfo {
    fn reason(&self) -> String;
}

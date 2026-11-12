use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::download::runtime::Downloads;
use crate::remote_devices;
use anyhow::Context as _;
use cef::*;

#[derive(Debug)]
pub enum Event {
    Paint {
        tab: i32,
        popup: bool,
        generation: u64,
        width: u16,
        height: u16,
        pixels: Vec<u8>,
    },
    Popup {
        tab: i32,
        rect: Rect,
    },
    HidePopup(i32),
    Url(i32, String),
    Title(i32, String),
    Loading(i32, bool),
    Error(i32, String),
    Open(String),
    Closed(i32),
}

#[derive(Clone)]
struct Bridge {
    tab: i32,
    size: Rc<Cell<(u16, u16)>>,
    generation: Rc<Cell<u64>>,
    events: Rc<RefCell<VecDeque<Event>>>,
    fatal: Rc<RefCell<Option<String>>>,
    downloads: Downloads,
    download_callbacks: Rc<RefCell<BTreeMap<u32, DownloadItemCallback>>>,
    browser_downloads: Rc<RefCell<BTreeSet<u32>>>,
    closing: Rc<Cell<bool>>,
    media_capture: bool,
}

impl Bridge {
    fn emit(&self, mut event: Event) {
        if self.closing.get() && !matches!(&event, Event::Closed(_)) {
            return;
        }
        match &mut event {
            Event::Url(_, text) | Event::Title(_, text) | Event::Error(_, text) | Event::Open(text) => {
                if 8192 < text.len() {
                    *text = text.chars().take(2048).collect();
                }
            }
            _ => {}
        }
        let mut events = self.events.borrow_mut();
        if let Event::Paint { tab, popup, .. } = &event {
            events.retain(|old| !matches!(old, Event::Paint { tab: id, popup: p, .. } if id == tab && p == popup));
        }
        if events.len() < 128 {
            events.push_back(event);
        } else {
            *self.fatal.borrow_mut() = Some("CEF callback queue exhausted".to_owned());
        }
    }
}

wrap_render_handler! {
    struct PaintHandler { bridge: Bridge }
    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let (width, height) = self.bridge.size.get();
                *rect = Rect { x: 0, y: 0, width: i32::from(width), height: i32::from(height) };
            }
        }
        fn screen_info(&self, _browser: Option<&mut Browser>, info: Option<&mut ScreenInfo>) -> i32 {
            if let Some(info) = info {
                let (width, height) = self.bridge.size.get();
                info.device_scale_factor = 1.0;
                info.depth = 32;
                info.depth_per_component = 8;
                info.rect = Rect { x: 0, y: 0, width: i32::from(width), height: i32::from(height) };
                info.available_rect = info.rect.clone();
                1
            } else {
                0
            }
        }
        fn on_paint(&self, _browser: Option<&mut Browser>, kind: PaintElementType,
                    _dirty: Option<&[Rect]>, buffer: *const u8, width: i32, height: i32) {
            let Ok(width) = u16::try_from(width) else {
                *self.bridge.fatal.borrow_mut() = Some("invalid CEF paint width".to_owned());
                return;
            };
            let Ok(height) = u16::try_from(height) else {
                *self.bridge.fatal.borrow_mut() = Some("invalid CEF paint height".to_owned());
                return;
            };
            if buffer.is_null() || width == 0 || height == 0 || !(width <= 3840 && height <= 2160) {
                *self.bridge.fatal.borrow_mut() = Some("invalid CEF paint dimensions or buffer".to_owned());
                return;
            }
            let len = usize::from(width) * usize::from(height) * 4;
            // SAFETY: CEF owns width * height BGRA pixels for the duration of OnPaint.
            let pixels = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            self.bridge.emit(Event::Paint { tab: self.bridge.tab, popup: kind == PaintElementType::POPUP,
                generation: self.bridge.generation.get(), width, height, pixels });
        }
        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            if let Some(rect) = rect {
                self.bridge.emit(Event::Popup { tab: self.bridge.tab, rect: rect.clone() });
            }
        }
        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: i32) {
            if show == 0 {
                self.bridge.emit(Event::HidePopup(self.bridge.tab));
            }
        }
    }
}

wrap_display_handler! {
    struct DisplayHandlerImpl { bridge: Bridge }
    impl DisplayHandler {
        fn on_address_change(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            if frame.is_some_and(|frame| frame.is_main() != 0) {
                self.bridge.emit(Event::Url(self.bridge.tab, url.map(ToString::to_string).unwrap_or_default()));
            }
        }
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            self.bridge.emit(Event::Title(self.bridge.tab, title.map(ToString::to_string).unwrap_or_default()));
        }
    }
}

wrap_load_handler! {
    struct LoadHandlerImpl { bridge: Bridge }
    impl LoadHandler {
        fn on_loading_state_change(&self, _browser: Option<&mut Browser>, loading: i32, _back: i32, _forward: i32) {
            self.bridge.emit(Event::Loading(self.bridge.tab, loading != 0));
        }
        fn on_load_error(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                         code: Errorcode, text: Option<&CefString>, _url: Option<&CefString>) {
            if code != Errorcode::ABORTED && frame.is_some_and(|frame| frame.is_main() != 0) {
                self.bridge.emit(Event::Error(self.bridge.tab, text.map(ToString::to_string).unwrap_or_else(|| format!("{code:?}"))));
            }
        }
    }
}

wrap_life_span_handler! {
    struct LifeHandler { bridge: Bridge }
    impl LifeSpanHandler {
        fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                           _popup_id: i32, target_url: Option<&CefString>, _target_frame_name: Option<&CefString>,
                           _disposition: WindowOpenDisposition, _gesture: i32, _features: Option<&PopupFeatures>,
                           _window: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>,
                           _settings: Option<&mut BrowserSettings>, _extra: Option<&mut Option<DictionaryValue>>,
                           _no_javascript_access: Option<&mut i32>) -> i32 {
            if let Some(url) = target_url {
                self.bridge.emit(Event::Open(url.to_string()));
            }
            1
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            self.bridge.emit(Event::Closed(self.bridge.tab));
        }
    }
}

wrap_download_handler! {
    struct DownloadHandlerImpl { bridge: Bridge }
    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _method: Option<&CefString>) -> i32 {
            if self.bridge.closing.get() {
                return 0;
            }
            match self.bridge.downloads.available() {
                Ok(()) => 1,
                Err(error) => {
                    self.bridge.downloads.notice(&error.to_string());
                    self.bridge.emit(Event::Error(self.bridge.tab, error.to_string()));
                    0
                }
            }
        }
        fn on_before_download(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
                              name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> i32 {
            if self.bridge.closing.get() {
                return 1;
            }
            if let (Some(item), Some(name), Some(callback)) = (item, name, callback) {
                match self.bridge.downloads.begin(item.id(), &name.to_string()) {
                    Ok(path) => {
                        self.bridge.browser_downloads.borrow_mut().insert(item.id());
                        callback.cont(Some(&path.to_string_lossy().as_ref().into()), 0);
                    }
                    Err(error) => {
                        self.bridge.downloads.notice(&error.to_string());
                        self.bridge.emit(Event::Error(self.bridge.tab, error.to_string()));
                    }
                }
            }
            1
        }
        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>,
                               callback: Option<&mut DownloadItemCallback>) {
            if let Some(item) = item {
                let id = item.id();
                let complete = item.is_complete() != 0;
                let cancelled = item.is_canceled() != 0 || item.is_interrupted() != 0;
                self.bridge.downloads.progress(id, item.received_bytes().max(0) as u64, complete, cancelled);
                if complete || cancelled {
                    self.bridge.download_callbacks.borrow_mut().remove(&id);
                    self.bridge.browser_downloads.borrow_mut().remove(&id);
                } else if let Some(callback) = callback {
                    if self.bridge.downloads.should_cancel(id) {
                        callback.cancel();
                    } else if self.bridge.downloads.tracked(id) {
                        self.bridge.download_callbacks.borrow_mut().insert(id, callback.clone());
                    }
                }
            }
        }
    }
}

wrap_dialog_handler! {
    struct DialogPolicy { bridge: Bridge }
    impl DialogHandler {
        fn on_file_dialog(&self, _browser: Option<&mut Browser>, _mode: FileDialogMode,
                          _title: Option<&CefString>, _path: Option<&CefString>,
                          _filters: Option<&mut CefStringList>, _extensions: Option<&mut CefStringList>,
                          _descriptions: Option<&mut CefStringList>, callback: Option<&mut FileDialogCallback>) -> i32 {
            if let Some(callback) = callback { callback.cancel(); }
            self.bridge.emit(Event::Error(self.bridge.tab, "native file dialogs and uploads are disabled".to_owned()));
            1
        }
    }
}

wrap_jsdialog_handler! {
    struct JsDialogPolicy { bridge: Bridge }
    impl JsdialogHandler {
        fn on_before_unload_dialog(&self, _browser: Option<&mut Browser>, _message: Option<&CefString>,
                                   _reload: i32, callback: Option<&mut JsdialogCallback>) -> i32 {
            if !self.bridge.closing.get() {
                return 0;
            }
            if let Some(callback) = callback {
                callback.cont(1, None);
            } else {
                *self.bridge.fatal.borrow_mut() = Some("CEF before-unload callback is missing".to_owned());
            }
            1
        }
    }
}

wrap_request_handler! {
    struct RequestHandlerImpl { bridge: Bridge }
    impl RequestHandler {
        fn on_before_browse(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                            request: Option<&mut Request>, _gesture: i32, _redirect: i32) -> i32 {
            if let Some(request) = request {
                let url = CefString::from(&request.url()).to_string();
                if !allowed_url(&url) {
                    self.bridge.emit(Event::Error(self.bridge.tab, "only HTTP, HTTPS, and about:blank navigation is supported".to_owned()));
                    return 1;
                }
            }
            0
        }
        fn on_render_process_terminated(&self, _browser: Option<&mut Browser>, status: TerminationStatus,
                                        code: i32, _text: Option<&CefString>) {
            *self.bridge.fatal.borrow_mut() = Some(format!("CEF renderer terminated: {status:?} ({code})"));
        }
        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                    _request: Option<&mut Request>, _navigation: i32, _download: i32,
                                    _initiator: Option<&CefString>, _disable: Option<&mut i32>) -> Option<ResourceRequestHandler> {
            Some(ResourcePolicy::new())
        }
    }
}

wrap_resource_request_handler! {
    struct ResourcePolicy;
    impl ResourceRequestHandler {
        fn on_before_resource_load(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                   _request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            ReturnValue::CONTINUE
        }
        fn on_protocol_execution(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                 _request: Option<&mut Request>, allow: Option<&mut i32>) {
            if let Some(allow) = allow { *allow = 0; }
        }
    }
}

wrap_context_menu_handler! {
    struct ContextMenuPolicy;
    impl ContextMenuHandler {
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                  _params: Option<&mut ContextMenuParams>, menu: Option<&mut MenuModel>) {
            if let Some(menu) = menu { menu.clear(); }
        }
    }
}

// UNTESTED: without a device feeder, camera and microphone requests are denied.
// The device suite always runs the simulator, so only desktop-capture denial is checked.
// Recheck with a plain bundle when changing this policy or the media switches.
wrap_permission_handler! {
    struct MediaPolicy { allow: bool }
    impl PermissionHandler {
        fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                              _origin: Option<&CefString>, requested: u32,
                                              callback: Option<&mut MediaAccessCallback>) -> std::ffi::c_int {
            use cef::sys::cef_media_access_permission_types_t as Types;
            const DEVICES: u32 =
                Types::CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE as u32 | Types::CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE as u32;
            // Screen capture would expose the host display, so only the remote camera and microphone qualify.
            let allowed = if self.allow && requested & !DEVICES == 0 { requested } else { 0 };
            if let Some(callback) = callback { callback.cont(allowed); }
            1
        }
    }
}

wrap_client! {
    struct BrowserClient { bridge: Bridge }
    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> { Some(PaintHandler::new(self.bridge.clone())) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(DisplayHandlerImpl::new(self.bridge.clone())) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(LoadHandlerImpl::new(self.bridge.clone())) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(LifeHandler::new(self.bridge.clone())) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(DownloadHandlerImpl::new(self.bridge.clone())) }
        fn dialog_handler(&self) -> Option<DialogHandler> { Some(DialogPolicy::new(self.bridge.clone())) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(JsDialogPolicy::new(self.bridge.clone())) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(RequestHandlerImpl::new(self.bridge.clone())) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(ContextMenuPolicy::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> { Some(MediaPolicy::new(self.bridge.media_capture)) }
    }
}

wrap_browser_process_handler! {
    struct ProcessHandler { ready: Rc<Cell<bool>> }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) { self.ready.set(true); }
    }
}

wrap_app! {
    struct BrowserApp { ready: Rc<Cell<bool>> }
    impl App {
        fn on_before_command_line_processing(&self, _kind: Option<&CefString>, command: Option<&mut CommandLine>) {
            if let Some(command) = command {
                command.append_switch(Some(&"no-startup-window".into()));
                command.append_switch(Some(&"disable-gpu".into()));
                command.append_switch(Some(&"disable-gpu-compositing".into()));
                #[cfg(unix)]
                command.append_switch(Some(&"disable-chrome-shutdown-signal-handlers".into()));
                #[cfg(target_os = "macos")]
                command.append_switch(Some(&"use-mock-keychain".into()));
                #[cfg(target_os = "macos")]
                let disabled = format!("AperitifHelpers,GpuPersistentCache,{}", remote_devices::DISABLE_FEATURES);
                #[cfg(not(target_os = "macos"))]
                let disabled = remote_devices::DISABLE_FEATURES.to_owned();
                command.append_switch_with_value(Some(&"disable-features".into()), Some(&disabled.as_str().into()));
                command.append_switch_with_value(
                    Some(&"enable-features".into()),
                    Some(&remote_devices::ENABLE_FEATURES.into()),
                );
                // A patched runtime keeps page copies in process memory; stock CEF ignores the switch.
                command.append_switch(Some(&crate::private_clipboard::SWITCH.into()));
                // Pages only reach client-supplied capture devices and authenticators, never host ones.
                for switch in remote_devices::MEDIA_SWITCHES {
                    command.append_switch(Some(&switch.into()));
                }
                command.append_switch(Some(&remote_devices::WEBAUTHN_SWITCH.into()));
                #[cfg(target_os = "linux")]
                command.append_switch_with_value(Some(&"ozone-platform".into()), Some(&"headless".into()));
            }
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(ProcessHandler::new(Rc::clone(&self.ready)))
        }
    }
}

struct Tab {
    browser: Browser,
    size: Rc<Cell<(u16, u16)>>,
    generation: Rc<Cell<u64>>,
    browser_downloads: Rc<RefCell<BTreeSet<u32>>>,
    closing: Rc<Cell<bool>>,
    close_started: Cell<bool>,
}

pub struct Engine {
    tabs: BTreeMap<i32, Tab>,
    events: Rc<RefCell<VecDeque<Event>>>,
    fatal: Rc<RefCell<Option<String>>>,
    downloads: Downloads,
    download_callbacks: Rc<RefCell<BTreeMap<u32, DownloadItemCallback>>>,
    media_capture: bool,
    #[cfg(target_os = "macos")]
    _loader: cef::library_loader::LibraryLoader,
}

pub fn allowed_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url == "about:blank"
}

impl Engine {
    pub fn new(
        main_args: &cef::MainArgs,
        sandbox_info: *mut u8,
        profile: &std::path::Path,
        log_file: &std::path::Path,
        downloads: Downloads,
        media_capture: bool,
    ) -> anyhow::Result<Self> {
        #[cfg(target_os = "macos")]
        let loader = cef::library_loader::LibraryLoader::new(&std::env::current_exe()?, false);
        #[cfg(target_os = "macos")]
        anyhow::ensure!(loader.load(), "failed to load the bundled CEF framework");
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        let ready = Rc::new(Cell::new(false));
        let mut app = BrowserApp::new(Rc::clone(&ready));
        #[cfg(target_os = "linux")]
        let directory = std::env::current_exe()?
            .parent()
            .context("browser executable has no directory")?
            .to_path_buf();
        let settings = Settings {
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            command_line_args_disabled: 1,
            disable_signal_handlers: 1,
            root_cache_path: profile.to_string_lossy().as_ref().into(),
            cache_path: profile.join("cache").to_string_lossy().as_ref().into(),
            log_file: log_file.to_string_lossy().as_ref().into(),
            // UNTESTED: The Linux helper must preserve Chromium's renderer sandbox for untrusted pages.
            // Check an unprivileged X11 launch and child sandbox status when changing packaging.
            #[cfg(target_os = "linux")]
            browser_subprocess_path: directory.join("ironrdp-cef-helper").to_string_lossy().as_ref().into(),
            #[cfg(target_os = "linux")]
            resources_dir_path: directory.to_string_lossy().as_ref().into(),
            #[cfg(target_os = "linux")]
            locales_dir_path: directory.join("locales").to_string_lossy().as_ref().into(),
            ..Default::default()
        };
        anyhow::ensure!(
            initialize(Some(main_args), Some(&settings), Some(&mut app), sandbox_info) == 1,
            "CEF initialization failed"
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while !ready.get() {
            if deadline <= Instant::now() {
                cef::shutdown();
                anyhow::bail!("CEF initialization timed out");
            }
            cef::do_message_loop_work();
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(Self {
            tabs: BTreeMap::new(),
            events: Rc::new(RefCell::new(VecDeque::new())),
            fatal: Rc::new(RefCell::new(None)),
            downloads,
            download_callbacks: Rc::new(RefCell::new(BTreeMap::new())),
            media_capture,
            #[cfg(target_os = "macos")]
            _loader: loader,
        })
    }

    pub fn create(&mut self, tab: i32, url: &str, width: u16, height: u16, generation: u64) -> anyhow::Result<()> {
        anyhow::ensure!(allowed_url(url), "unsupported navigation URL");
        anyhow::ensure!(self.tabs.len() < 16, "too many browser instances awaiting closure");
        let size = Rc::new(Cell::new((width, height)));
        let generation = Rc::new(Cell::new(generation));
        let browser_downloads = Rc::new(RefCell::new(BTreeSet::new()));
        let closing = Rc::new(Cell::new(false));
        let bridge = Bridge {
            tab,
            size: Rc::clone(&size),
            generation: Rc::clone(&generation),
            events: Rc::clone(&self.events),
            fatal: Rc::clone(&self.fatal),
            downloads: self.downloads.clone(),
            download_callbacks: Rc::clone(&self.download_callbacks),
            browser_downloads: Rc::clone(&browser_downloads),
            closing: Rc::clone(&closing),
            media_capture: self.media_capture,
        };
        let window = WindowInfo {
            windowless_rendering_enabled: 1,
            ..Default::default()
        };
        let settings = BrowserSettings {
            windowless_frame_rate: 30,
            background_color: 0xFFFF_FFFF,
            ..Default::default()
        };
        let browser = browser_host_create_browser_sync(
            Some(&window),
            Some(&mut BrowserClient::new(bridge)),
            Some(&url.into()),
            Some(&settings),
            None,
            None,
        )
        .context("failed to create an offscreen browser")?;
        if let Some(host) = browser.host() {
            host.set_audio_muted(1);
        }
        self.tabs.insert(
            tab,
            Tab {
                browser,
                size,
                generation,
                browser_downloads,
                closing,
                close_started: Cell::new(false),
            },
        );
        Ok(())
    }

    pub fn browser(&self, tab: i32) -> Option<&Browser> {
        self.tabs.get(&tab).map(|tab| &tab.browser)
    }

    pub fn activate(&self, active: i32) {
        for (&id, tab) in &self.tabs {
            if let Some(host) = tab.browser.host() {
                host.was_hidden(i32::from(id != active));
                host.set_focus(i32::from(id == active));
                if id == active {
                    host.invalidate(PaintElementType::VIEW);
                }
            }
        }
    }

    pub fn resize(&self, width: u16, height: u16, generation: u64) {
        self.events
            .borrow_mut()
            .retain(|event| !matches!(event, Event::Popup { .. } | Event::HidePopup(_)));
        for tab in self.tabs.values() {
            tab.size.set((width, height));
            // UNTESTED: CEF paints must follow UI-thread resize order.
            // Paints have no native resize token; check rapid size cycling when updating CEF.
            tab.generation.set(generation);
            if let Some(host) = tab.browser.host() {
                host.was_resized();
                host.notify_screen_info_changed();
            }
        }
    }

    pub fn close(&self, tab: i32) -> anyhow::Result<()> {
        if let Some(tab) = self.tabs.get(&tab) {
            if tab.closing.replace(true) {
                return Ok(());
            }
            let host = tab.browser.host().context("closed tab has no CEF host")?;
            host.set_focus(0);
            host.was_hidden(1);
            if tab.browser_downloads.borrow().is_empty() {
                tab.close_started.set(true);
                host.close_browser(1);
            } else {
                let frame = tab.browser.main_frame().context("closed tab has no CEF main frame")?;
                frame.load_url(Some(&"about:blank".into()));
            }
        }
        Ok(())
    }

    pub fn cancel_downloads(&self) {
        self.downloads.cancel_all();
    }

    pub fn download_status(&self) -> String {
        self.downloads.status()
    }

    pub fn poll(&mut self) -> anyhow::Result<Vec<Event>> {
        cef::do_message_loop_work();
        let cancelled = self
            .download_callbacks
            .borrow()
            .iter()
            .filter(|(id, _)| self.downloads.should_cancel(**id))
            .map(|(_, callback)| callback.clone())
            .collect::<Vec<_>>();
        for callback in cancelled {
            callback.cancel();
        }
        for tab in self.tabs.values() {
            if tab.closing.get() && !tab.close_started.get() && tab.browser_downloads.borrow().is_empty() {
                if let Some(host) = tab.browser.host() {
                    tab.close_started.set(true);
                    host.close_browser(1);
                }
            }
        }
        if let Some(error) = self.fatal.borrow_mut().take() {
            anyhow::bail!("{error}");
        }
        let events: Vec<_> = self.events.borrow_mut().drain(..).collect();
        for event in &events {
            if let Event::Closed(tab) = event {
                self.tabs.remove(tab);
            }
        }
        Ok(events)
    }

    pub fn shutdown(mut self) -> anyhow::Result<()> {
        self.downloads.cancel_all();
        let callbacks = self.download_callbacks.borrow().values().cloned().collect::<Vec<_>>();
        for callback in callbacks {
            callback.cancel();
        }
        for tab in self.tabs.values() {
            tab.closing.set(true);
            if !tab.close_started.replace(true) {
                if let Some(host) = tab.browser.host() {
                    host.close_browser(1);
                }
            }
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut failure = None;
        while !self.tabs.is_empty() {
            if deadline <= Instant::now() {
                // UNTESTED: A timed-out browser must keep CEF and its callbacks alive until process exit.
                // Force a browser-close timeout and check the library is not unloaded while helpers are alive.
                std::mem::forget(self);
                anyhow::bail!("CEF browser shutdown timed out");
            }
            if let Err(error) = self.poll() {
                tracing::error!(%error, "CEF failed while closing browsers");
                failure = Some(error);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.download_callbacks.borrow_mut().clear();
        cef::shutdown();
        failure.map_or(Ok(()), Err)
    }
}

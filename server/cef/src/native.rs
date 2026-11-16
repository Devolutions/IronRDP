use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::cursor::CursorShape;
use crate::download::runtime::Downloads;
use crate::location::{SessionViewConfig, timezone_id_from_rdp};
use crate::remote_devices;
use crate::sound::{AudioOutput, CAPTURE_FRAMES_PER_BUFFER, CAPTURE_SAMPLE_RATE};
use anyhow::Context as _;
use cef::*;
use ironrdp_rdpel::server::LocationFix;

#[derive(Clone, Copy, Debug)]
pub struct Permissions {
    pub media_capture: bool,
    pub location: bool,
}

#[cfg(target_os = "macos")]
type NativeCursorHandle = *mut u8;
#[cfg(windows)]
type NativeCursorHandle = cef::sys::HCURSOR;
#[cfg(target_os = "linux")]
type NativeCursorHandle = std::os::raw::c_ulong;

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
    Cursor(i32, CursorShape),
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
    geolocation_ready: Rc<Cell<bool>>,
    closing: Rc<Cell<bool>>,
    permissions: Permissions,
    sound: AudioOutput,
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
        } else if let Event::Cursor(tab, _) = &event {
            events.retain(|old| !matches!(old, Event::Cursor(id, _) if id == tab));
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
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: NativeCursorHandle,
            kind: CursorType,
            custom: Option<&CursorInfo>,
        ) -> i32 {
            self.bridge
                .emit(Event::Cursor(self.bridge.tab, CursorShape::from_cef(kind, custom)));
            1
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

wrap_print_handler! {
    struct PrintPolicy { bridge: Bridge }
    impl PrintHandler {
        fn on_print_dialog(
            &self,
            _browser: Option<&mut Browser>,
            _has_selection: i32,
            callback: Option<&mut PrintDialogCallback>,
        ) -> i32 {
            if let Some(callback) = callback {
                callback.cancel();
            }
            self.bridge.emit(Event::Error(
                self.bridge.tab,
                "native print dialogs are disabled; press Ctrl+P to export PDF".to_owned(),
            ));
            1
        }
    }
}

wrap_pdf_print_callback! {
    struct PdfDoneCallback {
        bridge: Bridge,
        pdf_printing: Rc<Cell<bool>>,
        download_id: u32,
        path: std::path::PathBuf,
    }
    impl PdfPrintCallback {
        fn on_pdf_print_finished(&self, _path: Option<&CefString>, ok: i32) {
            self.pdf_printing.set(false);
            self.bridge.browser_downloads.borrow_mut().remove(&self.download_id);
            let received = std::fs::metadata(&self.path).map_or(0, |meta| meta.len());
            let cancelled = self.bridge.downloads.should_cancel(self.download_id);
            let complete = !cancelled && ok != 0 && received > 0;
            self.bridge
                .downloads
                .progress(self.download_id, received, complete, !complete);
            if !complete
                || self.bridge.downloads.should_cancel(self.download_id)
                || !self.bridge.downloads.tracked(self.download_id)
            {
                let _ = std::fs::remove_file(&self.path);
            }
            if !cancelled && !complete {
                self.bridge.emit(Event::Error(
                    self.bridge.tab,
                    "failed to render page to PDF".to_owned(),
                ));
            }
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

// UNTESTED: Permission must never expose host cameras, microphones, or location services.
// Check the remote-media switches, DevTools geolocation override, and device factory hooks when changing this policy.
wrap_permission_handler! {
    struct MediaPolicy {
        permissions: Permissions,
        geolocation_ready: Rc<Cell<bool>>,
    }
    impl PermissionHandler {
        fn on_request_media_access_permission(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
                                              _origin: Option<&CefString>, requested: u32,
                                              callback: Option<&mut MediaAccessCallback>) -> std::ffi::c_int {
            use cef::sys::cef_media_access_permission_types_t as Types;
            const DEVICES: u32 =
                Types::CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE as u32 | Types::CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE as u32;
            // Screen capture would expose the host display, so only the remote camera and microphone qualify.
            let allowed = if self.permissions.media_capture && requested & !DEVICES == 0 { requested } else { 0 };
            if let Some(callback) = callback { callback.cont(allowed); }
            1
        }
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            _origin: Option<&CefString>,
            requested: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> std::ffi::c_int {
            // The raw enum is `u32` on Unix but `i32` with MSVC, so cast the discriminant rather than `get_raw()`.
            let geolocation = cef::sys::cef_permission_request_types_t::CEF_PERMISSION_TYPE_GEOLOCATION as u32;
            let result = if self.permissions.location && self.geolocation_ready.get() && requested == geolocation {
                PermissionRequestResult::ACCEPT
            } else {
                PermissionRequestResult::DENY
            };
            if let Some(callback) = callback {
                callback.cont(result);
            }
            1
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SubtargetEvent {
    Attached {
        tab: i32,
        parent_session_id: Option<String>,
        session_id: String,
        target_type: String,
        waiting_for_debugger: bool,
    },
    Detached {
        tab: i32,
        session_id: String,
    },
}

fn remove_session_tree(
    session_parents: &mut BTreeMap<String, Option<String>>,
    child_sessions: &mut BTreeSet<String>,
    root_session_id: &str,
) -> bool {
    let mut stack = vec![root_session_id.to_owned()];
    let mut removed_emulation = false;
    while let Some(sid) = stack.pop() {
        if session_parents.remove(&sid).is_some() {
            removed_emulation |= child_sessions.remove(&sid);
        }
        let children: Vec<String> = session_parents
            .iter()
            .filter(|(_, parent)| parent.as_deref() == Some(sid.as_str()))
            .map(|(child, _)| child.clone())
            .collect();
        stack.extend(children);
    }
    removed_emulation
}

const CDP_SESSION_NOT_FOUND: i32 = -32001;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CdpReply {
    Ok,
    Rejected,
    SessionNotFound,
}

#[derive(Default)]
struct DevToolsMail {
    expected: BTreeSet<i32>,
    results: BTreeMap<i32, CdpReply>,
    subtargets: VecDeque<SubtargetEvent>,
}

impl DevToolsMail {
    fn expect_id(&mut self, id: i32) {
        self.expected.insert(id);
    }

    fn record_response(&mut self, id: i32, reply: CdpReply) {
        if self.expected.remove(&id) {
            self.results.insert(id, reply);
        }
    }

    fn release_ids(&mut self, ids: &[i32]) {
        for id in ids {
            self.expected.remove(id);
            self.results.remove(id);
        }
    }

    fn drain_detachments(&mut self, mut on_detached: impl FnMut(i32, &str)) {
        let mut retained: VecDeque<SubtargetEvent> = VecDeque::with_capacity(self.subtargets.len());
        while let Some(event) = self.subtargets.pop_front() {
            match event {
                SubtargetEvent::Detached { tab, session_id } => {
                    let mut dead = BTreeSet::from([session_id.clone()]);
                    loop {
                        let mut grew = false;
                        for queued in &retained {
                            if let SubtargetEvent::Attached {
                                tab: t,
                                parent_session_id: Some(parent),
                                session_id: child,
                                ..
                            } = queued
                                && *t == tab
                                && dead.contains(parent)
                                && !dead.contains(child)
                            {
                                dead.insert(child.clone());
                                grew = true;
                            }
                        }
                        if !grew {
                            break;
                        }
                    }
                    retained.retain(|queued| {
                        !matches!(
                            queued,
                            SubtargetEvent::Attached { tab: t, session_id: s, .. }
                                if *t == tab && dead.contains(s)
                        )
                    });
                    for sid in &dead {
                        on_detached(tab, sid);
                    }
                }
                attached => retained.push_back(attached),
            }
        }
        self.subtargets = retained;
    }

    fn resolve_single(&mut self, id: i32, target_gone: bool) -> Option<bool> {
        if target_gone {
            self.expected.remove(&id);
            self.results.remove(&id);
            return Some(true);
        }
        match self.results.get(&id).copied() {
            Some(CdpReply::Ok) => {
                self.results.remove(&id);
                Some(true)
            }
            Some(CdpReply::Rejected) => {
                self.results.remove(&id);
                Some(false)
            }
            // Hold `-32001` (`SESSION_NOT_FOUND`) pending across event-loop pumps until
            // `Target.detachedFromTarget` marks `target_gone`, or let the bounded wait deadline
            // fail closed if detachment never arrives.
            Some(CdpReply::SessionNotFound) | None => None,
        }
    }
}

wrap_dev_tools_message_observer! {
    struct DevToolsObserver {
        tab: i32,
        mail: Rc<RefCell<DevToolsMail>>,
    }
    impl DevToolsMessageObserver {
        fn on_dev_tools_message(
            &self,
            _browser: Option<&mut Browser>,
            message: Option<&[u8]>,
        ) -> std::os::raw::c_int {
            let Some(raw) = message.and_then(|bytes| std::str::from_utf8(bytes).ok()) else {
                return 0;
            };
            let Some(dict) = parse_json(Some(&raw.into()), JsonParserOptions::default())
                .and_then(|value| value.dictionary())
            else {
                return 0;
            };
            if dict.get_type(Some(&"id".into())) == ValueType::INT {
                let id = dict.int(Some(&"id".into()));
                let reply = match dict.dictionary(Some(&"error".into())) {
                    None => CdpReply::Ok,
                    Some(err) if err.int(Some(&"code".into())) == CDP_SESSION_NOT_FOUND => CdpReply::SessionNotFound,
                    Some(_) => CdpReply::Rejected,
                };
                self.mail.borrow_mut().record_response(id, reply);
                return 1;
            }
            let parent_session_id = {
                let outer_sid = CefString::from(&dict.string(Some(&"sessionId".into()))).to_string();
                is_safe_cdp_session_id(&outer_sid).then_some(outer_sid)
            };
            let method = CefString::from(&dict.string(Some(&"method".into()))).to_string();
            let Some(params) = dict.dictionary(Some(&"params".into())) else {
                return 1;
            };
            match method.as_str() {
                "Target.attachedToTarget" => {
                    let session_id = CefString::from(&params.string(Some(&"sessionId".into()))).to_string();
                    let target_type = params
                        .dictionary(Some(&"targetInfo".into()))
                        .map(|info| CefString::from(&info.string(Some(&"type".into()))).to_string())
                        .unwrap_or_default();
                    let waiting_for_debugger = params.bool(Some(&"waitingForDebugger".into())) != 0;
                    if is_safe_cdp_session_id(&session_id) {
                        self.mail.borrow_mut().subtargets.push_back(SubtargetEvent::Attached {
                            tab: self.tab,
                            parent_session_id,
                            session_id,
                            target_type,
                            waiting_for_debugger,
                        });
                    }
                }
                "Target.detachedFromTarget" => {
                    let session_id = CefString::from(&params.string(Some(&"sessionId".into()))).to_string();
                    if is_safe_cdp_session_id(&session_id) {
                        self.mail
                            .borrow_mut()
                            .subtargets
                            .push_back(SubtargetEvent::Detached { tab: self.tab, session_id });
                    }
                }
                _ => {}
            }
            1
        }
    }
}

wrap_audio_handler! {
    struct SoundCapture {
        tab: i32,
        sound: AudioOutput,
        channels: Arc<AtomicUsize>,
    }
    impl AudioHandler {
        fn audio_parameters(&self, _browser: Option<&mut Browser>, params: Option<&mut AudioParameters>) -> i32 {
            if let Some(params) = params {
                params.channel_layout = ChannelLayout::LAYOUT_STEREO;
                params.sample_rate = i32::try_from(CAPTURE_SAMPLE_RATE).expect("sample rate");
                params.frames_per_buffer = CAPTURE_FRAMES_PER_BUFFER;
            }
            1
        }
        fn on_audio_stream_started(
            &self,
            _browser: Option<&mut Browser>,
            _params: Option<&AudioParameters>,
            channels: i32,
        ) {
            self.channels.store(usize::try_from(channels).unwrap_or(0), Ordering::Release);
        }
        fn on_audio_stream_packet(
            &self,
            _browser: Option<&mut Browser>,
            data: *mut *const f32,
            frames: i32,
            pts: i64,
        ) {
            let Ok(frames) = usize::try_from(frames) else {
                return;
            };
            let channel_count = self.channels.load(Ordering::Acquire);
            if data.is_null() || frames == 0 || !(1..=8).contains(&channel_count) {
                return;
            }
            // SAFETY: CEF passes `channel_count` channel pointers, each holding `frames` f32 samples for this call.
            let raw_planes = unsafe { std::slice::from_raw_parts(data.cast_const(), channel_count) };
            let mut planes = Vec::with_capacity(channel_count);
            for &ptr in raw_planes {
                if ptr.is_null() {
                    return;
                }
                // SAFETY: Verified non-null channel buffer of `frames` f32 values owned by CEF during callback.
                planes.push(unsafe { std::slice::from_raw_parts(ptr, frames) });
            }
            self.sound.push_planar_f32(self.tab, &planes, frames, pts);
        }
        fn on_audio_stream_stopped(&self, _browser: Option<&mut Browser>) {
            self.channels.store(0, Ordering::Release);
        }
        fn on_audio_stream_error(&self, _browser: Option<&mut Browser>, message: Option<&CefString>) {
            let message = message.map(ToString::to_string).unwrap_or_default();
            tracing::warn!(tab = self.tab, %message, "CEF audio capture reported an error");
        }
    }
}

wrap_client! {
    struct BrowserClient { bridge: Bridge }
    impl Client {
        fn audio_handler(&self) -> Option<AudioHandler> {
            self.bridge.sound.is_enabled().then(|| {
                SoundCapture::new(self.bridge.tab, self.bridge.sound.clone(), Arc::new(AtomicUsize::new(0)))
            })
        }
        fn render_handler(&self) -> Option<RenderHandler> { Some(PaintHandler::new(self.bridge.clone())) }
        fn display_handler(&self) -> Option<DisplayHandler> { Some(DisplayHandlerImpl::new(self.bridge.clone())) }
        fn load_handler(&self) -> Option<LoadHandler> { Some(LoadHandlerImpl::new(self.bridge.clone())) }
        fn life_span_handler(&self) -> Option<LifeSpanHandler> { Some(LifeHandler::new(self.bridge.clone())) }
        fn download_handler(&self) -> Option<DownloadHandler> { Some(DownloadHandlerImpl::new(self.bridge.clone())) }
        fn dialog_handler(&self) -> Option<DialogHandler> { Some(DialogPolicy::new(self.bridge.clone())) }
        fn print_handler(&self) -> Option<PrintHandler> { Some(PrintPolicy::new(self.bridge.clone())) }
        fn jsdialog_handler(&self) -> Option<JsdialogHandler> { Some(JsDialogPolicy::new(self.bridge.clone())) }
        fn request_handler(&self) -> Option<RequestHandler> { Some(RequestHandlerImpl::new(self.bridge.clone())) }
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> { Some(ContextMenuPolicy::new()) }
        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(MediaPolicy::new(
                self.bridge.permissions,
                Rc::clone(&self.bridge.geolocation_ready),
            ))
        }
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
    _devtools_observer: Registration,
    session_parents: BTreeMap<String, Option<String>>,
    child_sessions: BTreeSet<String>,
    size: Rc<Cell<(u16, u16)>>,
    generation: Rc<Cell<u64>>,
    browser_downloads: Rc<RefCell<BTreeSet<u32>>>,
    geolocation_ready: Rc<Cell<bool>>,
    pdf_printing: Rc<Cell<bool>>,
    user_zoom: Cell<f64>,
    closing: Rc<Cell<bool>>,
    close_started: Cell<bool>,
}

pub struct Engine {
    tabs: BTreeMap<i32, Tab>,
    events: Rc<RefCell<VecDeque<Event>>>,
    fatal: Rc<RefCell<Option<String>>>,
    downloads: Downloads,
    download_callbacks: Rc<RefCell<BTreeMap<u32, DownloadItemCallback>>>,
    devtools_mail: Rc<RefCell<DevToolsMail>>,
    permissions: Permissions,
    sound: AudioOutput,
    location: Option<LocationFix>,
    session_view: SessionViewConfig,
    applied_timezone_id: String,
    timezone_targets_dirty: bool,
    configuring_subtargets: bool,
    updating_timezone_owners: bool,
    last_timezone_check: Instant,
    next_devtools_id: Cell<i32>,
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
        permissions: Permissions,
        sound: AudioOutput,
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
        if let Err(error) = disable_scripted_printing() {
            cef::shutdown();
            return Err(error);
        }
        Ok(Self {
            tabs: BTreeMap::new(),
            events: Rc::new(RefCell::new(VecDeque::new())),
            fatal: Rc::new(RefCell::new(None)),
            downloads,
            download_callbacks: Rc::new(RefCell::new(BTreeMap::new())),
            devtools_mail: Rc::new(RefCell::new(DevToolsMail::default())),
            permissions,
            sound,
            location: None,
            session_view: SessionViewConfig::default(),
            applied_timezone_id: "UTC".to_owned(),
            timezone_targets_dirty: false,
            configuring_subtargets: false,
            updating_timezone_owners: false,
            last_timezone_check: Instant::now(),
            next_devtools_id: Cell::new(1),
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
        let geolocation_ready = Rc::new(Cell::new(false));
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
            geolocation_ready: Rc::clone(&geolocation_ready),
            closing: Rc::clone(&closing),
            permissions: self.permissions,
            sound: self.sound.clone(),
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
            Some(&"about:blank".into()),
            Some(&settings),
            None,
            None,
        )
        .context("failed to create an offscreen browser")?;
        let host = browser.host().context("created browser has no CEF host")?;
        if !self.sound.is_enabled() {
            host.set_audio_muted(1);
        }
        let mut observer = DevToolsObserver::new(tab, Rc::clone(&self.devtools_mail));
        let Some(registration) = host.add_dev_tools_message_observer(Some(&mut observer)) else {
            closing.set(true);
            host.close_browser(1);
            anyhow::bail!("failed to register CEF DevTools message observer");
        };
        self.tabs.insert(
            tab,
            Tab {
                browser: browser.clone(),
                _devtools_observer: registration,
                session_parents: BTreeMap::new(),
                child_sessions: BTreeSet::new(),
                size,
                generation,
                browser_downloads,
                geolocation_ready,
                pdf_printing: Rc::new(Cell::new(false)),
                user_zoom: Cell::new(0.0),
                closing,
                close_started: Cell::new(false),
            },
        );
        // Confirm fail-closed geolocation (`POSITION_UNAVAILABLE`), timezone (`UTC` default), and zoom
        // overrides via DevTools observer completion before loading any requested document URL.
        if let Err(error) = self.confirm_new_tab_overrides(tab) {
            if let Some(entry) = self.tabs.get(&tab) {
                entry.geolocation_ready.set(false);
                entry.closing.set(true);
                entry.close_started.set(true);
            }
            host.close_browser(1);
            return Err(error);
        }
        if url != "about:blank"
            && let Some(frame) = browser.main_frame()
        {
            frame.load_url(Some(&url.into()));
        }
        Ok(())
    }

    pub fn browser(&self, tab: i32) -> Option<&Browser> {
        self.tabs.get(&tab).map(|tab| &tab.browser)
    }

    pub fn activate(&self, active: i32) {
        self.sound.set_active_tab(active);
        for (&id, tab) in &self.tabs {
            if let Some(host) = tab.browser.host() {
                if self.sound.is_enabled() {
                    host.set_audio_muted(i32::from(id != active));
                }
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
            tab.geolocation_ready.set(false);
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

    pub fn print_to_pdf(&self, tab: i32, title: &str) -> anyhow::Result<()> {
        let entry = self.tabs.get(&tab).context("unknown browser tab")?;
        anyhow::ensure!(!entry.closing.get(), "browser tab is closing");
        anyhow::ensure!(!entry.pdf_printing.get(), "a PDF export is already in progress");
        let host = entry.browser.host().context("browser tab has no CEF host")?;
        let (download_id, path) = match self.downloads.begin_print(title) {
            Ok(started) => started,
            Err(error) => {
                self.downloads.notice(&error.to_string());
                return Err(error);
            }
        };
        entry.pdf_printing.set(true);
        entry.browser_downloads.borrow_mut().insert(download_id);
        let bridge = Bridge {
            tab,
            size: Rc::clone(&entry.size),
            generation: Rc::clone(&entry.generation),
            events: Rc::clone(&self.events),
            fatal: Rc::clone(&self.fatal),
            downloads: self.downloads.clone(),
            download_callbacks: Rc::clone(&self.download_callbacks),
            browser_downloads: Rc::clone(&entry.browser_downloads),
            geolocation_ready: Rc::clone(&entry.geolocation_ready),
            closing: Rc::clone(&entry.closing),
            permissions: self.permissions,
            sound: self.sound.clone(),
        };
        let settings = PdfPrintSettings {
            print_background: 1,
            ..Default::default()
        };
        let cef_path: CefString = path.to_string_lossy().as_ref().into();
        let mut callback = PdfDoneCallback::new(bridge, Rc::clone(&entry.pdf_printing), download_id, path);
        host.print_to_pdf(Some(&cef_path), Some(&settings), Some(&mut callback));
        Ok(())
    }

    pub fn set_location(&mut self, location: Option<LocationFix>) -> anyhow::Result<()> {
        if self.location == location {
            return Ok(());
        }
        self.location = location;
        let ids = self.active_tab_ids();
        self.confirm_location_for_tabs(&ids)
    }

    pub fn set_session_view(&mut self, view: SessionViewConfig) -> anyhow::Result<()> {
        let tz_changed = self.session_view.timezone != view.timezone;
        self.session_view = view;
        if tz_changed {
            self.last_timezone_check = Instant::now();
        }
        let ids = self.active_tab_ids();
        if tz_changed {
            self.confirm_timezone_for_tabs(&ids)?;
        }
        for &id in &ids {
            self.apply_tab_zoom(id);
        }
        Ok(())
    }

    pub fn reset_session_overrides(&mut self) -> anyhow::Result<()> {
        self.location = None;
        self.session_view = SessionViewConfig::default();
        self.last_timezone_check = Instant::now();
        for entry in self.tabs.values() {
            entry.geolocation_ready.set(false);
        }
        let ids = self.active_tab_ids();
        self.confirm_location_for_tabs(&ids)?;
        self.confirm_timezone_for_tabs(&ids)?;
        for &id in &ids {
            self.apply_tab_zoom(id);
        }
        Ok(())
    }

    pub fn adjust_zoom(&self, tab: i32, delta: i8) {
        let Some(entry) = self.tabs.get(&tab) else {
            return;
        };
        let next = match delta {
            0 => 0.0,
            d => (entry.user_zoom.get() + f64::from(d) * 0.5).clamp(-5.0, 5.0),
        };
        entry.user_zoom.set(next);
        self.apply_tab_zoom(tab);
    }

    pub fn refresh_tab_zoom(&self, tab: i32) {
        self.apply_tab_zoom(tab);
    }

    fn active_tab_ids(&self) -> Vec<i32> {
        self.tabs
            .iter()
            .filter_map(|(&id, entry)| (!entry.closing.get()).then_some(id))
            .collect()
    }

    fn confirm_new_tab_overrides(&mut self, tab: i32) -> anyhow::Result<()> {
        self.confirm_auto_attach(tab, None)?;
        self.confirm_location_for_tabs(&[tab])?;
        self.confirm_timezone_for_tabs(&[tab])?;
        self.apply_tab_zoom(tab);
        Ok(())
    }

    fn confirm_auto_attach(&mut self, tab: i32, session_id: Option<&str>) -> anyhow::Result<()> {
        let id = self.send_cdp_command(
            tab,
            session_id,
            "Target.setAutoAttach",
            "{\"autoAttach\":true,\"waitForDebuggerOnStart\":true,\"flatten\":true}",
        )?;
        let outcomes = self.wait_devtools_outcomes(&[(id, (tab, session_id.map(ToOwned::to_owned)))])?;
        anyhow::ensure!(outcomes == [true], "CEF rejected Target.setAutoAttach");
        Ok(())
    }

    fn next_message_id(&self) -> i32 {
        let id = self.next_devtools_id.get();
        self.next_devtools_id.set(id.wrapping_add(1).max(1));
        id
    }

    fn send_cdp_command(
        &self,
        tab: i32,
        session_id: Option<&str>,
        method: &str,
        params_json: &str,
    ) -> anyhow::Result<i32> {
        let entry = self.tabs.get(&tab).context("unknown browser tab")?;
        let host = entry.browser.host().context("browser tab has no CEF host")?;
        let id = self.next_message_id();
        let payload = match session_id {
            Some(sid) => {
                format!("{{\"id\":{id},\"sessionId\":\"{sid}\",\"method\":\"{method}\",\"params\":{params_json}}}")
            }
            None => format!("{{\"id\":{id},\"method\":\"{method}\",\"params\":{params_json}}}"),
        };
        self.devtools_mail.borrow_mut().expect_id(id);
        if host.send_dev_tools_message(Some(payload.as_bytes())) == 0 {
            self.devtools_mail.borrow_mut().release_ids(&[id]);
            anyhow::bail!("failed to send DevTools command {method}");
        }
        Ok(id)
    }

    fn confirm_location_for_tabs(&mut self, tabs: &[i32]) -> anyhow::Result<()> {
        let mut submitted = Vec::with_capacity(tabs.len());
        for &tab in tabs {
            let Some(entry) = self.tabs.get(&tab) else {
                continue;
            };
            entry.geolocation_ready.set(false);
            if entry.closing.get() {
                continue;
            }
            let params_json = match self.location {
                Some(fix) => {
                    let accuracy = fix.horizontal_accuracy.unwrap_or(25.0).max(1.0);
                    let mut fields = format!(
                        "\"latitude\":{},\"longitude\":{},\"accuracy\":{},\"altitude\":{}",
                        fix.latitude, fix.longitude, accuracy, fix.altitude
                    );
                    if let Some(speed) = fix.speed {
                        fields.push_str(&format!(",\"speed\":{speed}"));
                    }
                    if let Some(heading) = fix.heading {
                        fields.push_str(&format!(",\"heading\":{heading}"));
                    }
                    format!("{{{fields}}}")
                }
                // Empty params instruct Chromium's DevTools emulation to report POSITION_UNAVAILABLE,
                // keeping host OS location providers unreachable whenever no client fix is active.
                None => "{}".to_owned(),
            };
            let assigned = self.send_cdp_command(tab, None, "Emulation.setGeolocationOverride", &params_json)?;
            submitted.push((assigned, (tab, None)));
        }
        let outcomes = self.wait_devtools_outcomes(&submitted)?;
        anyhow::ensure!(
            outcomes.iter().all(|&ok| ok),
            "CEF rejected Emulation.setGeolocationOverride"
        );
        for (_, (tab, _)) in submitted {
            if let Some(entry) = self.tabs.get(&tab)
                && !entry.closing.get()
            {
                entry.geolocation_ready.set(true);
            }
        }
        Ok(())
    }

    fn desired_timezone_id(&self) -> String {
        self.session_view
            .timezone
            .as_ref()
            .and_then(timezone_id_from_rdp)
            .filter(|id| is_safe_timezone_id(id))
            .unwrap_or_else(|| "UTC".to_owned())
    }

    fn confirm_timezone_for_tabs(&mut self, tabs: &[i32]) -> anyhow::Result<()> {
        let desired = self.desired_timezone_id();
        // Record `applied_timezone_id` up front so any child target attaching while responses are
        // in flight receives the new timezone rather than a stale value.
        self.applied_timezone_id.clone_from(&desired);
        if tabs.is_empty() {
            self.timezone_targets_dirty = false;
            return Ok(());
        }
        // `desired` comes exclusively from our closed set of verified ICU zone IDs (`UTC`,
        // `GMT±HH:MM`, or invariant ICU zones), and updates directly in-place without ever
        // clearing to `""`.
        anyhow::ensure!(
            self.apply_timezone_in_place(tabs, &desired)?,
            "CEF rejected timezone override {desired}"
        );
        Ok(())
    }

    fn apply_timezone_in_place(&mut self, tabs: &[i32], timezone_id: &str) -> anyhow::Result<bool> {
        for _ in 0..8 {
            self.timezone_targets_dirty = false;
            let mut targets = Vec::new();
            for &tab in tabs {
                let Some(entry) = self.tabs.get(&tab) else {
                    continue;
                };
                if entry.closing.get() {
                    continue;
                }
                targets.push((tab, None));
                for sid in &entry.child_sessions {
                    targets.push((tab, Some(sid.clone())));
                }
            }
            if !targets.is_empty() {
                // Update existing renderer owners BEFORE configuring newly queued paused children so
                // a same-process worker/OOPIF never contends with a stale owner override.
                self.updating_timezone_owners = true;
                let owner_res = (|| -> anyhow::Result<bool> {
                    let failed = self.submit_and_collect_failed_timezone(&targets, timezone_id)?;
                    if !failed.is_empty() {
                        let retry_failed = self.submit_and_collect_failed_timezone(&failed, timezone_id)?;
                        if !retry_failed.is_empty() {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                })();
                self.updating_timezone_owners = false;
                if !owner_res? {
                    return Ok(false);
                }
            }
            self.process_subtarget_events()?;
            if !self.timezone_targets_dirty && self.devtools_mail.borrow().subtargets.is_empty() {
                return Ok(true);
            }
        }
        anyhow::bail!("CEF subtarget timezone overrides did not converge")
    }

    fn submit_and_collect_failed_timezone(
        &mut self,
        targets: &[(i32, Option<String>)],
        timezone_id: &str,
    ) -> anyhow::Result<Vec<(i32, Option<String>)>> {
        let params_json = format!("{{\"timezoneId\":\"{timezone_id}\"}}");
        let mut pending = Vec::with_capacity(targets.len());
        for (tab, session) in targets {
            let Some(entry) = self.tabs.get(tab) else {
                continue;
            };
            if entry.closing.get() {
                continue;
            }
            if let Some(sid) = session
                && !entry.child_sessions.contains(sid)
            {
                continue;
            }
            let id = self.send_cdp_command(*tab, session.as_deref(), "Emulation.setTimezoneOverride", &params_json)?;
            pending.push((id, (*tab, session.clone())));
        }
        let outcomes = self.wait_devtools_outcomes(&pending)?;
        let mut failed = Vec::new();
        for ((_, target), ok) in pending.into_iter().zip(outcomes) {
            if !ok {
                failed.push(target);
            }
        }
        Ok(failed)
    }

    fn drain_detached_subtargets(&mut self) {
        let mut dirty = false;
        let tabs = &mut self.tabs;
        self.devtools_mail.borrow_mut().drain_detachments(|tab, session_id| {
            if let Some(entry) = tabs.get_mut(&tab) {
                dirty |= remove_session_tree(&mut entry.session_parents, &mut entry.child_sessions, session_id);
            }
        });
        self.timezone_targets_dirty |= dirty;
    }

    fn is_subtarget_gone(&self, tab: i32, session_id: Option<&str>) -> bool {
        let Some(sid) = session_id else {
            return false;
        };
        self.tabs
            .get(&tab)
            .is_none_or(|entry| entry.closing.get() || !entry.session_parents.contains_key(sid))
    }

    fn process_subtarget_events(&mut self) -> anyhow::Result<()> {
        if self.configuring_subtargets || self.updating_timezone_owners {
            self.drain_detached_subtargets();
            return Ok(());
        }
        self.configuring_subtargets = true;
        let result = (|| -> anyhow::Result<()> {
            loop {
                // Always fold queued detachments first so an `[Attached(S), Detached(S)]` pair
                // cancels out before any command is ever sent to the dead session `S`.
                self.drain_detached_subtargets();
                let next = self.devtools_mail.borrow_mut().subtargets.pop_front();
                let Some(event) = next else {
                    break;
                };
                if let SubtargetEvent::Attached {
                    tab,
                    parent_session_id,
                    session_id,
                    target_type,
                    waiting_for_debugger,
                } = event
                {
                    let supports_emulation = matches!(
                        target_type.as_str(),
                        "page" | "iframe" | "webview" | "worker" | "shared_worker" | "service_worker"
                    );
                    {
                        let Some(entry) = self.tabs.get_mut(&tab) else {
                            continue;
                        };
                        if entry.closing.get() {
                            continue;
                        }
                        if let Some(ref parent) = parent_session_id
                            && !entry.session_parents.contains_key(parent)
                        {
                            // Parent session already detached before its child's attach was popped.
                            continue;
                        }
                        entry.session_parents.insert(session_id.clone(), parent_session_id);
                        if supports_emulation {
                            entry.child_sessions.insert(session_id.clone());
                            self.timezone_targets_dirty = true;
                        }
                    }
                    if supports_emulation {
                        self.confirm_auto_attach(tab, Some(&session_id))?;
                        if !self.is_subtarget_gone(tab, Some(&session_id)) {
                            let tz = self.applied_timezone_id.clone();
                            let single = [(tab, Some(session_id.clone()))];
                            let failed = self.submit_and_collect_failed_timezone(&single, &tz)?;
                            if !failed.is_empty() && !self.is_subtarget_gone(tab, Some(&session_id)) {
                                // Ensure any existing owner across active tabs has updated to `tz`,
                                // then retry the paused child target before failing closed.
                                let mut all_targets = Vec::new();
                                for (&id, entry) in &self.tabs {
                                    if !entry.closing.get() {
                                        all_targets.push((id, None));
                                        for sid in &entry.child_sessions {
                                            all_targets.push((id, Some(sid.clone())));
                                        }
                                    }
                                }
                                let _ = self.submit_and_collect_failed_timezone(&all_targets, &tz)?;
                                let retry = self.submit_and_collect_failed_timezone(&single, &tz)?;
                                anyhow::ensure!(
                                    retry.is_empty(),
                                    "CEF rejected child target Emulation.setTimezoneOverride"
                                );
                            }
                        }
                    }
                    if waiting_for_debugger && !self.is_subtarget_gone(tab, Some(&session_id)) {
                        let resume_id =
                            self.send_cdp_command(tab, Some(&session_id), "Runtime.runIfWaitingForDebugger", "{}")?;
                        let outcomes = self.wait_devtools_outcomes(&[(resume_id, (tab, Some(session_id)))])?;
                        anyhow::ensure!(
                            outcomes == [true],
                            "CEF failed to resume child target after override confirmation"
                        );
                    }
                }
            }
            Ok(())
        })();
        self.configuring_subtargets = false;
        result
    }

    fn wait_devtools_outcomes(&mut self, pending: &[(i32, (i32, Option<String>))]) -> anyhow::Result<Vec<bool>> {
        if pending.is_empty() {
            return Ok(Vec::new());
        }
        struct CleanupGuard {
            mail: Rc<RefCell<DevToolsMail>>,
            ids: Vec<i32>,
        }
        impl Drop for CleanupGuard {
            fn drop(&mut self) {
                self.mail.borrow_mut().release_ids(&self.ids);
            }
        }
        let ids: Vec<i32> = pending.iter().map(|&(id, _)| id).collect();
        let _guard = CleanupGuard {
            mail: Rc::clone(&self.devtools_mail),
            ids,
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut resolved: BTreeMap<i32, bool> = BTreeMap::new();
        loop {
            self.process_subtarget_events()?;
            for &(id, (tab, ref sid)) in pending {
                if resolved.contains_key(&id) {
                    continue;
                }
                let gone = self.is_subtarget_gone(tab, sid.as_deref());
                if let Some(ok) = self.devtools_mail.borrow_mut().resolve_single(id, gone) {
                    resolved.insert(id, ok);
                }
            }
            if resolved.len() == pending.len() {
                return Ok(pending.iter().map(|(id, _)| resolved[id]).collect());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out awaiting CEF DevTools override confirmation");
            }
            cef::do_message_loop_work();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn apply_tab_zoom(&self, tab: i32) {
        let Some(entry) = self.tabs.get(&tab) else {
            return;
        };
        let Some(host) = entry.browser.host() else {
            return;
        };
        let combined = (self.session_view.base_zoom + entry.user_zoom.get()).clamp(-6.0, 8.0);
        if (host.zoom_level() - combined).abs() > 1e-4 {
            host.set_zoom_level(combined);
        }
    }

    pub fn download_status(&self) -> String {
        self.downloads.status()
    }

    pub fn poll(&mut self) -> anyhow::Result<Vec<Event>> {
        cef::do_message_loop_work();
        self.process_subtarget_events()?;
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
            if let Event::Closed(tab) = event
                && self.tabs.remove(tab).is_some()
            {
                self.timezone_targets_dirty = true;
            }
        }
        let dst_shifted =
            self.session_view.timezone.is_some() && self.last_timezone_check.elapsed() >= Duration::from_secs(1) && {
                self.last_timezone_check = Instant::now();
                self.desired_timezone_id() != self.applied_timezone_id
            };
        if self.timezone_targets_dirty || dst_shifted {
            let ids = self.active_tab_ids();
            self.confirm_timezone_for_tabs(&ids)?;
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

fn is_safe_cdp_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

fn is_safe_timezone_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'+' | b':'))
}

fn disable_scripted_printing() -> anyhow::Result<()> {
    let context = request_context_get_global_context().context("global CEF request context is unavailable")?;
    let mut disabled = value_create().context("failed to allocate CEF preference value")?;
    anyhow::ensure!(
        disabled.set_bool(0) == 1,
        "failed to initialize printing.enabled preference"
    );
    let mut error = CefString::from("");
    anyhow::ensure!(
        context.set_preference(Some(&"printing.enabled".into()), Some(&mut disabled), Some(&mut error)) == 1,
        "failed to disable scripted printing: {error}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: CDP Target.attachedToTarget/detachedFromTarget ordering; mutant sending commands to
    // `[Attached, Detached]` churn, leaking orphaned `results` entries, or failing when a detached
    // target's `-32001` error arrives alongside detachment.
    #[test]
    fn devtools_mail_cancels_queued_attach_drops_untracked_replies_and_ignores_detached_error() {
        let mut mail = DevToolsMail::default();
        mail.subtargets.push_back(SubtargetEvent::Attached {
            tab: 1,
            parent_session_id: None,
            session_id: "s-ephemeral".to_owned(),
            target_type: "iframe".to_owned(),
            waiting_for_debugger: true,
        });
        mail.subtargets.push_back(SubtargetEvent::Attached {
            tab: 1,
            parent_session_id: Some("s-ephemeral".to_owned()),
            session_id: "s-ephemeral-worker".to_owned(),
            target_type: "worker".to_owned(),
            waiting_for_debugger: true,
        });
        mail.subtargets.push_back(SubtargetEvent::Attached {
            tab: 1,
            parent_session_id: None,
            session_id: "s-live".to_owned(),
            target_type: "worker".to_owned(),
            waiting_for_debugger: true,
        });
        mail.subtargets.push_back(SubtargetEvent::Detached {
            tab: 1,
            session_id: "s-ephemeral".to_owned(),
        });

        let mut detached = Vec::new();
        mail.drain_detachments(|tab, sid| detached.push((tab, sid.to_owned())));
        assert_eq!(
            detached,
            vec![(1, "s-ephemeral".to_owned()), (1, "s-ephemeral-worker".to_owned())]
        );
        assert_eq!(
            mail.subtargets.iter().collect::<Vec<_>>(),
            vec![&SubtargetEvent::Attached {
                tab: 1,
                parent_session_id: None,
                session_id: "s-live".to_owned(),
                target_type: "worker".to_owned(),
                waiting_for_debugger: true,
            }]
        );

        // Active parent teardown without a separate child `Detached` notification must also
        // cascade-remove tracked descendants so their `-32001` replies resolve immediately.
        let mut parents = BTreeMap::from([
            ("s-oopif".to_owned(), None),
            ("s-nested-worker".to_owned(), Some("s-oopif".to_owned())),
            ("s-grandchild".to_owned(), Some("s-nested-worker".to_owned())),
        ]);
        let mut children = BTreeSet::from([
            "s-oopif".to_owned(),
            "s-nested-worker".to_owned(),
            "s-grandchild".to_owned(),
        ]);
        mail.expect_id(5);
        mail.record_response(5, CdpReply::SessionNotFound);
        assert_eq!(mail.resolve_single(5, !parents.contains_key("s-grandchild")), None);
        assert!(remove_session_tree(&mut parents, &mut children, "s-oopif"));
        assert!(parents.is_empty());
        assert!(children.is_empty());
        assert_eq!(
            mail.resolve_single(5, !parents.contains_key("s-grandchild")),
            Some(true)
        );

        // Untracked/late replies must be discarded instead of accumulating in `results`.
        mail.record_response(99, CdpReply::Rejected);
        assert!(mail.results.is_empty());

        // `-32001` (`SessionNotFound`) stays pending across multiple pumps until `Detached`
        // arrives, then resolves as cancelled (`true`) once `target_gone` flips true.
        mail.expect_id(7);
        mail.record_response(7, CdpReply::SessionNotFound);
        assert_eq!(mail.resolve_single(7, false), None);
        assert_eq!(mail.resolve_single(7, false), None);
        assert_eq!(mail.resolve_single(7, true), Some(true));
        assert!(mail.results.is_empty());
        assert!(mail.expected.is_empty());

        // A live target's non-detach error response (`Rejected`, e.g. `-32000`) immediately reports `false`.
        mail.expect_id(8);
        mail.record_response(8, CdpReply::Rejected);
        assert_eq!(mail.resolve_single(8, false), Some(false));
    }
}

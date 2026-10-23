//! `IMsRdpClientAdvancedSettings` slots whose mstscax behavior IronRDP reproduces outside the
//! mapped property table: slots mstscax keeps as deprecated no-ops, the connection and idle
//! timeouts, and properties IronRDP honors only at the value it always uses.
//!
//! Slot numbers in trace markers and comments are vtable positions after `IDispatch`, as in the
//! advanced-settings vtable table. This is a child module of `control`, which owns the settings
//! objects and the session worker.

use core::ffi::c_void;
use core::future::pending;
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use ironrdp_client::output_channel::OutputEventReceiver;
use ironrdp_client::rdp::{RdpInputSender, RdpOutputEvent};
use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, S_FALSE, S_OK, VARIANT_FALSE, VARIANT_TRUE};
use windows::core::HRESULT;

use super::{
    AdvancedSettingsObject, DisconnectInfo, EXTENDED_DISCONNECT_REASON_NO_INFO, string_from_bstr, trace_host_call,
    write_bstr, write_out,
};
use crate::mstsc::{Bstr, BstrOut};

/// `IMsTscAxEvents::OnIdleTimeoutNotification`.
pub(super) const DISPID_ON_IDLE_TIMEOUT_NOTIFICATION: i32 = 13;

/// `disconnectReasonTimeout`, which mstscax reports when its overall connection timer expires.
const DISCONNECT_REASON_CONNECTION_TIMEOUT: i32 = 0x108;

// mstscax validates these properties with a `[0, max]` range in its core property table and
// rejects other values with E_INVALIDARG. Its default overall timeout exceeds the settable maximum.
const MAX_CONNECTION_TIMEOUT_SECONDS: u32 = 600;
const DEFAULT_OVERALL_CONNECTION_TIMEOUT_SECONDS: u32 = 900;
const DEFAULT_SINGLE_CONNECTION_TIMEOUT_SECONDS: u32 = 8;
const DEFAULT_SHUTDOWN_TIMEOUT_SECONDS: i32 = 5;
const MAX_MINUTES_TO_IDLE_TIMEOUT: u32 = 1_440;

/// mstscax samples idle time from a one-minute timer, so a notification arrives within a minute
/// after the configured idle time elapses.
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(60);

/// The Kerberos service class IronRDP always uses for the server's service principal name.
const AUTHENTICATION_SERVICE_CLASS: &str = "TERMSRV";

/// Advanced settings that configure the worker's timers and observe the running session.
#[derive(Debug)]
pub(super) struct AdvancedSettingsCompat {
    overall_connection_timeout_seconds: u32,
    single_connection_timeout_seconds: u32,
    idle_timeout: Arc<IdleTimeout>,
    session: Option<RdpInputSender>,
}

impl Default for AdvancedSettingsCompat {
    fn default() -> Self {
        Self {
            overall_connection_timeout_seconds: DEFAULT_OVERALL_CONNECTION_TIMEOUT_SECONDS,
            single_connection_timeout_seconds: DEFAULT_SINGLE_CONNECTION_TIMEOUT_SECONDS,
            idle_timeout: Arc::default(),
            session: None,
        }
    }
}

impl AdvancedSettingsCompat {
    /// Time allowed for a connection attempt to become an active session, or `None` when 0 disables the limit.
    pub(super) fn overall_connection_timeout(&self) -> Option<Duration> {
        nonzero_seconds(self.overall_connection_timeout_seconds)
    }

    /// Time allowed for each resolved server address to accept the TCP connection, or `None` when 0 disables
    /// the limit.
    pub(super) fn single_connection_timeout(&self) -> Option<Duration> {
        nonzero_seconds(self.single_connection_timeout_seconds)
    }

    pub(super) fn idle_timeout(&self) -> Arc<IdleTimeout> {
        Arc::clone(&self.idle_timeout)
    }

    /// Records the input sender of the session the control currently runs, or `None` once it stops.
    pub(super) fn set_session(&mut self, session: Option<RdpInputSender>) {
        self.session = session;
    }

    fn can_auto_reconnect(&self, enable_auto_reconnect: bool) -> bool {
        enable_auto_reconnect
            && self
                .session
                .as_ref()
                .is_some_and(RdpInputSender::auto_reconnect_cookie_available)
    }
}

/// `MinutesToIdleTimeout`, shared between the settings object and the session worker.
#[derive(Debug, Default)]
pub(super) struct IdleTimeout {
    minutes: AtomicU32,
    reset_at: Mutex<Option<Instant>>,
}

impl IdleTimeout {
    /// Sets the idle time and restarts idle measurement, re-arming a notification already sent.
    fn set_minutes(&self, minutes: u32, now: Instant) {
        self.minutes.store(minutes, Ordering::Release);
        *self.reset_at.lock().unwrap_or_else(PoisonError::into_inner) = Some(now);
    }

    fn minutes(&self) -> u32 {
        self.minutes.load(Ordering::Acquire)
    }

    fn reset_at(&self) -> Option<Instant> {
        *self.reset_at.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Decides when a connected session has been idle for `MinutesToIdleTimeout`.
///
/// Like mstscax, the notification is sent once; only setting `MinutesToIdleTimeout` again
/// re-arms it. Input sent to the session restarts idle measurement but does not re-arm it.
#[derive(Debug)]
struct IdleTracker {
    connected_at: Instant,
    reset_seen: Option<Instant>,
    notified: bool,
}

impl IdleTracker {
    fn new(idle_timeout: &IdleTimeout, connected_at: Instant) -> Self {
        Self {
            connected_at,
            reset_seen: idle_timeout.reset_at(),
            notified: false,
        }
    }

    fn expired(&mut self, idle_timeout: &IdleTimeout, last_user_input: Option<Instant>, now: Instant) -> bool {
        let reset_at = idle_timeout.reset_at();
        if reset_at != self.reset_seen {
            self.reset_seen = reset_at;
            self.notified = false;
        }
        let minutes = idle_timeout.minutes();
        if minutes == 0 || self.notified {
            return false;
        }
        let idle_since = reset_at
            .into_iter()
            .chain(last_user_input)
            .fold(self.connected_at, Ord::max);
        if now.saturating_duration_since(idle_since) < Duration::from_secs(u64::from(minutes) * 60) {
            return false;
        }
        self.notified = true;
        true
    }
}

/// An item the session worker receives from its client or its timers.
pub(super) enum WorkerOutput {
    Rdp(RdpOutputEvent),
    /// The connection did not become an active session within `overallConnectionTimeout`.
    /// The client has been asked to close.
    ConnectionTimedOut,
    /// The session has been idle for `MinutesToIdleTimeout`.
    IdleTimeout,
}

/// Runs the overall connection timer and the idle timer alongside the client's output events.
pub(super) struct WorkerTimers {
    input_sender: RdpInputSender,
    connection_deadline: Option<tokio::time::Instant>,
    idle_timeout: Arc<IdleTimeout>,
    idle: Option<(IdleTracker, tokio::time::Interval)>,
    idle_poll_interval: Duration,
}

fn nonzero_seconds(seconds: u32) -> Option<Duration> {
    (seconds != 0).then(|| Duration::from_secs(u64::from(seconds)))
}

impl WorkerTimers {
    pub(super) fn new(
        input_sender: RdpInputSender,
        overall_connection_timeout: Option<Duration>,
        idle_timeout: Arc<IdleTimeout>,
    ) -> Self {
        Self::with_idle_poll_interval(
            input_sender,
            overall_connection_timeout,
            idle_timeout,
            IDLE_POLL_INTERVAL,
        )
    }

    fn with_idle_poll_interval(
        input_sender: RdpInputSender,
        overall_connection_timeout: Option<Duration>,
        idle_timeout: Arc<IdleTimeout>,
        idle_poll_interval: Duration,
    ) -> Self {
        Self {
            input_sender,
            connection_deadline: overall_connection_timeout
                .and_then(|timeout| tokio::time::Instant::now().checked_add(timeout)),
            idle_timeout,
            idle: None,
            idle_poll_interval,
        }
    }

    /// Returns the next client output event or timer expiry, or `None` once the client stops.
    pub(super) async fn next(&mut self, receiver: &mut OutputEventReceiver) -> Option<WorkerOutput> {
        loop {
            let connection_deadline = self.connection_deadline;
            tokio::select! {
                biased;
                output = receiver.recv() => {
                    let output = output?;
                    self.observe(&output);
                    return Some(WorkerOutput::Rdp(output));
                }
                () = sleep_until(connection_deadline) => {
                    self.connection_deadline = None;
                    self.input_sender.request_close();
                    return Some(WorkerOutput::ConnectionTimedOut);
                }
                () = tick(self.idle.as_mut().map(|(_, poll)| poll)) => {
                    let last_user_input = self.input_sender.last_user_input_sent();
                    if let Some((tracker, _)) = self.idle.as_mut()
                        && tracker.expired(&self.idle_timeout, last_user_input, Instant::now())
                    {
                        return Some(WorkerOutput::IdleTimeout);
                    }
                }
            }
        }
    }

    fn observe(&mut self, output: &RdpOutputEvent) {
        match output {
            RdpOutputEvent::Connected | RdpOutputEvent::AutoReconnected => {
                self.connection_deadline = None;
                let start = tokio::time::Instant::now() + self.idle_poll_interval;
                let mut poll = tokio::time::interval_at(start, self.idle_poll_interval);
                poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                self.idle = Some((IdleTracker::new(&self.idle_timeout, Instant::now()), poll));
            }
            // The user cannot send input while the session reconnects, so idle time is not measured.
            RdpOutputEvent::AutoReconnecting { .. } => self.idle = None,
            RdpOutputEvent::ConnectionFailure(_) | RdpOutputEvent::Terminated(_) => {
                self.connection_deadline = None;
                self.idle = None;
            }
            _ => {}
        }
    }
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => pending().await,
    }
}

async fn tick(interval: Option<&mut tokio::time::Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => pending().await,
    }
}

/// The disconnect reported when `overallConnectionTimeout` expires.
pub(super) const fn connection_timeout_disconnect() -> DisconnectInfo {
    DisconnectInfo {
        event_reason: DISCONNECT_REASON_CONNECTION_TIMEOUT,
        extended_reason: EXTENDED_DISCONNECT_REASON_NO_INFO,
        description: "The connection attempt timed out.",
    }
}

// mstscax points each of these slots at one folded `mov eax, 1; ret` body: the call succeeds with
// S_FALSE and the argument is ignored. Its getters leave the out-parameter untouched; these write
// zero or NULL instead, so a caller that treats S_FALSE as success never reads an uninitialized value.

macro_rules! deprecated_put {
    ($(($slot:literal, $name:ident)),+ $(,)?) => {
        $(
            // `usize` keeps the stdcall argument size of every property type these slots take.
            pub(super) unsafe extern "system" fn $name(_this: *mut c_void, _value: usize) -> HRESULT {
                trace_host_call(concat!("S_FALSE:AdvancedSettings::slot_", stringify!($slot)));
                S_FALSE
            }
        )+
    };
}

macro_rules! deprecated_get {
    ($(($slot:literal, $name:ident, $value:ty, $zero:expr)),+ $(,)?) => {
        $(
            pub(super) unsafe extern "system" fn $name(_this: *mut c_void, value: *mut $value) -> HRESULT {
                if !value.is_null() {
                    unsafe { value.write($zero) };
                }
                trace_host_call(concat!("S_FALSE:AdvancedSettings::slot_", stringify!($slot)));
                S_FALSE
            }
        )+
    };
}

deprecated_put!(
    (14, advanced_put_smooth_scroll),
    (18, advanced_put_shadow_bitmap),
    (20, advanced_put_transport_type),
    (26, advanced_put_dedicated_terminal),
    (54, advanced_put_order_draw_threshold),
    (60, advanced_put_scale_bitmap_caches_by_bpp),
    (62, advanced_put_num_bitmap_caches),
    (66, advanced_put_persist_cache_directory),
    (67, advanced_put_brush_support_level),
    (71, advanced_put_input_events_at_once),
    (73, advanced_put_max_event_count),
    (99, advanced_put_rdpdr_local_printing_doc_name),
    (101, advanced_put_rdpdr_clip_clean_temp_dir_string),
    (103, advanced_put_rdpdr_clip_paste_info_string),
    (183, advanced_put_redirect_directx),
);

deprecated_get!(
    (15, advanced_get_smooth_scroll, i32, 0),
    (19, advanced_get_shadow_bitmap, i32, 0),
    (21, advanced_get_transport_type, i32, 0),
    (27, advanced_get_dedicated_terminal, i32, 0),
    (55, advanced_get_order_draw_threshold, i32, 0),
    (61, advanced_get_scale_bitmap_caches_by_bpp, i32, 0),
    (63, advanced_get_num_bitmap_caches, i32, 0),
    (68, advanced_get_brush_support_level, i32, 0),
    (72, advanced_get_input_events_at_once, i32, 0),
    (74, advanced_get_max_event_count, i32, 0),
    (100, advanced_get_rdpdr_local_printing_doc_name, Bstr, ptr::null()),
    (102, advanced_get_rdpdr_clip_clean_temp_dir_string, Bstr, ptr::null()),
    (104, advanced_get_rdpdr_clip_paste_info_string, Bstr, ptr::null()),
);

fn hresult(result: windows::core::Result<()>) -> HRESULT {
    result.map_or_else(|error| error.code(), |()| S_OK)
}

/// # Safety
///
/// `this` must be the advanced-settings object COM passed to the vtable entry.
unsafe fn settings<'a>(this: *mut c_void) -> &'a AdvancedSettingsObject {
    unsafe { &*(this.cast::<AdvancedSettingsObject>()) }
}

/// mstscax stores these timeouts as unsigned properties, so a negative LONG is out of range.
fn connection_timeout_seconds(value: i32) -> Option<u32> {
    let value = u32::from_ne_bytes(value.to_ne_bytes());
    (value <= MAX_CONNECTION_TIMEOUT_SECONDS).then_some(value)
}

fn seconds_as_long(seconds: u32) -> i32 {
    i32::try_from(seconds).unwrap_or(i32::MAX)
}

/// Defines the put and get accessors of a connection timeout stored in whole seconds.
macro_rules! connection_timeout_accessors {
    ($(($put:ident, $get:ident, $field:ident)),+ $(,)?) => {
        $(
            pub(super) unsafe extern "system" fn $put(this: *mut c_void, value: i32) -> HRESULT {
                let Some(seconds) = connection_timeout_seconds(value) else {
                    return E_INVALIDARG;
                };
                unsafe { settings(this) }.settings.borrow_mut().advanced_compat.$field = seconds;
                S_OK
            }

            pub(super) unsafe extern "system" fn $get(this: *mut c_void, value: *mut i32) -> HRESULT {
                let seconds = unsafe { settings(this) }.settings.borrow().advanced_compat.$field;
                hresult(write_out(value, seconds_as_long(seconds)))
            }
        )+
    };
}

connection_timeout_accessors!(
    (
        advanced_put_overall_connection_timeout,
        advanced_get_overall_connection_timeout,
        overall_connection_timeout_seconds
    ),
    (
        advanced_put_single_connection_timeout,
        advanced_get_single_connection_timeout,
        single_connection_timeout_seconds
    ),
);

// mstscax applies `shutdownTimeout` only after `RequestClose` sends a Shutdown Request PDU and
// waits for the server's answer. IronRDP's `RequestClose` asks the host through OnConfirmClose
// and never starts that exchange, and `Disconnect` closes immediately in both clients.

pub(super) unsafe extern "system" fn advanced_put_shutdown_timeout(_this: *mut c_void, _value: i32) -> HRESULT {
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_77");
    E_NOTIMPL
}

pub(super) unsafe extern "system" fn advanced_get_shutdown_timeout(_this: *mut c_void, value: *mut i32) -> HRESULT {
    if let Err(error) = write_out(value, DEFAULT_SHUTDOWN_TIMEOUT_SECONDS) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_78");
    E_NOTIMPL
}

pub(super) unsafe extern "system" fn advanced_put_minutes_to_idle_timeout(this: *mut c_void, value: i32) -> HRESULT {
    let minutes = u32::from_ne_bytes(value.to_ne_bytes());
    if MAX_MINUTES_TO_IDLE_TIMEOUT < minutes {
        return E_INVALIDARG;
    }
    unsafe { settings(this) }
        .settings
        .borrow()
        .advanced_compat
        .idle_timeout
        .set_minutes(minutes, Instant::now());
    S_OK
}

pub(super) unsafe extern "system" fn advanced_get_minutes_to_idle_timeout(
    this: *mut c_void,
    value: *mut i32,
) -> HRESULT {
    let minutes = unsafe { settings(this) }
        .settings
        .borrow()
        .advanced_compat
        .idle_timeout
        .minutes();
    hresult(write_out(value, seconds_as_long(minutes)))
}

pub(super) unsafe extern "system" fn advanced_get_can_auto_reconnect(this: *mut c_void, value: *mut i16) -> HRESULT {
    if value.is_null() {
        return E_INVALIDARG;
    }
    let settings = unsafe { settings(this) }.settings.borrow();
    let can_auto_reconnect = settings
        .advanced_compat
        .can_auto_reconnect(settings.enable_auto_reconnect);
    hresult(write_out(
        value,
        if can_auto_reconnect {
            VARIANT_TRUE.0
        } else {
            VARIANT_FALSE.0
        },
    ))
}

// IronRDP always sets INFO_MAXIMIZESHELL in the Client Info PDU and always encrypts the session,
// which are mstscax's defaults. These setters accept only those values. mstscax treats any nonzero
// LONG as TRUE and its getters report 1 or 0, not VARIANT_TRUE.

pub(super) unsafe extern "system" fn advanced_put_maximize_shell(_this: *mut c_void, value: i32) -> HRESULT {
    if value != 0 {
        return S_OK;
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_38");
    E_NOTIMPL
}

pub(super) unsafe extern "system" fn advanced_get_maximize_shell(_this: *mut c_void, value: *mut i32) -> HRESULT {
    hresult(write_out(value, 1))
}

pub(super) unsafe extern "system" fn advanced_put_encryption_enabled(_this: *mut c_void, value: i32) -> HRESULT {
    if value != 0 {
        return S_OK;
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::slot_24");
    E_NOTIMPL
}

pub(super) unsafe extern "system" fn advanced_get_encryption_enabled(_this: *mut c_void, value: *mut i32) -> HRESULT {
    hresult(write_out(value, 1))
}

pub(super) unsafe extern "system" fn advanced_get_authentication_service_class(
    _this: *mut c_void,
    value: BstrOut,
) -> HRESULT {
    hresult(write_bstr(value, AUTHENTICATION_SERVICE_CLASS))
}

pub(super) unsafe extern "system" fn advanced_put_authentication_service_class(
    _this: *mut c_void,
    value: Bstr,
) -> HRESULT {
    match string_from_bstr(value) {
        Ok(value) if value.eq_ignore_ascii_case(AUTHENTICATION_SERVICE_CLASS) => S_OK,
        Ok(_) => {
            trace_host_call("E_NOTIMPL:AdvancedSettings::slot_159");
            E_NOTIMPL
        }
        Err(error) => error.code(),
    }
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use std::rc::Rc;

    use ironrdp_client::output_channel::output_channel;
    use windows::Win32::Foundation::E_POINTER;
    use windows::core::BSTR;

    use super::*;
    use crate::control::{CompatibilitySettings, CompatibilitySettingsObject, advanced_vtable};

    fn settings_object(settings: &Rc<RefCell<CompatibilitySettings>>) -> AdvancedSettingsObject {
        CompatibilitySettingsObject {
            vtable: advanced_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(settings),
            native_mstsc_credential_bridge: None,
            remote_program_bridge: None,
            server_object: false,
        }
    }

    #[test]
    fn deprecated_slots_succeed_with_s_false_and_clear_outputs() {
        // oracle: reference mstscax.dll 10.0.26100.9711 AdvancedSettings vtable at 0x180685760
        // Each of these slots points at one folded
        // `mov eax, 1; ret` body, so every call returns S_FALSE without touching its output.
        type Put = unsafe extern "system" fn(*mut c_void, usize) -> HRESULT;
        type GetLong = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
        type GetBstr = unsafe extern "system" fn(*mut c_void, BstrOut) -> HRESULT;

        let this = ptr::null_mut();
        let puts: [(&str, Put); 15] = [
            ("put_SmoothScroll", advanced_put_smooth_scroll),
            ("put_ShadowBitmap", advanced_put_shadow_bitmap),
            ("put_TransportType", advanced_put_transport_type),
            ("put_DedicatedTerminal", advanced_put_dedicated_terminal),
            ("put_orderDrawThreshold", advanced_put_order_draw_threshold),
            ("put_ScaleBitmapCachesByBPP", advanced_put_scale_bitmap_caches_by_bpp),
            ("put_NumBitmapCaches", advanced_put_num_bitmap_caches),
            ("put_PersistCacheDirectory", advanced_put_persist_cache_directory),
            ("put_brushSupportLevel", advanced_put_brush_support_level),
            ("put_InputEventsAtOnce", advanced_put_input_events_at_once),
            ("put_maxEventCount", advanced_put_max_event_count),
            (
                "put_RdpdrLocalPrintingDocName",
                advanced_put_rdpdr_local_printing_doc_name,
            ),
            (
                "put_RdpdrClipCleanTempDirString",
                advanced_put_rdpdr_clip_clean_temp_dir_string,
            ),
            (
                "put_RdpdrClipPasteInfoString",
                advanced_put_rdpdr_clip_paste_info_string,
            ),
            ("put_RedirectDirectX", advanced_put_redirect_directx),
        ];
        for (name, put) in puts {
            assert_eq!(unsafe { put(this, 1) }, S_FALSE, "{name}");
        }
        let gets: [(&str, GetLong); 10] = [
            ("get_SmoothScroll", advanced_get_smooth_scroll),
            ("get_ShadowBitmap", advanced_get_shadow_bitmap),
            ("get_TransportType", advanced_get_transport_type),
            ("get_DedicatedTerminal", advanced_get_dedicated_terminal),
            ("get_orderDrawThreshold", advanced_get_order_draw_threshold),
            ("get_ScaleBitmapCachesByBPP", advanced_get_scale_bitmap_caches_by_bpp),
            ("get_NumBitmapCaches", advanced_get_num_bitmap_caches),
            ("get_brushSupportLevel", advanced_get_brush_support_level),
            ("get_InputEventsAtOnce", advanced_get_input_events_at_once),
            ("get_maxEventCount", advanced_get_max_event_count),
        ];
        for (name, get) in gets {
            let mut value = 42;
            assert_eq!(unsafe { get(this, &mut value) }, S_FALSE, "{name}");
            assert_eq!(value, 0, "{name} clears its output");
            assert_eq!(
                unsafe { get(this, ptr::null_mut()) },
                S_FALSE,
                "{name} with a null output"
            );
        }
        let bstr_gets: [(&str, GetBstr); 3] = [
            (
                "get_RdpdrLocalPrintingDocName",
                advanced_get_rdpdr_local_printing_doc_name,
            ),
            (
                "get_RdpdrClipCleanTempDirString",
                advanced_get_rdpdr_clip_clean_temp_dir_string,
            ),
            (
                "get_RdpdrClipPasteInfoString",
                advanced_get_rdpdr_clip_paste_info_string,
            ),
        ];
        for (name, get) in bstr_gets {
            let mut value: Bstr = c"x".as_ptr().cast();
            assert_eq!(unsafe { get(this, &mut value) }, S_FALSE, "{name}");
            assert!(value.is_null(), "{name} clears its output");
        }
    }

    #[test]
    fn connection_timeouts_use_mstscax_defaults_and_range() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = settings_object(&settings);
        let this = (&raw mut object).cast::<c_void>();

        let mut overall = 0;
        let mut single = 0;
        assert_eq!(
            unsafe { advanced_get_overall_connection_timeout(this, &mut overall) },
            S_OK
        );
        assert_eq!(
            unsafe { advanced_get_single_connection_timeout(this, &mut single) },
            S_OK
        );
        assert_eq!((overall, single), (900, 8));

        assert_eq!(unsafe { advanced_put_overall_connection_timeout(this, 600) }, S_OK);
        assert_eq!(unsafe { advanced_put_single_connection_timeout(this, 0) }, S_OK);
        assert_eq!(
            unsafe { advanced_put_overall_connection_timeout(this, 601) },
            E_INVALIDARG
        );
        assert_eq!(
            unsafe { advanced_put_single_connection_timeout(this, -1) },
            E_INVALIDARG
        );
        assert_eq!(
            settings.borrow().advanced_compat.overall_connection_timeout(),
            Some(Duration::from_secs(600))
        );
        assert_eq!(
            settings.borrow().advanced_compat.single_connection_timeout(),
            None,
            "0 disables the per-address limit"
        );
        assert_eq!(
            unsafe { advanced_get_overall_connection_timeout(this, ptr::null_mut()) },
            E_POINTER
        );
    }

    #[test]
    fn shutdown_timeout_is_reported_as_unsupported() {
        let mut value = 0;
        assert_eq!(unsafe { advanced_put_shutdown_timeout(ptr::null_mut(), 10) }, E_NOTIMPL);
        assert_eq!(
            unsafe { advanced_get_shutdown_timeout(ptr::null_mut(), &mut value) },
            E_NOTIMPL
        );
        assert_eq!(value, 5);
    }

    #[test]
    fn minutes_to_idle_timeout_validates_and_restarts_measurement() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = settings_object(&settings);
        let this = (&raw mut object).cast::<c_void>();

        let mut minutes = -1;
        assert_eq!(
            unsafe { advanced_get_minutes_to_idle_timeout(this, &mut minutes) },
            S_OK
        );
        assert_eq!(minutes, 0);
        assert_eq!(
            unsafe { advanced_put_minutes_to_idle_timeout(this, 1_441) },
            E_INVALIDARG
        );
        assert_eq!(unsafe { advanced_put_minutes_to_idle_timeout(this, -1) }, E_INVALIDARG);
        assert!(settings.borrow().advanced_compat.idle_timeout.reset_at().is_none());

        assert_eq!(unsafe { advanced_put_minutes_to_idle_timeout(this, 1_440) }, S_OK);
        assert_eq!(
            unsafe { advanced_get_minutes_to_idle_timeout(this, &mut minutes) },
            S_OK
        );
        assert_eq!(minutes, 1_440);
        assert!(settings.borrow().advanced_compat.idle_timeout.reset_at().is_some());
    }

    #[test]
    fn idle_tracker_notifies_once_until_the_timeout_is_set_again() {
        let idle_timeout = IdleTimeout::default();
        let connected_at = Instant::now();
        let minute = Duration::from_secs(60);
        let mut tracker = IdleTracker::new(&idle_timeout, connected_at);

        // Zero disables the notification.
        assert!(!tracker.expired(&idle_timeout, None, connected_at + 10 * minute));

        idle_timeout.set_minutes(2, connected_at);
        assert!(!tracker.expired(&idle_timeout, None, connected_at + minute));
        // Input restarts idle measurement.
        let input = connected_at + minute;
        assert!(!tracker.expired(&idle_timeout, Some(input), connected_at + 2 * minute));
        assert!(tracker.expired(&idle_timeout, Some(input), input + 2 * minute));
        // Notified once, even as idle time keeps growing or input arrives.
        assert!(!tracker.expired(&idle_timeout, Some(input), input + 10 * minute));
        let later_input = input + 11 * minute;
        assert!(!tracker.expired(&idle_timeout, Some(later_input), later_input + 3 * minute));

        // Setting the timeout again re-arms the notification and restarts measurement.
        let reset = later_input + 4 * minute;
        idle_timeout.set_minutes(1, reset);
        assert!(!tracker.expired(&idle_timeout, Some(later_input), reset));
        assert!(tracker.expired(&idle_timeout, Some(later_input), reset + minute));
    }

    #[test]
    fn can_auto_reconnect_requires_a_session_and_auto_reconnect() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = settings_object(&settings);
        let this = (&raw mut object).cast::<c_void>();

        let mut value = VARIANT_TRUE.0;
        assert_eq!(unsafe { advanced_get_can_auto_reconnect(this, &mut value) }, S_OK);
        assert_eq!(value, VARIANT_FALSE.0);
        assert_eq!(
            unsafe { advanced_get_can_auto_reconnect(this, ptr::null_mut()) },
            E_INVALIDARG
        );

        // A session without a Server Auto-Reconnect Cookie cannot reconnect automatically.
        let (sender, _receiver) = RdpInputSender::channel(1);
        settings.borrow_mut().advanced_compat.set_session(Some(sender));
        assert!(!settings.borrow().advanced_compat.can_auto_reconnect(true));
        value = VARIANT_TRUE.0;
        assert_eq!(unsafe { advanced_get_can_auto_reconnect(this, &mut value) }, S_OK);
        assert_eq!(value, VARIANT_FALSE.0);
    }

    #[test]
    fn fixed_value_properties_accept_only_the_value_ironrdp_uses() {
        let this = ptr::null_mut();
        let mut value = 0;
        assert_eq!(unsafe { advanced_put_maximize_shell(this, 1) }, S_OK);
        assert_eq!(unsafe { advanced_put_maximize_shell(this, -1) }, S_OK);
        assert_eq!(unsafe { advanced_put_maximize_shell(this, 0) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_get_maximize_shell(this, &mut value) }, S_OK);
        assert_eq!(value, 1);

        value = 0;
        assert_eq!(unsafe { advanced_put_encryption_enabled(this, 1) }, S_OK);
        assert_eq!(unsafe { advanced_put_encryption_enabled(this, 0) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_get_encryption_enabled(this, &mut value) }, S_OK);
        assert_eq!(value, 1);

        let termsrv = BSTR::from("termsrv");
        let http = BSTR::from("HTTP");
        assert_eq!(
            unsafe { advanced_put_authentication_service_class(this, termsrv.as_ptr()) },
            S_OK
        );
        assert_eq!(
            unsafe { advanced_put_authentication_service_class(this, http.as_ptr()) },
            E_NOTIMPL
        );
        let mut class: Bstr = ptr::null();
        assert_eq!(
            unsafe { advanced_get_authentication_service_class(this, &mut class) },
            S_OK
        );
        let class = unsafe { BSTR::from_raw(class) };
        assert_eq!(class.to_string(), "TERMSRV");
    }

    #[test]
    fn worker_timers_close_a_connection_that_exceeds_the_overall_timeout() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (_output_sender, mut output_receiver) = output_channel(1);
            let (sender, _receiver) = RdpInputSender::channel(1);
            let mut timers = WorkerTimers::new(sender, Some(Duration::ZERO), Arc::default());
            assert!(matches!(
                timers.next(&mut output_receiver).await,
                Some(WorkerOutput::ConnectionTimedOut)
            ));
        });
    }

    #[test]
    fn worker_timers_stop_the_connection_timer_and_notify_idle_sessions() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (output_sender, mut output_receiver) = output_channel(4);
            let (sender, _receiver) = RdpInputSender::channel(1);
            let idle_timeout = Arc::new(IdleTimeout::default());
            let mut timers = WorkerTimers::with_idle_poll_interval(
                sender,
                Some(Duration::from_millis(50)),
                Arc::clone(&idle_timeout),
                Duration::from_millis(10),
            );
            output_sender
                .send(RdpOutputEvent::Connected)
                .await
                .expect("output receiver is open");
            assert!(matches!(
                timers.next(&mut output_receiver).await,
                Some(WorkerOutput::Rdp(RdpOutputEvent::Connected))
            ));

            // A zero-minute idle time keeps the session quiet past the original connection deadline.
            let quiet = tokio::time::timeout(Duration::from_millis(100), timers.next(&mut output_receiver)).await;
            assert!(quiet.is_err());

            // A reset in the past makes the session already idle for the configured minute.
            let past = Instant::now()
                .checked_sub(Duration::from_secs(120))
                .expect("monotonic clock covers two minutes");
            idle_timeout.set_minutes(1, past);
            timers.idle.as_mut().expect("idle timer armed").0.connected_at = past;
            assert!(matches!(
                timers.next(&mut output_receiver).await,
                Some(WorkerOutput::IdleTimeout)
            ));

            drop(output_sender);
            assert!(timers.next(&mut output_receiver).await.is_none());
        });
    }

    #[test]
    fn worker_timers_pause_idle_measurement_while_reconnecting() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let (sender, _receiver) = RdpInputSender::channel(1);
            let mut timers = WorkerTimers::new(sender, None, Arc::default());
            assert!(timers.connection_deadline.is_none(), "0 disables the overall timeout");

            timers.observe(&RdpOutputEvent::Connected);
            assert!(timers.idle.is_some());
            let (response, _decision) = tokio::sync::oneshot::channel();
            timers.observe(&RdpOutputEvent::AutoReconnecting {
                disconnect_reason: 0,
                attempt: 1,
                maximum_attempts: 20,
                response,
            });
            assert!(timers.idle.is_none(), "no idle notification while reconnecting");
            timers.observe(&RdpOutputEvent::AutoReconnected);
            assert!(timers.idle.is_some(), "idle measurement restarts after reconnecting");
        });
    }
}

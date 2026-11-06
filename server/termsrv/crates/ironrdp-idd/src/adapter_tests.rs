use core::ffi::c_void;
use core::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};

use windows_core::GUID;

use crate::iddcx::{
    IDARG_IN_ADAPTER_INIT, IDARG_IN_ADAPTERDISPLAYCONFIGUPDATE, IDARG_IN_MONITORCREATE, IDARG_OUT_ADAPTER_INIT,
    IDARG_OUT_GETVERSION, IDARG_OUT_MONITORARRIVAL, IDARG_OUT_MONITORCREATE, IDD_CX_CLIENT_CONFIG, IDD_DRIVER_GLOBALS,
};
use crate::wdf::{
    PFN_WDF_TIMER, WDF_DRIVER_GLOBALS, WDF_FILEOBJECT_CONFIG, WDF_OBJECT_ATTRIBUTES, WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_TIMER_CONFIG, WDFDEVICE, WDFDEVICE_INIT, WDFTIMER,
};
use crate::{NTSTATUS, UNICODE_STRING};

type Dispatch = unsafe extern "system" fn();

unsafe extern "C" {
    static mut WdfFunctions_02033: *const Option<Dispatch>;
    static mut IddFunctions: [Dispatch; 25];
}

static FRAMEWORK_LOCK: Mutex<()> = Mutex::new(());
static LINK_STATUS: AtomicI32 = AtomicI32::new(0);
static LINK_CALLS: AtomicUsize = AtomicUsize::new(0);
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static SESSION_QUERY_STATUS: AtomicI32 = AtomicI32::new(0);
static ADAPTER_INIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static DISPLAY_CALL: (Mutex<(bool, bool)>, Condvar) = (Mutex::new((false, false)), Condvar::new());
static TIMER_CALLBACK: Mutex<Option<PFN_WDF_TIMER>> = Mutex::new(None);
static TIMER_PARENT: AtomicUsize = AtomicUsize::new(0);
static TIMER_SYNCHRONIZATION_SCOPE: AtomicU32 = AtomicU32::new(0);
static TIMER_STOP_WAIT: AtomicI32 = AtomicI32::new(-1);
static DISPLAY_CALLS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn power_exit_retires_display_without_waiting_for_a_native_update() {
    // oracle: mutant waiting for a display update that itself needs the PnP transition to finish.
    assert_eq!(run_device_add(0, 0), (0, true));
    let _lock = FRAMEWORK_LOCK.lock().expect("framework fixture lock");
    *DISPLAY_CALL.0.lock().expect("display fixture lock") = (false, false);
    *TIMER_CALLBACK.lock().expect("timer fixture lock") = None;
    TIMER_PARENT.store(0, Ordering::SeqCst);
    TIMER_SYNCHRONIZATION_SCOPE.store(0, Ordering::SeqCst);
    TIMER_STOP_WAIT.store(-1, Ordering::SeqCst);
    DISPLAY_CALLS.store(0, Ordering::SeqCst);
    let mut table: [Option<Dispatch>; 201] = [None; 201];
    let slots = [
        (2, initialize_adapter_async as *const ()),
        (3, create_monitor as *const ()),
        (4, arrive_monitor as *const ()),
        (6, update_monitor_modes as *const ()),
        (19, get_version as *const ()),
        (24, blocked_display_update as *const ()),
    ];

    // SAFETY: Each replacement matches its native dispatch ABI and stays installed until callbacks finish.
    let (previous_wdf, previous_iddcx) = unsafe {
        table[28] = Some(core::mem::transmute::<*const (), Dispatch>(
            enable_interface as *const (),
        ));
        table[198] = Some(core::mem::transmute::<*const (), Dispatch>(create_timer as *const ()));
        table[199] = Some(core::mem::transmute::<*const (), Dispatch>(start_timer as *const ()));
        table[200] = Some(core::mem::transmute::<*const (), Dispatch>(stop_timer as *const ()));
        let previous_wdf = WdfFunctions_02033;
        WdfFunctions_02033 = table.as_ptr();
        let iddcx = ptr::addr_of_mut!(IddFunctions).cast::<*const ()>();
        let previous = slots.map(|(slot, replacement)| {
            let original = iddcx.add(slot).read();
            iddcx.add(slot).write(replacement);
            original
        });
        crate::adapter::device_d0_entry(ptr::dangling_mut::<c_void>(), 0);
        (previous_wdf, previous)
    };

    let completion = std::thread::spawn(|| {
        let status = crate::adapter::adapter_init_finished(
            ptr::dangling_mut::<c_void>(),
            &crate::adapter::IDARG_IN_ADAPTER_INIT_FINISHED { AdapterInitStatus: 0 },
        );
        let callback = *TIMER_CALLBACK.lock().expect("timer fixture lock");
        if let Some(callback) = callback {
            // SAFETY: Invoke the registered framework callback with the fixture's live timer.
            unsafe { callback(3usize as WDFTIMER) };
        }
        status
    });
    let (state, _) = DISPLAY_CALL
        .1
        .wait_timeout_while(
            DISPLAY_CALL.0.lock().expect("display fixture lock"),
            Duration::from_secs(2),
            |state| !state.0,
        )
        .expect("display call started");
    let entered = state.0;
    drop(state);

    let (sender, receiver) = mpsc::channel();
    let power_exit = std::thread::spawn(move || {
        // SAFETY: The callback's native dispatch remains installed until this thread finishes.
        let status = unsafe { crate::adapter::device_d0_exit(ptr::dangling_mut::<c_void>(), 0) };
        sender.send(status).expect("power exit receiver");
    });
    let exited_before_update = receiver.recv_timeout(Duration::from_secs(2)).is_ok();
    DISPLAY_CALL.0.lock().expect("display fixture lock").1 = true;
    DISPLAY_CALL.1.notify_all();
    completion.join().expect("completion thread");
    power_exit.join().expect("power exit thread");

    let callback = *TIMER_CALLBACK.lock().expect("timer fixture lock");
    if let Some(callback) = callback {
        // SAFETY: A callback already queued when the timer stops may still arrive before cleanup.
        unsafe { callback(3usize as WDFTIMER) };
    }

    // SAFETY: Power exit has closed admission, and every fixture callback has returned.
    unsafe {
        WdfFunctions_02033 = previous_wdf;
        let iddcx = ptr::addr_of_mut!(IddFunctions).cast::<*const ()>();
        for ((slot, _), original) in slots.into_iter().zip(previous_iddcx) {
            iddcx.add(slot).write(original);
        }
    }

    assert!(entered, "the native update was never reached");
    assert!(exited_before_update, "power exit waited for the blocked native update");
    // oracle: spec EVT_WDF_OBJECT_CONTEXT_CLEANUP §Remarks drains timer callbacks before parent cleanup.
    assert_eq!(
        TIMER_PARENT.load(Ordering::SeqCst),
        2,
        "timer must own the monitor lifetime"
    );
    assert_eq!(TIMER_STOP_WAIT.load(Ordering::SeqCst), 0);
    // oracle: spec WDK 2.33 wdfobject.h §WDF_SYNCHRONIZATION_SCOPE defines None as 4, not Queue (3).
    assert_eq!(TIMER_SYNCHRONIZATION_SCOPE.load(Ordering::SeqCst), 4);
    assert_eq!(
        DISPLAY_CALLS.load(Ordering::SeqCst),
        1,
        "late timer callback used a retired adapter"
    );
}

#[test]
fn failed_adapter_completion_is_not_reported_as_success_after_power_exit() {
    // oracle: spec EVT_IDD_CX_ADAPTER_INIT_FINISHED §Return value requires an error for a failed operation.
    let _lock = FRAMEWORK_LOCK.lock().expect("framework fixture lock");
    // SAFETY: Power exit does not dereference the device handle.
    unsafe { crate::adapter::device_d0_exit(ptr::dangling_mut::<c_void>(), 0) };
    let args = crate::adapter::IDARG_IN_ADAPTER_INIT_FINISHED {
        AdapterInitStatus: i32::from_ne_bytes(0xC000_0001_u32.to_ne_bytes()),
    };

    let status = crate::adapter::adapter_init_finished(ptr::dangling_mut::<c_void>(), &args);

    assert!(status < 0, "failed initialization was reported as successful");
}

#[test]
fn power_entry_does_not_wait_for_async_adapter_completion() {
    // oracle: mutant polling for completion from the power callback delays delivery of that completion.
    assert_eq!(run_device_add(0, 0), (0, true));
    let _lock = FRAMEWORK_LOCK.lock().expect("framework fixture lock");
    ADAPTER_INIT_REQUESTED.store(false, Ordering::SeqCst);
    let mut table: [Option<Dispatch>; 29] = [None; 29];

    // SAFETY: The fixture restores each dispatch table before its storage expires.
    // Completion is deliberately withheld until after the D0 entry callback returns.
    let (status, elapsed) = unsafe {
        table[28] = Some(core::mem::transmute::<*const (), Dispatch>(
            enable_interface as *const (),
        ));
        let previous_wdf = WdfFunctions_02033;
        let iddcx = ptr::addr_of_mut!(IddFunctions).cast::<*const ()>();
        let previous_iddcx = [iddcx.add(2).read(), iddcx.add(19).read()];
        WdfFunctions_02033 = table.as_ptr();
        iddcx.add(2).write(initialize_adapter_async as *const ());
        iddcx.add(19).write(get_version as *const ());

        let start = Instant::now();
        let status = crate::adapter::device_d0_entry(ptr::dangling_mut::<c_void>(), 0);
        let elapsed = start.elapsed();
        crate::adapter::device_d0_exit(ptr::dangling_mut::<c_void>(), 0);

        WdfFunctions_02033 = previous_wdf;
        iddcx.add(2).write(previous_iddcx[0]);
        iddcx.add(19).write(previous_iddcx[1]);
        (status, elapsed)
    };

    assert_eq!(status, 0);
    assert!(ADAPTER_INIT_REQUESTED.load(Ordering::SeqCst));
    assert!(
        elapsed < Duration::from_secs(2),
        "power entry waited for asynchronous completion: {elapsed:?}"
    );
}

#[test]
fn device_startup_requires_an_os_session_binding() {
    // oracle: mutant accepting a device without its OS-owned session permits cross-session display requests.
    let (status, initialized) = run_device_add(0, i32::from_ne_bytes(0xC000_0225_u32.to_ne_bytes()));
    assert!(status < 0, "an unbound remote display must not start");
    assert!(!initialized);
}

#[test]
fn existing_umdf_alias_does_not_abort_device_startup() {
    // oracle: reference UMDF reconnect capture, 2026-10-26: 0x800700B7 collisions precede 0x80070003.
    let (status, initialized) = run_device_add(i32::from_ne_bytes(0x8007_00B7_u32.to_ne_bytes()), 0);
    assert_eq!(
        status, 0,
        "a retiring device's alias must not prevent the new device from starting"
    );
    assert!(initialized, "the new device must reach IddCxDeviceInitialize");
}

#[test]
fn existing_ntstatus_alias_does_not_abort_device_startup() {
    // oracle: mutant losing the existing STATUS_OBJECT_NAME_COLLISION handling.
    let (status, initialized) = run_device_add(i32::from_ne_bytes(0xC000_0035_u32.to_ne_bytes()), 0);
    assert_eq!(status, 0);
    assert!(initialized);
}

#[test]
fn created_alias_allows_device_startup() {
    // oracle: mutant requiring a collision rather than allowing a successful alias creation.
    let (status, initialized) = run_device_add(0, 0);
    assert_eq!(status, 0);
    assert!(initialized);
}

#[test]
fn unrelated_alias_errors_still_abort_device_startup() {
    // oracle: mutant accepting every alias error when making collisions nonfatal.
    let (status, initialized) = run_device_add(i32::from_ne_bytes(0xC000_0022_u32.to_ne_bytes()), 0);
    assert!(status < 0);
    assert!(!initialized);
}

fn run_device_add(link_status: NTSTATUS, session_query_status: NTSTATUS) -> (NTSTATUS, bool) {
    let _lock = FRAMEWORK_LOCK.lock().expect("framework fixture lock");
    LINK_STATUS.store(link_status, Ordering::SeqCst);
    LINK_CALLS.store(0, Ordering::SeqCst);
    INITIALIZED.store(false, Ordering::SeqCst);
    SESSION_QUERY_STATUS.store(session_query_status, Ordering::SeqCst);

    let mut table: [Option<Dispatch>; 55] = [None; 55];
    // SAFETY: Each entry has the ABI of its UMDF 2.33 dispatch slot; the wrappers restore its type.
    unsafe {
        table[19] = Some(core::mem::transmute::<*const (), Dispatch>(
            set_power_callbacks as *const (),
        ));
        table[23] = Some(core::mem::transmute::<*const (), Dispatch>(
            set_file_config as *const (),
        ));
        table[25] = Some(core::mem::transmute::<*const (), Dispatch>(create_device as *const ()));
        table[27] = Some(core::mem::transmute::<*const (), Dispatch>(
            create_interface as *const (),
        ));
        table[28] = Some(core::mem::transmute::<*const (), Dispatch>(
            enable_interface as *const (),
        ));
        table[30] = Some(core::mem::transmute::<*const (), Dispatch>(create_link as *const ()));
        table[54] = Some(core::mem::transmute::<*const (), Dispatch>(
            query_session_property as *const (),
        ));
    }

    // SAFETY: The lock serializes the only framework calls in this test executable.
    // Read unbound IddCx slots as raw pointers, since they may contain null rather than a function.
    // The callback cannot unwind, and the tables are restored before their local storage expires.
    let status = unsafe {
        let previous_wdf = WdfFunctions_02033;
        let iddcx = ptr::addr_of_mut!(IddFunctions).cast::<*const ()>();
        let previous_iddcx = [iddcx.read(), iddcx.add(1).read()];
        WdfFunctions_02033 = table.as_ptr();
        iddcx.write(init_config as *const ());
        iddcx.add(1).write(initialize_device as *const ());

        let status = crate::adapter::device_add(ptr::null_mut(), ptr::dangling_mut::<c_void>());

        WdfFunctions_02033 = previous_wdf;
        iddcx.write(previous_iddcx[0]);
        iddcx.add(1).write(previous_iddcx[1]);
        status
    };
    (status, INITIALIZED.load(Ordering::SeqCst))
}

unsafe extern "system" fn set_power_callbacks(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device_init: *mut WDFDEVICE_INIT,
    _callbacks: *const WDF_PNPPOWER_EVENT_CALLBACKS,
) {
}

unsafe extern "system" fn set_file_config(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device_init: *mut WDFDEVICE_INIT,
    _config: *const WDF_FILEOBJECT_CONFIG,
    _attributes: *const WDF_OBJECT_ATTRIBUTES,
) {
}

unsafe extern "system" fn create_device(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device_init: *mut *mut WDFDEVICE_INIT,
    _attributes: *const WDF_OBJECT_ATTRIBUTES,
    device: *mut WDFDEVICE,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies a valid output pointer.
    unsafe { device.write(ptr::dangling_mut::<c_void>()) };
    0
}

unsafe extern "system" fn create_interface(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device: WDFDEVICE,
    _guid: *const GUID,
    _reference: *const UNICODE_STRING,
) -> NTSTATUS {
    0
}

unsafe extern "system" fn enable_interface(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device: WDFDEVICE,
    _guid: *const GUID,
    _reference: *const UNICODE_STRING,
    _enabled: u8,
) {
}

unsafe extern "system" fn create_link(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device: WDFDEVICE,
    _name: *const UNICODE_STRING,
) -> NTSTATUS {
    if LINK_CALLS.fetch_add(1, Ordering::SeqCst) < 2 {
        LINK_STATUS.load(Ordering::SeqCst)
    } else {
        i32::from_ne_bytes(0x8007_0003_u32.to_ne_bytes())
    }
}

unsafe extern "system" fn init_config(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _device_init: *mut WDFDEVICE_INIT,
    _config: *const IDD_CX_CLIENT_CONFIG,
) -> NTSTATUS {
    0
}

unsafe extern "system" fn initialize_device(_globals: *mut IDD_DRIVER_GLOBALS, _device: WDFDEVICE) -> NTSTATUS {
    INITIALIZED.store(true, Ordering::SeqCst);
    0
}

unsafe extern "system" fn initialize_adapter_async(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _input: *const IDARG_IN_ADAPTER_INIT,
    output: *mut IDARG_OUT_ADAPTER_INIT,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies a valid output pointer.
    unsafe { (*output).AdapterObject = ptr::dangling_mut::<c_void>() };
    ADAPTER_INIT_REQUESTED.store(true, Ordering::SeqCst);
    0
}

unsafe extern "system" fn get_version(
    _globals: *mut IDD_DRIVER_GLOBALS,
    output: *mut IDARG_OUT_GETVERSION,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies a valid output pointer.
    unsafe { (*output).IddCxVersion = 0x1A80 };
    0
}

unsafe extern "system" fn create_monitor(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _adapter: *mut c_void,
    _input: *const IDARG_IN_MONITORCREATE,
    output: *mut IDARG_OUT_MONITORCREATE,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies writable output storage.
    unsafe { (*output).MonitorObject = 2usize as *mut c_void };
    0
}

unsafe extern "system" fn arrive_monitor(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _monitor: *mut c_void,
    output: *mut IDARG_OUT_MONITORARRIVAL,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies writable output storage.
    unsafe { (*output).OsTargetId = 0 };
    0
}

unsafe extern "system" fn update_monitor_modes(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _monitor: *mut c_void,
    _input: *const c_void,
) -> NTSTATUS {
    0
}

unsafe extern "system" fn blocked_display_update(
    _globals: *mut IDD_DRIVER_GLOBALS,
    _adapter: *mut c_void,
    _input: *const IDARG_IN_ADAPTERDISPLAYCONFIGUPDATE,
) -> NTSTATUS {
    DISPLAY_CALLS.fetch_add(1, Ordering::SeqCst);
    let mut state = DISPLAY_CALL.0.lock().expect("display fixture lock");
    state.0 = true;
    DISPLAY_CALL.1.notify_all();
    let _ = DISPLAY_CALL
        .1
        .wait_timeout_while(state, Duration::from_secs(8), |state| !state.1)
        .expect("display call release");
    i32::from_ne_bytes(0xC000_02B6_u32.to_ne_bytes())
}

unsafe extern "system" fn create_timer(
    _globals: *mut WDF_DRIVER_GLOBALS,
    config: *const WDF_TIMER_CONFIG,
    attributes: *const WDF_OBJECT_ATTRIBUTES,
    output: *mut WDFTIMER,
) -> NTSTATUS {
    // SAFETY: The production wrapper supplies valid configuration and output storage.
    unsafe {
        *TIMER_CALLBACK.lock().expect("timer fixture lock") = (*config).EvtTimerFunc;
        TIMER_PARENT.store((*attributes).ParentObject as usize, Ordering::SeqCst);
        TIMER_SYNCHRONIZATION_SCOPE.store((*attributes).SynchronizationScope, Ordering::SeqCst);
        output.write(3usize as WDFTIMER);
    }
    0
}

unsafe extern "system" fn start_timer(_globals: *mut WDF_DRIVER_GLOBALS, _timer: WDFTIMER, _due_time: i64) -> u8 {
    0
}

unsafe extern "system" fn stop_timer(_globals: *mut WDF_DRIVER_GLOBALS, _timer: WDFTIMER, wait: u8) -> u8 {
    TIMER_STOP_WAIT.store(i32::from(wait), Ordering::SeqCst);
    1
}

unsafe extern "system" fn query_session_property(
    _globals: *mut WDF_DRIVER_GLOBALS,
    _device: WDFDEVICE,
    _property: *const c_void,
    size: u32,
    buffer: *mut c_void,
    required: *mut u32,
    property_type: *mut u32,
) -> NTSTATUS {
    let status = SESSION_QUERY_STATUS.load(Ordering::SeqCst);
    if status < 0 {
        return status;
    }
    if size != 4 {
        return i32::from_ne_bytes(0xC000_000D_u32.to_ne_bytes());
    }
    // SAFETY: WdfDeviceQueryPropertyEx supplies writable output pointers and a four-byte buffer.
    unsafe {
        buffer.cast::<u32>().write(2);
        required.write(4);
        property_type.write(7);
    }
    0
}

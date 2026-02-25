use crate::{ntstatus_to_u32, IDDCX_ADAPTER, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};

#[repr(C)]
pub(crate) struct IDARG_IN_ADAPTER_INIT_FINISHED {
    _private: [u8; 0],
}

#[repr(C)]
pub(crate) struct IDARG_IN_COMMITMODES {
    _private: [u8; 0],
}

#[derive(Debug, Clone, Copy)]
pub struct IronRdpIddAdapter {
    pub iddcx_adapter: IDDCX_ADAPTER,
    pub is_remote: bool,
}

impl IronRdpIddAdapter {
    pub fn new(iddcx_adapter: IDDCX_ADAPTER, is_remote: bool) -> Self {
        Self {
            iddcx_adapter,
            is_remote,
        }
    }

    pub fn init_async(&self) -> NTSTATUS {
        tracing::info!(is_remote = self.is_remote, "IddCxAdapterInitAsync (stub)");
        STATUS_NOT_SUPPORTED
    }
}

pub(crate) extern "system" fn adapter_init_finished(
    _adapter: IDDCX_ADAPTER,
    _args: *const IDARG_IN_ADAPTER_INIT_FINISHED,
) -> NTSTATUS {
    tracing::info!("EvtIddCxAdapterInitFinished (stub)");
    STATUS_SUCCESS
}

pub(crate) extern "system" fn adapter_commit_modes(
    _adapter: IDDCX_ADAPTER,
    _args: *const IDARG_IN_COMMITMODES,
) -> NTSTATUS {
    tracing::info!("EvtIddCxAdapterCommitModes (stub)");
    STATUS_SUCCESS
}

// ──────────────── EvtDriverDeviceAdd — real IddCx device initialization ──────────────────────

#[cfg(ironrdp_idd_link)]
pub(crate) unsafe extern "system" fn device_add(
    _driver: crate::wdf::WDFDRIVER,
    device_init: *mut crate::wdf::WDFDEVICE_INIT,
) -> NTSTATUS {
    use crate::iddcx::{
        ENDPOINT_MANUFACTURER_UTF16, ENDPOINT_MODEL_NAME_UTF16, IDARG_IN_ADAPTER_INIT,
        IDARG_OUT_ADAPTER_INIT, IDD_CX_CLIENT_CONFIG, IDDCX_ADAPTER_CAPS,
        IDDCX_ENDPOINT_DIAGNOSTIC_INFO, IDDCX_ENDPOINT_VERSION,
    };
    use crate::wdf::WDF_OBJECT_ATTRIBUTES;
    use core::mem::{size_of, transmute};

    tracing::info!("EvtDriverDeviceAdd started");

    // ── 1. Build IDD_CX_CLIENT_CONFIG ────────────────────────────────────────────────────────
    // Zero-initialize then populate mandatory callbacks.
    // SAFETY: IDD_CX_CLIENT_CONFIG is a POD struct of function pointers — zero-initialization
    // produces all-None (null) entries, which is valid C ABI for unused callbacks.
    let mut config: IDD_CX_CLIENT_CONFIG = unsafe { core::mem::zeroed() };
    config.Size = size_of::<IDD_CX_CLIENT_CONFIG>() as u32;
    tracing::info!(idd_client_config_size = config.Size, "Prepared IDD_CX_CLIENT_CONFIG");

    // Assign mandatory callbacks by transmuting typed fn pointers → Option<unsafe extern "system" fn()>.
    // SAFETY: all function pointer types share the same calling convention and pointer size.
    config.EvtIddCxAdapterInitFinished =
        Some(unsafe { transmute::<_, unsafe extern "system" fn()>(adapter_init_finished as usize) });
    config.EvtIddCxAdapterCommitModes =
        Some(unsafe { transmute::<_, unsafe extern "system" fn()>(adapter_commit_modes as usize) });
    config.EvtIddCxParseMonitorDescription = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::parse_monitor_description as usize)
    });
    config.EvtIddCxMonitorGetDefaultDescriptionModes = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(
            crate::monitor::monitor_get_default_description_modes as usize,
        )
    });
    config.EvtIddCxMonitorQueryTargetModes = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_query_target_modes as usize)
    });
    config.EvtIddCxMonitorAssignSwapChain = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_assign_swapchain as usize)
    });
    config.EvtIddCxMonitorUnassignSwapChain = Some(unsafe {
        transmute::<_, unsafe extern "system" fn()>(crate::monitor::monitor_unassign_swapchain as usize)
    });

    // ── 2. IddCxDeviceInitConfig ─────────────────────────────────────────────────────────────
    // SAFETY: device_init is a valid PWDFDEVICE_INIT provided by WDF via EvtDriverDeviceAdd.
    let status = unsafe { crate::iddcx::device_init_config(device_init, &config) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxDeviceInitConfig failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        "IddCxDeviceInitConfig succeeded"
    );

    // ── 3. WdfDeviceCreate ───────────────────────────────────────────────────────────────────
    let attribs = WDF_OBJECT_ATTRIBUTES::init_no_context();
    let mut device_init_ptr = device_init;
    // SAFETY: device_init_ptr is a valid PWDFDEVICE_INIT that has been configured above.
    let (status, device) = unsafe { crate::wdf::device_create(&mut device_init_ptr, &attribs) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "WdfDeviceCreate failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        device = ?device,
        "WdfDeviceCreate succeeded"
    );

    // ── 4. IddCxDeviceInitialize ─────────────────────────────────────────────────────────────
    // SAFETY: device is a valid WDFDEVICE handle created by WdfDeviceCreate.
    let status = unsafe { crate::iddcx::device_initialize(device) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxDeviceInitialize failed"
        );
        return status;
    }
    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        "IddCxDeviceInitialize succeeded"
    );

    // ── 5. Build IDDCX_ADAPTER_CAPS ──────────────────────────────────────────────────────────
    let endpoint_version = IDDCX_ENDPOINT_VERSION {
        Size: size_of::<IDDCX_ENDPOINT_VERSION>() as u32,
        MajorVer: 1,
        MinorVer: 0,
        Build: 0,
        SKU: 0,
    };

    // IDDCX_TRANSMISSION_TYPE_OTHER = 0xFFFFFFFF (IddCx 1.2)
    let diag = IDDCX_ENDPOINT_DIAGNOSTIC_INFO {
        Size: size_of::<IDDCX_ENDPOINT_DIAGNOSTIC_INFO>() as u32,
        TransmissionType: 0xFFFF_FFFF,
        pEndPointFriendlyName: core::ptr::null(),
        pEndPointModelName: ENDPOINT_MODEL_NAME_UTF16.as_ptr(),
        pEndPointManufacturerName: ENDPOINT_MANUFACTURER_UTF16.as_ptr(),
        pHardwareVersion: &endpoint_version,
        pFirmwareVersion: &endpoint_version,
        // IDDCX_FEATURE_IMPLEMENTATION_NONE = 1
        GammaSupport: 1,
        _pad: 0,
    };

    // IDDCX_ADAPTER_FLAGS_REMOTE_SESSION_DRIVER(4) | IDDCX_ADAPTER_FLAGS_USE_SMALLEST_MODE(1)
    let mut caps = IDDCX_ADAPTER_CAPS {
        Size: size_of::<IDDCX_ADAPTER_CAPS>() as u32,
        Flags: 4 | 1,
        MaxDisplayPipelineRate: 0,
        MaxMonitorsSupported: 1,
        _pad: 0,
        EndPointDiagnostics: diag,
        StaticDesktopReencodeFrameCount: 0,
        _pad2: 0,
    };
    tracing::info!(
        adapter_caps_size = caps.Size,
        transmission_type = caps.EndPointDiagnostics.TransmissionType,
        max_monitors = caps.MaxMonitorsSupported,
        "Prepared IDDCX_ADAPTER_CAPS"
    );

    // ── 6. IddCxAdapterInitAsync ─────────────────────────────────────────────────────────────
    let in_args = IDARG_IN_ADAPTER_INIT {
        WdfDevice: device,
        pCaps: &mut caps,
        ObjectAttributes: core::ptr::null(),
    };
    let mut out_args = IDARG_OUT_ADAPTER_INIT {
        AdapterObject: core::ptr::null_mut(),
    };
    // SAFETY: in_args and out_args are valid, stack-allocated structures.
    let status = unsafe { crate::iddcx::adapter_init_async(&in_args, &mut out_args) };
    if status < 0 {
        tracing::error!(
            status,
            status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
            "IddCxAdapterInitAsync failed"
        );
        return status;
    }

    tracing::info!(
        status,
        status_hex = format_args!("0x{:08X}", ntstatus_to_u32(status)),
        adapter_object = ?out_args.AdapterObject,
        "IddCxAdapterInitAsync succeeded"
    );

    tracing::info!("EvtDriverDeviceAdd: IDD device and adapter initialized successfully");
    STATUS_SUCCESS
}

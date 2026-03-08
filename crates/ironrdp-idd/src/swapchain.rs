use crate::{IDDCX_SWAPCHAIN, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::io::Write as _;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows::core::w;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, HMODULE, LUID, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    ID3D11Resource, ID3D11Texture2D,
};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIResource};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, IDXGIAdapter1, IDXGIFactory5, DXGI_CREATE_FACTORY_FLAGS};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW, SetEvent, WaitForMultipleObjects,
    INFINITE,
};

#[cfg(ironrdp_idd_link)]
use crate::iddcx;

#[cfg(ironrdp_idd_link)]
use windows_core::Interface as _;

#[cfg(ironrdp_idd_link)]
const E_PENDING: windows_core::HRESULT = windows_core::HRESULT(-2147483638);

#[derive(Debug, Clone, Copy)]
struct SendHandle(HANDLE);

// SAFETY: Win32 handles are process-wide and may be used from any thread within the same process.
unsafe impl Send for SendHandle {}

#[derive(Debug, Clone, Copy)]
struct SendSwapChain(IDDCX_SWAPCHAIN);

// SAFETY: swapchain handles are opaque identifiers; they are intended to be used from a swapchain processing thread.
unsafe impl Send for SendSwapChain {}

#[cfg(ironrdp_idd_link)]
#[derive(Clone)]
struct SendD3dDevice(ID3D11Device);

#[cfg(ironrdp_idd_link)]
// SAFETY: D3D11 devices are free-threaded COM objects; we only use it to obtain the underlying IDXGIDevice pointer.
unsafe impl Send for SendD3dDevice {}

#[cfg(ironrdp_idd_link)]
#[derive(Clone)]
struct SendD3dDeviceContext(ID3D11DeviceContext);

#[cfg(ironrdp_idd_link)]
// SAFETY: D3D11 immediate contexts are COM objects used on the dedicated swapchain worker thread.
unsafe impl Send for SendD3dDeviceContext {}

#[cfg(ironrdp_idd_link)]
impl SendSwapChain {
    #[must_use]
    fn raw(&self) -> IDDCX_SWAPCHAIN {
        self.0
    }
}

#[cfg(ironrdp_idd_link)]
impl SendD3dDevice {
    #[must_use]
    fn device(&self) -> &ID3D11Device {
        &self.0
    }
}

#[cfg(ironrdp_idd_link)]
impl SendD3dDeviceContext {
    #[must_use]
    fn context(&self) -> &ID3D11DeviceContext {
        &self.0
    }
}

impl SendHandle {
    #[must_use]
    fn raw(&self) -> HANDLE {
        self.0
    }
}

#[derive(Debug)]
struct OwnedHandle(HANDLE);

// SAFETY: Win32 handles are process-wide and may be used from any thread within the same process.
unsafe impl Send for OwnedHandle {}
// SAFETY: Win32 handles are process-wide and may be used from any thread within the same process.
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    #[must_use]
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.0.is_invalid() {
            return;
        }

        // SAFETY: `self.0` is a Win32 handle created by this crate.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct MmcssGuard(HANDLE);

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        if self.0.is_invalid() {
            return;
        }

        // SAFETY: `self.0` is the handle returned by `AvSetMmThreadCharacteristicsW`.
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(self.0);
        }
    }
}

const IDD_DUMP_INTERVAL_MS: u64 = 2_000;
const IDD_DUMP_MAX_COUNT: u64 = 60;

static IDD_DUMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IDD_DUMP_LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IDD_DUMP_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static IDD_DUMP_ENABLED_LOGGED: AtomicBool = AtomicBool::new(false);
static IDD_DUMP_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);
static IDD_FIRST_FRAME_SESSION: AtomicU32 = AtomicU32::new(0);

fn now_unix_ms_best_effort() -> Option<u64> {
    let now = std::time::SystemTime::now();
    let dur = now.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn write_bmp_bgra32(path: &std::path::Path, width: usize, height: usize, pixels: &[u8]) -> Result<(), String> {
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| "bmp stride overflow".to_owned())?;
    let expected = stride
        .checked_mul(height)
        .ok_or_else(|| "bmp payload length overflow".to_owned())?;

    if pixels.len() != expected {
        return Err(format!(
            "bmp payload size mismatch: expected={expected} actual={}",
            pixels.len()
        ));
    }

    let width_i32 = i32::try_from(width).map_err(|_| "bmp width out of range".to_owned())?;
    let height_i32 = i32::try_from(height).map_err(|_| "bmp height out of range".to_owned())?;
    let top_down_height = height_i32
        .checked_neg()
        .ok_or_else(|| "bmp top-down height overflow".to_owned())?;

    let payload_u32 = u32::try_from(expected).map_err(|_| "bmp payload too large".to_owned())?;
    let header_len_u32 = 54u32;
    let file_len_u32 = header_len_u32
        .checked_add(payload_u32)
        .ok_or_else(|| "bmp file length overflow".to_owned())?;

    let mut header = [0u8; 54];
    header[0..2].copy_from_slice(b"BM");
    header[2..6].copy_from_slice(&file_len_u32.to_le_bytes());
    header[10..14].copy_from_slice(&header_len_u32.to_le_bytes());
    header[14..18].copy_from_slice(&40u32.to_le_bytes());
    header[18..22].copy_from_slice(&width_i32.to_le_bytes());
    header[22..26].copy_from_slice(&top_down_height.to_le_bytes());
    header[26..28].copy_from_slice(&1u16.to_le_bytes());
    header[28..30].copy_from_slice(&32u16.to_le_bytes());
    header[30..34].copy_from_slice(&0u32.to_le_bytes());
    header[34..38].copy_from_slice(&payload_u32.to_le_bytes());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create dump directory '{}': {error}", parent.display()))?;
    }

    let mut file = std::fs::File::create(path)
        .map_err(|error| format!("failed to create bmp '{}': {error}", path.display()))?;
    file.write_all(&header)
        .map_err(|error| format!("failed to write bmp header '{}': {error}", path.display()))?;
    file.write_all(pixels)
        .map_err(|error| format!("failed to write bmp pixels '{}': {error}", path.display()))?;

    Ok(())
}

#[cfg(ironrdp_idd_link)]
fn maybe_dump_swapchain_surface_bmp(
    d3d_device: &ID3D11Device,
    d3d_context: &ID3D11DeviceContext,
    surface_ptr: *mut core::ffi::c_void,
    presentation_frame_number: u32,
) -> Result<(), String> {
    let config = crate::remote::swapchain_dump_runtime()?;
    let dir = config
        .dump_dir
        .as_ref()
        .ok_or_else(|| "dump_dir missing from runtime config".to_owned())?;
    let session_id = config
        .session_id
        .ok_or_else(|| "session_id missing from runtime config".to_owned())?;

    if surface_ptr.is_null() {
        return Err("swapchain metadata surface pointer is null".to_owned());
    }

    if !IDD_DUMP_ENABLED_LOGGED.swap(true, Ordering::Relaxed) {
        tracing::info!(
            dir = %dir.display(),
            session_id,
            "IDD swapchain bitmap dumping enabled"
        );
    }

    if let Some(now_ms) = now_unix_ms_best_effort() {
        let last_ms = IDD_DUMP_LAST_MS.load(Ordering::Relaxed);
        if last_ms != 0 && now_ms.saturating_sub(last_ms) < IDD_DUMP_INTERVAL_MS {
            return Ok(());
        }

        let count = IDD_DUMP_COUNT.load(Ordering::Relaxed);
        if count >= IDD_DUMP_MAX_COUNT {
            return Ok(());
        }

        IDD_DUMP_LAST_MS.store(now_ms, Ordering::Relaxed);
        IDD_DUMP_COUNT.fetch_add(1, Ordering::Relaxed);
    }

    // IddCx hands the current surface back as a borrowed `IDXGIResource*` that remains valid
    // until the next `ReleaseAndAcquireBuffer` call. Do not take ownership of that raw COM
    // pointer here or we'll under-release the OS-owned surface and eventually corrupt the
    // swapchain lifetime.
    let surface_raw = surface_ptr.cast::<core::ffi::c_void>();
    let acquired_surface = unsafe { IDXGIResource::from_raw_borrowed(&surface_raw) }
        .ok_or_else(|| "swapchain metadata surface pointer is null".to_owned())?;
    let source_resource: ID3D11Resource = acquired_surface
        .cast()
        .map_err(|error| format!("cast acquired IDXGIResource to ID3D11Resource failed: {error}"))?;
    let source_texture: ID3D11Texture2D = source_resource
        .cast()
        .map_err(|error| format!("cast acquired surface to ID3D11Texture2D failed: {error}"))?;

    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        source_texture.GetDesc(&mut source_desc);
    }

    if source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM && source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM_SRGB {
        return Err(format!(
            "unsupported surface format for bmp dump: {:?}",
            source_desc.Format
        ));
    }

    let mut staging_desc = source_desc;
    staging_desc.BindFlags = 0;
    staging_desc.MiscFlags = 0;
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;

    let mut staging_texture = None;
    unsafe {
        d3d_device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
            .map_err(|error| format!("CreateTexture2D(staging) failed: {error}"))?;
    }

    let staging_texture = staging_texture.ok_or_else(|| "CreateTexture2D returned no texture".to_owned())?;

    let staging_resource: ID3D11Resource = staging_texture
        .cast()
        .map_err(|error| format!("cast staging texture to ID3D11Resource failed: {error}"))?;

    unsafe {
        d3d_context.CopyResource(&staging_resource, &source_resource);
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { d3d_context.Map(&staging_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(|error| format!("ID3D11DeviceContext::Map failed: {error}"))?;

    let width = usize::try_from(source_desc.Width).map_err(|_| "surface width out of range".to_owned())?;
    let height = usize::try_from(source_desc.Height).map_err(|_| "surface height out of range".to_owned())?;
    let row_pitch = usize::try_from(mapped.RowPitch).map_err(|_| "row pitch out of range".to_owned())?;
    let packed_stride = width
        .checked_mul(4)
        .ok_or_else(|| "packed stride overflow".to_owned())?;

    if row_pitch < packed_stride {
        unsafe {
            d3d_context.Unmap(&staging_resource, 0);
        }
        return Err(format!(
            "mapped row pitch smaller than expected stride: row_pitch={row_pitch} packed_stride={packed_stride}"
        ));
    }

    let packed_len = packed_stride
        .checked_mul(height)
        .ok_or_else(|| "packed frame length overflow".to_owned())?;
    let mut packed = vec![0u8; packed_len];

    for row in 0..height {
        let src_offset = row
            .checked_mul(row_pitch)
            .ok_or_else(|| "source row offset overflow".to_owned())?;
        let dst_offset = row
            .checked_mul(packed_stride)
            .ok_or_else(|| "destination row offset overflow".to_owned())?;

        unsafe {
            let src = (mapped.pData.cast::<u8>()).add(src_offset);
            let dst = packed.as_mut_ptr().add(dst_offset);
            core::ptr::copy_nonoverlapping(src, dst, packed_stride);
        }
    }

    unsafe {
        d3d_context.Unmap(&staging_resource, 0);
    }

    let seq = IDD_DUMP_SEQ.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let path = dir.join(format!(
        "idd-swapchain-s{session_id:04}-f{presentation_frame_number:010}-{seq:06}.bmp"
    ));

    write_bmp_bgra32(&path, width, height, &packed)?;
    crate::debug_trace(&format!(
        "SESSION_PROOF_IDD_SWAPCHAIN_DUMP session_id={session_id} presentation_frame_number={presentation_frame_number} width={width} height={height} path={}",
        path.display()
    ));

    if IDD_FIRST_FRAME_SESSION.load(Ordering::Relaxed) != session_id {
        IDD_FIRST_FRAME_SESSION.store(session_id, Ordering::Relaxed);
        crate::remote::note_first_frame(session_id, presentation_frame_number, width, height, &path);
    }

    Ok(())
}

fn try_enable_mmcss() -> Option<MmcssGuard> {
    let mut task_index = 0u32;
    // SAFETY: `task_index` is a valid out pointer.
    let handle = match unsafe { AvSetMmThreadCharacteristicsW(w!("Games"), &mut task_index) } {
        Ok(handle) => handle,
        Err(error) => {
            tracing::debug!(?error, "AvSetMmThreadCharacteristicsW failed");
            return None;
        }
    };

    tracing::debug!(task_index, "MMCSS enabled for swapchain thread");
    Some(MmcssGuard(handle))
}

#[derive(Debug)]
struct SwapChainWorker {
    stop: Arc<AtomicBool>,
    join_handle: JoinHandle<()>,
}

#[derive(Debug)]
pub struct SwapChainProcessor {
    swapchain: SendSwapChain,
    render_adapter_luid: LUID,
    new_frame_event: SendHandle,
    terminate_event: OwnedHandle,
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    worker: Mutex<Option<SwapChainWorker>>,
}

impl SwapChainProcessor {
    pub fn new(
        swapchain: IDDCX_SWAPCHAIN,
        render_adapter_luid: LUID,
        new_frame_event: HANDLE,
    ) -> Result<Self, windows_core::HRESULT> {
        // SAFETY: `CreateEventW` is an FFI call. We create an unnamed manual-reset event, initially non-signaled.
        let terminate_event = unsafe { CreateEventW(None, true, false, None)? };

        let (d3d_device, d3d_context) = create_d3d_device(render_adapter_luid)?;
        Ok(Self {
            swapchain: SendSwapChain(swapchain),
            render_adapter_luid,
            new_frame_event: SendHandle(new_frame_event),
            terminate_event: OwnedHandle(terminate_event),
            d3d_device,
            d3d_context,
            worker: Mutex::new(None),
        })
    }

    pub fn start(&self) -> NTSTATUS {
        let mut worker = match self.worker.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        if worker.is_some() {
            return STATUS_SUCCESS;
        }

        let stop = Arc::new(AtomicBool::new(false));

        tracing::info!(
            swapchain = ?self.swapchain.0,
            render_adapter_luid = ?self.render_adapter_luid,
            "IddCxSwapChain processing thread start requested (stub)"
        );

        let stop_for_thread = Arc::clone(&stop);
        let new_frame_event = self.new_frame_event;
        let terminate_event = SendHandle(self.terminate_event.raw());
        let render_adapter_luid = self.render_adapter_luid;
        #[cfg(ironrdp_idd_link)]
        let swapchain = self.swapchain;
        #[cfg(ironrdp_idd_link)]
        let d3d_device = SendD3dDevice(self.d3d_device.clone());
        #[cfg(ironrdp_idd_link)]
        let d3d_context = SendD3dDeviceContext(self.d3d_context.clone());

        let join_handle = thread::spawn(move || {
            let _mmcss = try_enable_mmcss();
            tracing::info!(?render_adapter_luid, "IddCxSwapChain processing thread started");

            #[cfg(ironrdp_idd_link)]
            {
                crate::debug_trace(&format!(
                    "SESSION_PROOF_IDD_SWAPCHAIN_THREAD_START swapchain=0x{:X} adapter_luid=0x{:08X}{:08X}",
                    swapchain.raw() as usize,
                    render_adapter_luid.HighPart as u32,
                    render_adapter_luid.LowPart,
                ));
                let dxgi_device: IDXGIDevice = match d3d_device.device().cast() {
                    Ok(device) => device,
                    Err(error) => {
                        crate::debug_trace(&format!(
                            "SESSION_PROOF_IDD_SWAPCHAIN_THREAD_ERROR stage=cast_dxgi_device error={error}"
                        ));
                        tracing::warn!(?error, "failed to cast D3D11 device to IDXGIDevice");
                        return;
                    }
                };

                let dxgi_device_ptr = dxgi_device.as_raw();
                // SAFETY: called from the dedicated swapchain thread, using a live swapchain handle + COM device.
                let hr = unsafe { iddcx::swapchain_set_device(swapchain.raw(), dxgi_device_ptr) };
                if hr.is_err() {
                    crate::debug_trace(&format!(
                        "SESSION_PROOF_IDD_SWAPCHAIN_SET_DEVICE_RESULT status={hr:?}"
                    ));
                    tracing::warn!(?hr, "IddCxSwapChainSetDevice failed");
                    return;
                }
                crate::debug_trace("SESSION_PROOF_IDD_SWAPCHAIN_SET_DEVICE_RESULT status=ok");

                let mut out_args = iddcx::IDARG_OUT_RELEASEANDACQUIREBUFFER {
                    MetaData: iddcx::IDDCX_METADATA {
                        Size: 0,
                        PresentationFrameNumber: 0,
                        DirtyRectCount: 0,
                        MoveRegionCount: 0,
                        HwProtectedSurface: 0,
                        PresentDisplayQPCTime: 0,
                        pSurface: core::ptr::null_mut(),
                    },
                };
                let mut acquire_pending_logged = false;
                let mut acquire_success_logged = false;
                let mut finished_frame_logged = false;

                while !stop_for_thread.load(Ordering::SeqCst) {
                    // SAFETY: IddCx expects out args to be a valid writable pointer.
                    let acquire_hr =
                        unsafe { iddcx::swapchain_release_and_acquire_buffer(swapchain.raw(), &mut out_args) };
                    if acquire_hr == E_PENDING {
                        if !acquire_pending_logged {
                            acquire_pending_logged = true;
                            crate::debug_trace("SESSION_PROOF_IDD_SWAPCHAIN_ACQUIRE_PENDING");
                        }
                        let handles = [new_frame_event.raw(), terminate_event.raw()];

                        // SAFETY: `handles` contains valid event handles.
                        let wait = unsafe { WaitForMultipleObjects(&handles, false, 16) };

                        if wait == WAIT_OBJECT_0 || wait == WAIT_TIMEOUT {
                            continue;
                        }

                        if wait.0 == WAIT_OBJECT_0.0 + 1 {
                            crate::debug_trace("SESSION_PROOF_IDD_SWAPCHAIN_THREAD_STOP reason=terminate_event");
                            tracing::info!("IddCxSwapChain terminate event signaled");
                            break;
                        }

                        if wait == WAIT_FAILED {
                            // SAFETY: `GetLastError` has no preconditions.
                            let error = unsafe { GetLastError() };
                            crate::debug_trace(&format!(
                                "SESSION_PROOF_IDD_SWAPCHAIN_THREAD_ERROR stage=wait error=0x{:08X}",
                                error.0
                            ));
                            tracing::warn!(?error, "WaitForMultipleObjects failed in swapchain thread");
                            break;
                        }

                        crate::debug_trace(&format!(
                            "SESSION_PROOF_IDD_SWAPCHAIN_THREAD_ERROR stage=wait unexpected_result=0x{:08X}",
                            wait.0
                        ));
                        tracing::warn!(?wait, "unexpected WaitForMultipleObjects result in swapchain thread");
                        break;
                    }

                    if acquire_hr.is_err() {
                        crate::debug_trace(&format!(
                            "SESSION_PROOF_IDD_SWAPCHAIN_ACQUIRE_RESULT status={acquire_hr:?}"
                        ));
                        tracing::warn!(?acquire_hr, "IddCxSwapChainReleaseAndAcquireBuffer failed");
                        break;
                    }

                    let meta = &out_args.MetaData;
                    if !acquire_success_logged {
                        acquire_success_logged = true;
                        crate::debug_trace(&format!(
                            "SESSION_PROOF_IDD_SWAPCHAIN_ACQUIRE_RESULT status=ok presentation_frame_number={} dirty_rects={} move_regions={} hw_protected={} surface_nonnull={}",
                            meta.PresentationFrameNumber,
                            meta.DirtyRectCount,
                            meta.MoveRegionCount,
                            meta.HwProtectedSurface,
                            !meta.pSurface.is_null(),
                        ));
                    }
                    tracing::debug!(
                        presentation_frame_number = meta.PresentationFrameNumber,
                        dirty_rects = meta.DirtyRectCount,
                        move_regions = meta.MoveRegionCount,
                        hw_protected = meta.HwProtectedSurface,
                        qpc_time = meta.PresentDisplayQPCTime,
                        surface = ?meta.pSurface,
                        "IddCxSwapChain acquired buffer"
                    );

                    {
                        if let Err(error) = maybe_dump_swapchain_surface_bmp(
                            d3d_device.device(),
                            d3d_context.context(),
                            meta.pSurface,
                            meta.PresentationFrameNumber,
                        ) {
                            if !IDD_DUMP_ERROR_LOGGED.swap(true, Ordering::Relaxed) {
                                crate::debug_trace(&format!(
                                    "SESSION_PROOF_IDD_SWAPCHAIN_DUMP_ERROR error={error}"
                                ));
                                tracing::warn!(error, "Failed to dump IDD swapchain bitmap frame");
                            }
                        }
                    }

                    // SAFETY: IddCx requires FinishedProcessingFrame be called after the driver is done with the acquired surface.
                    let finished_hr = unsafe { iddcx::swapchain_finished_processing_frame(swapchain.raw()) };
                    if finished_hr.is_err() {
                        crate::debug_trace(&format!(
                            "SESSION_PROOF_IDD_SWAPCHAIN_FINISH_RESULT status={finished_hr:?}"
                        ));
                        tracing::warn!(?finished_hr, "IddCxSwapChainFinishedProcessingFrame failed");
                        break;
                    }
                    if !finished_frame_logged {
                        finished_frame_logged = true;
                        crate::debug_trace("SESSION_PROOF_IDD_SWAPCHAIN_FINISH_RESULT status=ok");
                    }
                }

                crate::debug_trace("SESSION_PROOF_IDD_SWAPCHAIN_THREAD_STOP reason=loop_exit");
                tracing::info!("IddCxSwapChain processing thread exiting");
            }

            #[cfg(not(ironrdp_idd_link))]
            {
                crate::debug_trace(&format!(
                    "SESSION_PROOF_IDD_SWAPCHAIN_THREAD_START swapchain=stub adapter_luid=0x{:08X}{:08X}",
                    render_adapter_luid.HighPart as u32,
                    render_adapter_luid.LowPart,
                ));
                while !stop_for_thread.load(Ordering::SeqCst) {
                    let handles = [new_frame_event.raw(), terminate_event.raw()];

                    // SAFETY: `handles` contains valid event handles.
                    let wait = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };

                    if wait == WAIT_OBJECT_0 {
                        tracing::debug!("IddCxSwapChain new frame signaled (stub)");
                        continue;
                    }

                    if wait.0 == WAIT_OBJECT_0.0 + 1 {
                        tracing::info!("IddCxSwapChain terminate event signaled (stub)");
                        break;
                    }

                    if wait == WAIT_FAILED {
                        // SAFETY: `GetLastError` has no preconditions.
                        let error = unsafe { GetLastError() };
                        tracing::warn!(?error, "WaitForMultipleObjects failed in swapchain thread");
                        break;
                    }

                    tracing::warn!(?wait, "unexpected WaitForMultipleObjects result in swapchain thread");
                    break;
                }

                tracing::info!("IddCxSwapChain processing thread exiting (stub)");
            }
        });

        *worker = Some(SwapChainWorker { stop, join_handle });
        STATUS_SUCCESS
    }

    pub fn stop(&self) -> NTSTATUS {
        let worker = {
            let mut guard = match self.worker.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.take()
        };

        let Some(worker) = worker else {
            return STATUS_SUCCESS;
        };

        worker.stop.store(true, Ordering::SeqCst);

        // SAFETY: `terminate_event` is an event handle created by this crate.
        unsafe {
            let _ = SetEvent(self.terminate_event.raw());
        }

        if let Err(error) = worker.join_handle.join() {
            tracing::warn!(?error, "IddCxSwapChain processing thread join failed");
            return STATUS_NOT_SUPPORTED;
        }

        STATUS_SUCCESS
    }

    #[must_use]
    pub fn swapchain(&self) -> IDDCX_SWAPCHAIN {
        self.swapchain.0
    }

    #[must_use]
    pub fn render_adapter_luid(&self) -> LUID {
        self.render_adapter_luid
    }

    #[must_use]
    pub fn new_frame_event(&self) -> HANDLE {
        self.new_frame_event.0
    }

    #[must_use]
    pub fn d3d_device(&self) -> &ID3D11Device {
        &self.d3d_device
    }

    #[must_use]
    pub fn d3d_context(&self) -> &ID3D11DeviceContext {
        &self.d3d_context
    }
}

impl Drop for SwapChainProcessor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn create_d3d_device(render_adapter_luid: LUID) -> windows_core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    // SAFETY: `CreateDXGIFactory2` is an FFI call. We pass valid flags (0) and rely on DXGI to return a valid COM factory.
    let factory: IDXGIFactory5 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))? };

    // SAFETY: `factory` is a valid COM object. `render_adapter_luid` is expected to identify a valid adapter on the system.
    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(render_adapter_luid)? };

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    let device_out: *mut Option<ID3D11Device> = core::ptr::addr_of_mut!(device);
    let context_out: *mut Option<ID3D11DeviceContext> = core::ptr::addr_of_mut!(context);

    // SAFETY: `adapter` is a live COM object. `device_out`/`context_out` are valid writable pointers to `Option<T>` slots.
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(device_out),
            None,
            Some(context_out),
        )?;
    }

    let device = device.ok_or_else(|| {
        windows_core::Error::new(
            windows_core::HRESULT(-2147467259),
            "d3d11createdevice returned no device",
        )
    })?;
    let context = context.ok_or_else(|| {
        windows_core::Error::new(
            windows_core::HRESULT(-2147467259),
            "d3d11createdevice returned no context",
        )
    })?;

    Ok((device, context))
}



use crate::{IDDCX_SWAPCHAIN, NTSTATUS, STATUS_NOT_SUPPORTED, STATUS_SUCCESS};
use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, HMODULE, LUID, WAIT_FAILED, WAIT_OBJECT_0};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
#[cfg(ironrdp_idd_link)]
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, IDXGIAdapter1, IDXGIFactory5, DXGI_CREATE_FACTORY_FLAGS};
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

        let join_handle = thread::spawn(move || {
            let _mmcss = try_enable_mmcss();
            tracing::info!(?render_adapter_luid, "IddCxSwapChain processing thread started");

            #[cfg(ironrdp_idd_link)]
            {
                let dxgi_device: IDXGIDevice = match d3d_device.device().cast() {
                    Ok(device) => device,
                    Err(error) => {
                        tracing::warn!(?error, "failed to cast D3D11 device to IDXGIDevice");
                        return;
                    }
                };

                let dxgi_device_ptr = dxgi_device.as_raw();
                // SAFETY: called from the dedicated swapchain thread, using a live swapchain handle + COM device.
                let hr = unsafe { iddcx::swapchain_set_device(swapchain.raw(), dxgi_device_ptr) };
                if hr.is_err() {
                    tracing::warn!(?hr, "IddCxSwapChainSetDevice failed");
                    return;
                }

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

                while !stop_for_thread.load(Ordering::SeqCst) {
                    // SAFETY: IddCx expects out args to be a valid writable pointer.
                    let acquire_hr =
                        unsafe { iddcx::swapchain_release_and_acquire_buffer(swapchain.raw(), &mut out_args) };
                    if acquire_hr == E_PENDING {
                        let handles = [new_frame_event.raw(), terminate_event.raw()];

                        // SAFETY: `handles` contains valid event handles.
                        let wait = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };

                        if wait == WAIT_OBJECT_0 {
                            continue;
                        }

                        if wait.0 == WAIT_OBJECT_0.0 + 1 {
                            tracing::info!("IddCxSwapChain terminate event signaled");
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

                    if acquire_hr.is_err() {
                        tracing::warn!(?acquire_hr, "IddCxSwapChainReleaseAndAcquireBuffer failed");
                        break;
                    }

                    let meta = &out_args.MetaData;
                    tracing::debug!(
                        presentation_frame_number = meta.PresentationFrameNumber,
                        dirty_rects = meta.DirtyRectCount,
                        move_regions = meta.MoveRegionCount,
                        hw_protected = meta.HwProtectedSurface,
                        qpc_time = meta.PresentDisplayQPCTime,
                        surface = ?meta.pSurface,
                        "IddCxSwapChain acquired buffer"
                    );

                    // SAFETY: IddCx requires FinishedProcessingFrame be called after the driver is done with the acquired surface.
                    let finished_hr = unsafe { iddcx::swapchain_finished_processing_frame(swapchain.raw()) };
                    if finished_hr.is_err() {
                        tracing::warn!(?finished_hr, "IddCxSwapChainFinishedProcessingFrame failed");
                        break;
                    }
                }

                tracing::info!("IddCxSwapChain processing thread exiting");
            }

            #[cfg(not(ironrdp_idd_link))]
            {
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

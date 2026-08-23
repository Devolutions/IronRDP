use core::mem::offset_of;
use core::ptr::NonNull;
use std::ffi::OsStr;
use std::fmt;
use std::os::windows::ffi::OsStrExt as _;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use ironrdp_core::{Encode, EncodeResult, WriteCursor, ensure_size, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduResult, pdu_other_err};
use tracing::{debug, error, trace, warn};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED_0, WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, IsValidSid, SID_AND_ATTRIBUTES, TOKEN_GROUPS, TOKEN_QUERY, TokenGroups,
};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, INFINITE, OpenEventW, OpenMutexW, OpenProcessToken, ReleaseMutex,
    SYNCHRONIZATION_ACCESS_RIGHTS, SetEvent, WaitForMultipleObjects,
};
use windows::core::PCWSTR;

/// Dynamic virtual channel used by the local Hyper-V frame-buffer fast path.
pub const FRAME_BUFFER_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Frame_Buffer::Control::v08.01";

const GLOBAL_OBJECT_PREFIX: &str = r"Global\Microsoft::Windows::RDS::FBR";
const CLIENT_HELLO: u32 = 1;
const SERVER_HELLO: u32 = 2;
const CLIENT_ACK: u32 = 3;
const CONTROL_HEADER_SIZE: usize = 9;
const CLIENT_HELLO_FIXED_SIZE: usize = 14;
const SERVER_HELLO_FIXED_SIZE: usize = 33;
const MAX_OBJECT_SUFFIX_BYTES: usize = 0x208;
const MAX_LOGON_SID_BYTES: usize = 0x44;
const SHARED_BUFFER_INFO_SIZE: usize = 56;
const BITMAP_INFO_HEADER_SIZE: u32 = 40;
const MAX_FRAME_DIMENSION: u16 = 8192;
const LOGON_ID_GROUP_ATTRIBUTES: u32 = 0xC000_0000;
const SYNCHRONIZE_ACCESS: SYNCHRONIZATION_ACCESS_RIGHTS = SYNCHRONIZATION_ACCESS_RIGHTS(0x0010_0000);

type FrameCallback = Arc<dyn Fn(Vec<u32>, u16, u16) + Send + Sync>;

/// Client endpoint for Hyper-V frame-buffer redirection.
///
/// The Hyper-V host offers this DVC only for a same-machine RDP connection.
/// Frames are delivered as `0x00RRGGBB` pixels in row-major order.
pub struct FrameBufferClient {
    on_frame: FrameCallback,
    worker: Option<FrameBufferWorker>,
}

impl FrameBufferClient {
    /// Creates a frame-buffer channel with a callback for complete frames.
    pub fn new<F>(on_frame: F) -> Self
    where
        F: Fn(Vec<u32>, u16, u16) + Send + Sync + 'static,
    {
        Self {
            on_frame: Arc::new(on_frame),
            worker: None,
        }
    }

    fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            drop(worker);
        }
    }
}

impl fmt::Debug for FrameBufferClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameBufferClient")
            .field("active", &self.worker.is_some())
            .finish()
    }
}

impl_as_any!(FrameBufferClient);

impl DvcProcessor for FrameBufferClient {
    fn channel_name(&self) -> &str {
        FRAME_BUFFER_CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.stop_worker();
        let sid = current_logon_sid()?;
        Ok(vec![Box::new(ControlPdu::client_hello(&sid)?)])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let names = parse_server_hello(payload)?;
        self.stop_worker();
        self.worker = Some(FrameBufferWorker::start(names, Arc::clone(&self.on_frame))?);
        Ok(vec![Box::new(ControlPdu::client_ack())])
    }

    fn close(&mut self, _channel_id: u32) {
        self.stop_worker();
    }
}

impl DvcClientProcessor for FrameBufferClient {}

#[derive(Debug)]
struct ControlPdu(Vec<u8>);

impl ControlPdu {
    fn client_hello(sid: &[u8]) -> PduResult<Self> {
        if sid.is_empty() || sid.len() > MAX_LOGON_SID_BYTES {
            return Err(pdu_other_err!("invalid FBR logon SID length"));
        }

        let total_length = CLIENT_HELLO_FIXED_SIZE
            .checked_add(sid.len())
            .ok_or_else(|| pdu_other_err!("fbr client hello length overflow"))?;
        let total_length_u32 =
            u32::try_from(total_length).map_err(|_| pdu_other_err!("fbr client hello is too large"))?;
        let sid_length = u32::try_from(sid.len()).map_err(|_| pdu_other_err!("fbr logon SID is too large"))?;

        let mut payload = Vec::with_capacity(total_length);
        payload.extend_from_slice(&total_length_u32.to_le_bytes());
        payload.extend_from_slice(&CLIENT_HELLO.to_le_bytes());
        payload.push(0);
        payload.extend_from_slice(&sid_length.to_le_bytes());
        payload.extend_from_slice(sid);
        payload.push(0);
        Ok(Self(payload))
    }

    fn client_ack() -> Self {
        let mut payload = Vec::with_capacity(CONTROL_HEADER_SIZE);
        payload.extend_from_slice(
            &u32::try_from(CONTROL_HEADER_SIZE)
                .expect("fixed size fits")
                .to_le_bytes(),
        );
        payload.extend_from_slice(&CLIENT_ACK.to_le_bytes());
        payload.push(0);
        Self(payload)
    }
}

impl Encode for ControlPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.0.len());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "FBR_CONTROL_PDU"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for ControlPdu {}

#[derive(Debug)]
struct ServerHello {
    mapping_suffix: String,
    mutex_suffix: String,
    event_suffix: String,
}

fn parse_server_hello(payload: &[u8]) -> PduResult<ServerHello> {
    if payload.len() < SERVER_HELLO_FIXED_SIZE {
        return Err(pdu_other_err!("fbr server hello is truncated"));
    }

    let total_length = usize::try_from(read_u32(payload, 0)?)
        .map_err(|_| pdu_other_err!("fbr server hello length does not fit usize"))?;
    if total_length != payload.len() {
        return Err(pdu_other_err!("fbr server hello length mismatch"));
    }
    if read_u32(payload, 4)? != SERVER_HELLO {
        return Err(pdu_other_err!("unexpected FBR control message"));
    }

    let mapping_offset = usize::try_from(read_u32(payload, 9)?)
        .map_err(|_| pdu_other_err!("fbr mapping suffix offset does not fit usize"))?;
    let mapping_length = usize::try_from(read_u32(payload, 13)?)
        .map_err(|_| pdu_other_err!("fbr mapping suffix length does not fit usize"))?;
    let mutex_offset = usize::try_from(read_u32(payload, 17)?)
        .map_err(|_| pdu_other_err!("fbr mutex suffix offset does not fit usize"))?;
    let mutex_length = usize::try_from(read_u32(payload, 21)?)
        .map_err(|_| pdu_other_err!("fbr mutex suffix length does not fit usize"))?;
    let event_offset = usize::try_from(read_u32(payload, 25)?)
        .map_err(|_| pdu_other_err!("fbr event suffix offset does not fit usize"))?;
    let event_length = usize::try_from(read_u32(payload, 29)?)
        .map_err(|_| pdu_other_err!("fbr event suffix length does not fit usize"))?;

    if mapping_offset != SERVER_HELLO_FIXED_SIZE
        || mutex_offset != checked_end(mapping_offset, mapping_length)?
        || event_offset != checked_end(mutex_offset, mutex_length)?
        || payload.len() != checked_end(event_offset, event_length)?
    {
        return Err(pdu_other_err!("fbr server hello contains invalid object-name ranges"));
    }

    Ok(ServerHello {
        mapping_suffix: decode_object_suffix(payload, mapping_offset, mapping_length)?,
        mutex_suffix: decode_object_suffix(payload, mutex_offset, mutex_length)?,
        event_suffix: decode_object_suffix(payload, event_offset, event_length)?,
    })
}

fn checked_end(offset: usize, length: usize) -> PduResult<usize> {
    offset
        .checked_add(length)
        .ok_or_else(|| pdu_other_err!("fbr object-name range overflow"))
}

fn decode_object_suffix(payload: &[u8], offset: usize, length: usize) -> PduResult<String> {
    if length == 0 || length > MAX_OBJECT_SUFFIX_BYTES || !length.is_multiple_of(2) {
        return Err(pdu_other_err!("invalid FBR object suffix length"));
    }
    let bytes = payload
        .get(offset..checked_end(offset, length)?)
        .ok_or_else(|| pdu_other_err!("fbr object suffix is truncated"))?;
    let words = bytes
        .chunks_exact(2)
        .map(|word| u16::from_le_bytes([word[0], word[1]]))
        .collect::<Vec<_>>();
    let suffix =
        String::from_utf16(&words).map_err(|error| pdu_other_err!("decode FBR object suffix", source: error))?;
    if !is_guid_suffix(&suffix) {
        return Err(pdu_other_err!("fbr object suffix is not a GUID"));
    }
    Ok(suffix)
}

fn is_guid_suffix(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(idx, byte)| {
            if matches!(idx, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn read_u32(payload: &[u8], offset: usize) -> PduResult<u32> {
    let end = offset
        .checked_add(size_of::<u32>())
        .ok_or_else(|| pdu_other_err!("fbr integer range overflow"))?;
    let bytes = payload
        .get(offset..end)
        .ok_or_else(|| pdu_other_err!("fbr control message is truncated"))?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("slice length checked above"),
    ))
}

fn current_logon_sid() -> PduResult<Vec<u8>> {
    let mut token = HANDLE::default();
    // SAFETY: GetCurrentProcess returns a pseudo-handle that must not be closed.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `process` is valid and `token` is a writable out parameter.
    unsafe {
        OpenProcessToken(process, TOKEN_QUERY, &mut token)
            .map_err(|error| pdu_other_err!("open current process token for FBR", source: error))?;
    }
    let token = OwnedHandle(token);

    let mut required = 0u32;
    // SAFETY: this is the documented size-probe call; only `required` is written.
    let probe = unsafe { GetTokenInformation(token.get(), TokenGroups, None, 0, &mut required) };
    if required == 0 {
        return Err(match probe {
            Ok(()) => pdu_other_err!("fbr token groups size is zero"),
            Err(error) => pdu_other_err!("query FBR token groups size", source: error),
        });
    }

    let required_usize =
        usize::try_from(required).map_err(|_| pdu_other_err!("fbr token groups size does not fit usize"))?;
    let mut buffer = vec![0u8; required_usize];
    // SAFETY: `buffer` contains `required` writable bytes and `token` remains valid.
    unsafe {
        GetTokenInformation(
            token.get(),
            TokenGroups,
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut required,
        )
        .map_err(|error| pdu_other_err!("query FBR token groups", source: error))?;
    }

    if buffer.len() < offset_of!(TOKEN_GROUPS, Groups) {
        return Err(pdu_other_err!("fbr token groups header is truncated"));
    }
    // SAFETY: `GetTokenInformation` initialized a TOKEN_GROUPS header; byte storage may be unaligned.
    let group_count = unsafe { buffer.as_ptr().cast::<TOKEN_GROUPS>().read_unaligned().GroupCount };
    let group_count =
        usize::try_from(group_count).map_err(|_| pdu_other_err!("fbr token group count does not fit usize"))?;
    let groups_size = group_count
        .checked_mul(size_of::<SID_AND_ATTRIBUTES>())
        .ok_or_else(|| pdu_other_err!("fbr token groups size overflow"))?;
    let groups_end = offset_of!(TOKEN_GROUPS, Groups)
        .checked_add(groups_size)
        .ok_or_else(|| pdu_other_err!("fbr token groups range overflow"))?;
    if groups_end > buffer.len() {
        return Err(pdu_other_err!("fbr token groups array is truncated"));
    }

    // SAFETY: the validated offset is within `buffer`.
    let groups_ptr = unsafe { buffer.as_ptr().add(offset_of!(TOKEN_GROUPS, Groups)) };
    // SAFETY: the bounds above cover the complete SID_AND_ATTRIBUTES array.
    let groups = unsafe { core::slice::from_raw_parts(groups_ptr, groups_size) };
    for group in groups.chunks_exact(size_of::<SID_AND_ATTRIBUTES>()) {
        // SAFETY: each chunk has exactly the size of SID_AND_ATTRIBUTES and may be unaligned.
        let group = unsafe { group.as_ptr().cast::<SID_AND_ATTRIBUTES>().read_unaligned() };
        if group.Attributes & LOGON_ID_GROUP_ATTRIBUTES != LOGON_ID_GROUP_ATTRIBUTES {
            continue;
        }
        // SAFETY: the SID pointer comes from the token buffer and is valid while `buffer` is alive.
        if !unsafe { IsValidSid(group.Sid) }.as_bool() {
            return Err(pdu_other_err!("current token contains an invalid FBR logon SID"));
        }
        // SAFETY: the SID was validated immediately above.
        let sid_length = unsafe { GetLengthSid(group.Sid) };
        let sid_length =
            usize::try_from(sid_length).map_err(|_| pdu_other_err!("fbr logon SID length does not fit usize"))?;
        if sid_length == 0 || sid_length > MAX_LOGON_SID_BYTES {
            return Err(pdu_other_err!("invalid FBR logon SID length"));
        }
        // SAFETY: GetLengthSid returned the valid byte length for this token-owned SID.
        return Ok(unsafe { core::slice::from_raw_parts(group.Sid.0.cast::<u8>(), sid_length) }.to_vec());
    }

    Err(pdu_other_err!("current token has no FBR logon SID"))
}

struct FrameBufferWorker {
    stop_event: Arc<OwnedHandle>,
    thread: Option<JoinHandle<()>>,
}

impl FrameBufferWorker {
    fn start(names: ServerHello, on_frame: FrameCallback) -> PduResult<Self> {
        let shared = SharedBuffer::open(&names)?;
        // SAFETY: unnamed manual-reset event with default security attributes.
        let stop_event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map(OwnedHandle)
            .map_err(|error| pdu_other_err!("create FBR stop event", source: error))?;
        let stop_event = Arc::new(stop_event);
        let thread_stop_event = Arc::clone(&stop_event);
        let thread = thread::Builder::new()
            .name("ironrdp-fbr-present".to_owned())
            .spawn(move || present_loop(thread_stop_event, shared, on_frame))
            .map_err(|error| pdu_other_err!("spawn FBR presenter", source: error))?;

        Ok(Self {
            stop_event,
            thread: Some(thread),
        })
    }
}

impl Drop for FrameBufferWorker {
    fn drop(&mut self) {
        // SAFETY: the event handle stays alive through the subsequent join.
        if let Err(error) = unsafe { SetEvent(self.stop_event.get()) } {
            warn!(%error, "Failed to signal the FBR presenter stop event");
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            error!("FBR presenter thread panicked");
        }
    }
}

struct SharedBuffer {
    event: OwnedHandle,
    mutex: OwnedHandle,
    header: MappedView,
    pixels: MappedView,
    _mapping: OwnedHandle,
    layout: BitmapLayout,
}

impl SharedBuffer {
    fn open(names: &ServerHello) -> PduResult<Self> {
        let mapping_name = global_object_name(&names.mapping_suffix);
        let mutex_name = global_object_name(&names.mutex_suffix);
        let event_name = global_object_name(&names.event_suffix);
        let mapping_name = wide_null(&mapping_name);
        let mutex_name = wide_null(&mutex_name);
        let event_name = wide_null(&event_name);

        // SAFETY: names are terminated UTF-16 strings and handles are owned by the returned wrappers.
        let event = unsafe { OpenEventW(SYNCHRONIZE_ACCESS, false, PCWSTR(event_name.as_ptr())) }
            .map(OwnedHandle)
            .map_err(|error| pdu_other_err!("open FBR frame event", source: error))?;
        // The server grants only SYNCHRONIZE. ReleaseMutex succeeds for the thread that acquires it,
        // matching mstscax's access mask and wait/release sequence.
        // SAFETY: `mutex_name` is a terminated UTF-16 string.
        let mutex = unsafe { OpenMutexW(SYNCHRONIZE_ACCESS, false, PCWSTR(mutex_name.as_ptr())) }
            .map(OwnedHandle)
            .map_err(|error| pdu_other_err!("open FBR frame mutex", source: error))?;
        // SAFETY: `mapping_name` is a terminated UTF-16 string.
        let mapping =
            unsafe { OpenFileMappingW((FILE_MAP_READ | FILE_MAP_WRITE).0, false, PCWSTR(mapping_name.as_ptr())) }
                .map(OwnedHandle)
                .map_err(|error| pdu_other_err!("open FBR file mapping", source: error))?;

        let header = MappedView::map(
            mapping.get(),
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            SHARED_BUFFER_INFO_SIZE,
        )
        .map_err(|error| pdu_other_err!("map FBR shared-buffer header", source: error))?;
        let layout = BitmapLayout::parse(&header)?;

        let mut system_info = SYSTEM_INFO::default();
        // SAFETY: `system_info` is a writable SYSTEM_INFO value.
        unsafe {
            GetSystemInfo(&mut system_info);
        }
        let granularity = usize::try_from(system_info.dwAllocationGranularity)
            .map_err(|_| pdu_other_err!("fbr allocation granularity does not fit usize"))?;
        let pixel_offset = SHARED_BUFFER_INFO_SIZE
            .checked_next_multiple_of(granularity)
            .ok_or_else(|| pdu_other_err!("fbr pixel mapping offset overflow"))?;
        let pixels = MappedView::map(mapping.get(), FILE_MAP_READ, pixel_offset, layout.pixel_bytes)
            .map_err(|error| pdu_other_err!("map FBR pixel buffer", source: error))?;

        debug!(
            width = layout.width,
            height = layout.height,
            top_down = layout.top_down,
            "Opened Hyper-V FBR shared buffer"
        );

        Ok(Self {
            event,
            mutex,
            header,
            pixels,
            _mapping: mapping,
            layout,
        })
    }

    fn take_frame(&mut self) -> Option<Vec<u32>> {
        // SAFETY: the header mapping contains at least the 16-byte dirty RECT and the mutex is held.
        let dirty = unsafe {
            let rect = core::slice::from_raw_parts(self.header.as_ptr(), 16);
            [
                i32::from_le_bytes(rect[0..4].try_into().expect("fixed range")),
                i32::from_le_bytes(rect[4..8].try_into().expect("fixed range")),
                i32::from_le_bytes(rect[8..12].try_into().expect("fixed range")),
                i32::from_le_bytes(rect[12..16].try_into().expect("fixed range")),
            ]
        };
        if dirty[0] >= dirty[2] || dirty[1] >= dirty[3] {
            return None;
        }

        // SAFETY: the pixel view was mapped for exactly `pixel_bytes`.
        let pixels = unsafe { core::slice::from_raw_parts(self.pixels.as_ptr(), self.layout.pixel_bytes) };
        let frame = decode_bgra32(pixels, self.layout);
        // SAFETY: the header view has write access and the mutex excludes the server update thread.
        unsafe {
            self.header.as_ptr().write_bytes(0, 16);
        }
        Some(frame)
    }
}

#[derive(Clone, Copy)]
struct BitmapLayout {
    width: u16,
    height: u16,
    stride: usize,
    pixel_bytes: usize,
    top_down: bool,
}

impl BitmapLayout {
    fn parse(header: &MappedView) -> PduResult<Self> {
        // SAFETY: the header view was mapped for SHARED_BUFFER_INFO_SIZE bytes.
        let header = unsafe { core::slice::from_raw_parts(header.as_ptr(), SHARED_BUFFER_INFO_SIZE) };
        let bitmap_header_size = u32::from_le_bytes(header[16..20].try_into().expect("fixed range"));
        let width = i32::from_le_bytes(header[20..24].try_into().expect("fixed range"));
        let signed_height = i32::from_le_bytes(header[24..28].try_into().expect("fixed range"));
        let planes = u16::from_le_bytes(header[28..30].try_into().expect("fixed range"));
        let bits_per_pixel = u16::from_le_bytes(header[30..32].try_into().expect("fixed range"));
        let compression = u32::from_le_bytes(header[32..36].try_into().expect("fixed range"));

        if bitmap_header_size != BITMAP_INFO_HEADER_SIZE
            || width <= 0
            || signed_height == 0
            || planes != 1
            || bits_per_pixel != 32
            || compression != 0
        {
            return Err(pdu_other_err!("unsupported FBR bitmap layout"));
        }

        let width = u16::try_from(width).map_err(|_| pdu_other_err!("fbr bitmap width is out of range"))?;
        let height = u16::try_from(
            signed_height
                .checked_abs()
                .ok_or_else(|| pdu_other_err!("fbr bitmap height is out of range"))?,
        )
        .map_err(|_| pdu_other_err!("fbr bitmap height is out of range"))?;
        if width > MAX_FRAME_DIMENSION || height > MAX_FRAME_DIMENSION {
            return Err(pdu_other_err!("fbr bitmap dimensions exceed the RDP maximum"));
        }

        let stride = usize::from(width)
            .checked_mul(4)
            .ok_or_else(|| pdu_other_err!("fbr bitmap stride overflow"))?;
        let pixel_bytes = stride
            .checked_mul(usize::from(height))
            .ok_or_else(|| pdu_other_err!("fbr bitmap size overflow"))?;
        Ok(Self {
            width,
            height,
            stride,
            pixel_bytes,
            top_down: signed_height < 0,
        })
    }
}

fn decode_bgra32(pixels: &[u8], layout: BitmapLayout) -> Vec<u32> {
    let width = usize::from(layout.width);
    let height = usize::from(layout.height);
    let mut frame = Vec::with_capacity(width.saturating_mul(height));
    for destination_y in 0..height {
        let source_y = if layout.top_down {
            destination_y
        } else {
            height - 1 - destination_y
        };
        let row_start = source_y * layout.stride;
        let row = &pixels[row_start..row_start + layout.stride];
        frame.extend(
            row.chunks_exact(4)
                .take(width)
                .map(|pixel| u32::from_be_bytes([0, pixel[2], pixel[1], pixel[0]])),
        );
    }
    frame
}

fn present_loop(stop_event: Arc<OwnedHandle>, mut shared: SharedBuffer, on_frame: FrameCallback) {
    let frame_wait = [stop_event.get(), shared.event.get()];
    loop {
        // SAFETY: both handles remain alive for the duration of the wait.
        match unsafe { WaitForMultipleObjects(&frame_wait, false, INFINITE) } {
            result if result == WAIT_OBJECT_0 => break,
            result if result == wait_index(WAIT_OBJECT_0, 1) => {}
            WAIT_FAILED => {
                error!(error = %windows::core::Error::from_thread(), "FBR frame wait failed");
                break;
            }
            result => {
                error!(wait_result = result.0, "FBR frame wait returned an unexpected result");
                break;
            }
        }

        let mutex_wait = [stop_event.get(), shared.mutex.get()];
        // SAFETY: both handles remain alive for the duration of the wait.
        let wait_result = unsafe { WaitForMultipleObjects(&mutex_wait, false, INFINITE) };
        if wait_result == WAIT_OBJECT_0 {
            break;
        }
        if wait_result == wait_index(WAIT_ABANDONED_0, 1) {
            warn!("FBR frame mutex was abandoned");
            // SAFETY: an abandoned wait grants this thread ownership of the mutex.
            if let Err(error) = unsafe { ReleaseMutex(shared.mutex.get()) } {
                error!(%error, "Failed to release the abandoned FBR frame mutex");
            }
            break;
        }
        if wait_result != wait_index(WAIT_OBJECT_0, 1) {
            if wait_result == WAIT_FAILED {
                error!(error = %windows::core::Error::from_thread(), "FBR mutex wait failed");
            } else {
                error!(
                    wait_result = wait_result.0,
                    "FBR mutex wait returned an unexpected result"
                );
            }
            break;
        }

        let frame = shared.take_frame();
        // SAFETY: the successful wait grants this thread ownership of the mutex.
        if let Err(error) = unsafe { ReleaseMutex(shared.mutex.get()) } {
            error!(%error, "Failed to release the FBR frame mutex");
            break;
        }
        if let Some(frame) = frame {
            trace!(
                width = shared.layout.width,
                height = shared.layout.height,
                "Received Hyper-V FBR frame"
            );
            on_frame(frame, shared.layout.width, shared.layout.height);
        }
    }
}

const fn wait_index(base: WAIT_EVENT, index: u32) -> WAIT_EVENT {
    WAIT_EVENT(base.0.wrapping_add(index))
}

fn global_object_name(suffix: &str) -> String {
    format!("{GLOBAL_OBJECT_PREFIX}-{suffix}")
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(core::iter::once(0)).collect()
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    const fn get(&self) -> HANDLE {
        self.0
    }
}

// SAFETY: Windows kernel handles may be used from any thread; this wrapper owns one handle.
unsafe impl Send for OwnedHandle {}
// SAFETY: operations performed through shared references do not mutate the wrapper or transfer ownership.
unsafe impl Sync for OwnedHandle {}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the non-pseudo handle and closes it exactly once.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            warn!(%error, "Failed to close an FBR handle");
        }
    }
}

struct MappedView {
    address: NonNull<u8>,
}

impl MappedView {
    fn map(
        handle: HANDLE,
        access: windows::Win32::System::Memory::FILE_MAP,
        offset: usize,
        size: usize,
    ) -> windows::core::Result<Self> {
        let offset = u64::try_from(offset).expect("Windows usize fits in u64");
        let offset_high = u32::try_from(offset >> 32).expect("shifted mapping offset fits in u32");
        let offset_low = u32::try_from(offset & u64::from(u32::MAX)).expect("masked mapping offset fits in u32");
        // SAFETY: the mapping handle is valid and the requested range is checked by the kernel.
        let view = unsafe { MapViewOfFile(handle, access, offset_high, offset_low, size) };
        let address = NonNull::new(view.Value.cast::<u8>()).ok_or_else(windows::core::Error::from_thread)?;
        Ok(Self { address })
    }

    const fn as_ptr(&self) -> *mut u8 {
        self.address.as_ptr()
    }
}

// SAFETY: a mapped view may be transferred to one worker thread and remains valid until dropped.
unsafe impl Send for MappedView {}

impl Drop for MappedView {
    fn drop(&mut self) {
        let view = MEMORY_MAPPED_VIEW_ADDRESS {
            Value: self.address.as_ptr().cast(),
        };
        // SAFETY: this wrapper owns the mapped view and unmaps it exactly once.
        if let Err(error) = unsafe { UnmapViewOfFile(view) } {
            warn!(%error, "Failed to unmap an FBR view");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_control_messages_match_mstscax_layout() {
        let sid = [1, 2, 3, 4];
        let hello = ControlPdu::client_hello(&sid).expect("valid hello");

        assert_eq!(hello.0.len(), CLIENT_HELLO_FIXED_SIZE + sid.len());
        assert_eq!(read_u32(&hello.0, 0).unwrap(), u32::try_from(hello.0.len()).unwrap());
        assert_eq!(read_u32(&hello.0, 4).unwrap(), CLIENT_HELLO);
        assert_eq!(hello.0[8], 0);
        assert_eq!(read_u32(&hello.0, 9).unwrap(), u32::try_from(sid.len()).unwrap());
        assert_eq!(&hello.0[13..17], sid);
        assert_eq!(hello.0[17], 0);

        let ack = ControlPdu::client_ack();
        assert_eq!(ack.0, [9, 0, 0, 0, 3, 0, 0, 0, 0]);
    }

    #[test]
    fn server_hello_parses_sequential_utf16_suffixes() {
        let suffixes = [
            "0401f834-0d9a-49f4-afd9-3158a7ee4e72",
            "dc449cad-b6f8-4f78-8641-0c645cd5d382",
            "2baeed20-8797-4b9f-99cc-07244049de33",
        ];
        let encoded = suffixes
            .iter()
            .map(|suffix| suffix.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let total = SERVER_HELLO_FIXED_SIZE + encoded.iter().map(Vec::len).sum::<usize>();
        let mut payload = vec![0u8; SERVER_HELLO_FIXED_SIZE];
        payload[0..4].copy_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        payload[4..8].copy_from_slice(&SERVER_HELLO.to_le_bytes());
        let mut offset = SERVER_HELLO_FIXED_SIZE;
        for (idx, value) in encoded.iter().enumerate() {
            let field = 9 + idx * 8;
            payload[field..field + 4].copy_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
            payload[field + 4..field + 8].copy_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
            payload.extend_from_slice(value);
            offset += value.len();
        }

        let hello = parse_server_hello(&payload).expect("valid server hello");
        assert_eq!(hello.mapping_suffix, suffixes[0]);
        assert_eq!(hello.mutex_suffix, suffixes[1]);
        assert_eq!(hello.event_suffix, suffixes[2]);
    }

    #[test]
    fn bgra_frames_are_converted_and_oriented() {
        let top_down = BitmapLayout {
            width: 2,
            height: 2,
            stride: 8,
            pixel_bytes: 16,
            top_down: true,
        };
        let pixels = [
            3, 2, 1, 0, 6, 5, 4, 0, // top row
            9, 8, 7, 0, 12, 11, 10, 0, // bottom row
        ];
        assert_eq!(
            decode_bgra32(&pixels, top_down),
            [0x0001_0203, 0x0004_0506, 0x0007_0809, 0x000A_0B0C]
        );

        let bottom_up = BitmapLayout {
            top_down: false,
            ..top_down
        };
        assert_eq!(
            decode_bgra32(&pixels, bottom_up),
            [0x0007_0809, 0x000A_0B0C, 0x0001_0203, 0x0004_0506]
        );
    }
}

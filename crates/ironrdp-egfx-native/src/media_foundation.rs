//! H.264 decoding with the Media Foundation decoder transform.

use core::time::Duration;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame};
use ironrdp_egfx::pdu::{avc_to_annex_b_into, is_avc_format};
use tracing::{debug, warn};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MSH264DecoderMFT, IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
    MF_MT_SUBTYPE, MF_VERSION, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
    MFSTARTUP_NOSOCKET, MFShutdown, MFStartup, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264,
    MFVideoFormat_NV12,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};

/// Longest time a decode waits for the worker thread. Pictures normally decode in milliseconds,
/// and the caller is blocked meanwhile.
const DECODE_TIMEOUT: Duration = Duration::from_secs(1);
/// Longest time creating the decoder waits for Media Foundation to start.
///
/// Starting COM, Media Foundation and the transform cold can take seconds on a loaded machine.
/// The graphics pipeline creates its decoder once per connection and loses H.264 for the whole
/// session when this expires, so it allows for that rather than for the usual start.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// Spacing of the synthetic sample times, in 100-ns units (30 frames per second).
const SAMPLE_DURATION: i64 = 333_333;

pub use ironrdp_egfx::decode::YuvMatrix;

/// Luma scale and offset, then V-to-R, U-to-G, V-to-G and U-to-B factors, in 16.16 fixed point.
fn coefficients(matrix: YuvMatrix) -> (i32, i32, [i32; 4]) {
    match matrix {
        // 255/219, 1.596, 0.392, 0.813, 2.017
        YuvMatrix::Bt601Limited => (76_309, 16, [104_597, 25_675, 53_279, 132_201]),
        // 1, 1.5748, 0.1873, 0.4681, 1.8556
        YuvMatrix::Bt709Full => (65_536, 0, [103_206, 12_276, 30_679, 121_609]),
    }
}

enum Command {
    Decode(Vec<u8>, mpsc::SyncSender<DecoderResult<Picture>>),
    Reset,
}

/// A decoded NV12 picture.
struct Picture {
    pixels: Vec<u8>,
    format: OutputFormat,
}

/// H.264 decoder backed by the Microsoft H.264 decoder transform shipped with Windows.
///
/// [`H264Decoder::decode`] converts pictures with the matrix given at creation, and
/// [`H264Decoder::decode_yuv420`] returns their planes.
pub struct MediaFoundationH264Decoder {
    commands: Option<mpsc::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
    matrix: YuvMatrix,
}

impl MediaFoundationH264Decoder {
    /// Creates a decoder converting pictures with `matrix`.
    ///
    /// Fails when Media Foundation or its H.264 decoder is unavailable.
    pub fn new(matrix: YuvMatrix) -> DecoderResult<Self> {
        let (commands, command_receiver) = mpsc::channel();
        let (ready, ready_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("h264-decoder".to_owned())
            .spawn(move || worker_main(&command_receiver, &ready))
            .map_err(|error| DecoderError::new("failed to spawn the H.264 decoder thread", error))?;
        match ready_receiver.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                commands: Some(commands),
                thread: Some(thread),
                matrix,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(DecoderError::new(
                    "failed to create the Media Foundation H.264 decoder",
                    error,
                ))
            }
            Err(_) => Err(DecoderError::msg(
                "the Media Foundation H.264 decoder did not start in time",
            )),
        }
    }

    fn decode_picture(&mut self, data: &[u8]) -> DecoderResult<Picture> {
        let commands = self
            .commands
            .as_ref()
            .ok_or_else(|| DecoderError::msg("the H.264 decoder thread has stopped"))?;
        let (reply, response) = mpsc::sync_channel(1);
        commands
            .send(Command::Decode(data.to_vec(), reply))
            .map_err(|_| DecoderError::msg("the H.264 decoder thread has stopped"))?;
        match response.recv_timeout(DECODE_TIMEOUT) {
            Ok(result) => result,
            Err(_) => {
                // A hung transform would stall every later sample, and joining it would block the
                // session: later decodes fail at once and the thread is left to end on its own.
                self.commands = None;
                drop(self.thread.take());
                Err(DecoderError::msg("the H.264 decoder did not answer and was abandoned"))
            }
        }
    }
}

impl H264Decoder for MediaFoundationH264Decoder {
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
        let Picture { pixels, format } = self.decode_picture(data)?;
        let (Ok(width), Ok(height)) = (usize::try_from(format.width), usize::try_from(format.height)) else {
            return Err(DecoderError::msg("H.264 picture dimensions overflow"));
        };
        let rgba = nv12_to_rgba(&pixels, width, height, format.stride, self.matrix)
            .ok_or_else(|| DecoderError::msg("the H.264 picture is smaller than its format"))?;
        Ok(DecodedFrame::new(rgba, format.width, format.height))
    }

    fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
        let Picture { pixels, format } = self.decode_picture(data)?;
        nv12_to_yuv420(&pixels, format)
    }

    fn supports_yuv420(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Reset);
        }
    }
}

impl Drop for MediaFoundationH264Decoder {
    fn drop(&mut self) {
        // Closing the command channel ends the worker after its current picture.
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn worker_main(commands: &mpsc::Receiver<Command>, ready: &mpsc::SyncSender<Result<(), windows::core::Error>>) {
    let _media_foundation = match MediaFoundationThread::initialize() {
        Ok(media_foundation) => media_foundation,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut transform = match Transform::create() {
        Ok(transform) => transform,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    while let Ok(command) = commands.recv() {
        match command {
            Command::Decode(data, reply) => {
                let _ = reply.send(transform.decode(&data));
            }
            Command::Reset => transform.flush(),
        }
    }
}

/// Initializes COM for the multithreaded apartment and starts Media Foundation on this thread.
struct MediaFoundationThread;

impl MediaFoundationThread {
    fn initialize() -> windows::core::Result<Self> {
        // SAFETY: balanced by `CoUninitialize` in `Drop` or below.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
        // SAFETY: COM is initialized on this thread.
        if let Err(error) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) } {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for MediaFoundationThread {
    fn drop(&mut self) {
        // SAFETY: balances the successful `MFStartup` in `initialize`.
        let _ = unsafe { MFShutdown() };
        // SAFETY: balances the successful `CoInitializeEx` in `initialize`.
        unsafe { CoUninitialize() };
    }
}

/// Layout of the NV12 pictures the transform produces.
#[derive(Debug, Clone, Copy)]
struct OutputFormat {
    width: u32,
    height: u32,
    stride: usize,
}

/// The decoder transform. Lives on the worker thread only.
struct Transform {
    transform: IMFTransform,
    output: OutputFormat,
    sample_time: i64,
    annex_b: Vec<u8>,
}

impl Transform {
    fn create() -> windows::core::Result<Self> {
        // SAFETY: COM is initialized on the worker thread.
        let transform: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER) }?;
        // SAFETY: valid transform.
        if let Ok(attributes) = unsafe { transform.GetAttributes() } {
            // SAFETY: valid attribute store; low-latency mode returns each picture with its sample.
            if let Err(error) = unsafe { attributes.SetUINT32(&MF_LOW_LATENCY, 1) } {
                debug!(%error, "H.264 decoder rejected low-latency mode");
            }
        }

        // SAFETY: creates an empty media type.
        let input = unsafe { MFCreateMediaType() }?;
        // SAFETY: valid media type and static keys.
        unsafe { input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video) }?;
        // SAFETY: as above.
        unsafe { input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264) }?;
        // SAFETY: valid transform and media type.
        unsafe { transform.SetInputType(0, &input, 0) }?;

        let output = select_output(&transform)?;
        // SAFETY: valid, configured transform.
        unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }?;
        // SAFETY: as above.
        unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }?;
        Ok(Self {
            transform,
            output,
            sample_time: 0,
            annex_b: Vec::new(),
        })
    }

    fn decode(&mut self, data: &[u8]) -> DecoderResult<Picture> {
        let data = if is_avc_format(data) {
            avc_to_annex_b_into(data, &mut self.annex_b);
            core::mem::take(&mut self.annex_b)
        } else {
            data.to_vec()
        };
        let sample = self
            .input_sample(&data)
            .map_err(|error| DecoderError::new("failed to prepare the H.264 sample", error))?;
        self.annex_b = data;

        let mut frames = Vec::new();
        // SAFETY: valid transform and sample.
        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_NOTACCEPTING => {
                // Collect the pending pictures, then retry.
                frames.extend(self.drain()?);
                // SAFETY: as above.
                unsafe { self.transform.ProcessInput(0, &sample, 0) }
                    .map_err(|error| DecoderError::new("the H.264 decoder rejected the sample", error))?;
            }
            Err(error) => return Err(DecoderError::new("the H.264 decoder rejected the sample", error)),
        }
        frames.extend(self.drain()?);
        frames
            .pop()
            .ok_or_else(|| DecoderError::msg("the H.264 decoder returned no picture"))
    }

    fn flush(&mut self) {
        // SAFETY: valid transform.
        if let Err(error) = unsafe { self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) } {
            warn!(%error, "Failed to flush the H.264 decoder");
        }
    }

    fn input_sample(&mut self, data: &[u8]) -> windows::core::Result<IMFSample> {
        let length = u32::try_from(data.len()).map_err(|_| windows::core::Error::empty())?;
        // SAFETY: creates a buffer of the sample's size.
        let buffer = unsafe { MFCreateMemoryBuffer(length.max(1)) }?;
        let mut pointer = core::ptr::null_mut();
        // SAFETY: valid buffer; the out pointer is a valid local.
        unsafe { buffer.Lock(&mut pointer, None, None) }?;
        if !pointer.is_null() {
            // SAFETY: the locked buffer holds at least `length` writable bytes until `Unlock`.
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len()) };
        }
        // SAFETY: balances the successful `Lock`.
        unsafe { buffer.Unlock() }?;
        // SAFETY: the buffer now holds `length` bytes.
        unsafe { buffer.SetCurrentLength(length) }?;
        // SAFETY: creates an empty sample.
        let sample = unsafe { MFCreateSample() }?;
        // SAFETY: valid sample and buffer.
        unsafe { sample.AddBuffer(&buffer) }?;
        // SAFETY: valid sample.
        unsafe { sample.SetSampleTime(self.sample_time) }?;
        // SAFETY: valid sample.
        unsafe { sample.SetSampleDuration(SAMPLE_DURATION) }?;
        self.sample_time = self.sample_time.saturating_add(SAMPLE_DURATION);
        Ok(sample)
    }

    /// Returns the pictures the transform has ready.
    fn drain(&mut self) -> DecoderResult<Vec<Picture>> {
        let mut frames = Vec::new();
        loop {
            // SAFETY: valid transform.
            let info = unsafe { self.transform.GetOutputStreamInfo(0) }
                .map_err(|error| DecoderError::new("failed to query the H.264 decoder output", error))?;
            let provides_samples = info.dwFlags & flag_bits(MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0) != 0;
            let output_sample = if provides_samples {
                None
            } else {
                Some(
                    output_sample(info.cbSize)
                        .map_err(|error| DecoderError::new("failed to allocate an H.264 picture", error))?,
                )
            };
            let mut output = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: core::mem::ManuallyDrop::new(output_sample),
                dwStatus: 0,
                pEvents: core::mem::ManuallyDrop::new(None),
            }];
            let mut status = 0u32;
            // SAFETY: valid transform; the array holds one initialized buffer description.
            let result = unsafe { self.transform.ProcessOutput(0, &mut output, &mut status) };
            let [output] = output;
            let picture = core::mem::ManuallyDrop::into_inner(output.pSample);
            drop(core::mem::ManuallyDrop::into_inner(output.pEvents));
            match result {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(frames),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    self.output = select_output(&self.transform)
                        .map_err(|error| DecoderError::new("failed to follow an H.264 format change", error))?;
                    debug!(output = ?self.output, "H.264 decoder output format changed");
                    continue;
                }
                Err(error) => return Err(DecoderError::new("the H.264 decoder failed", error)),
            }
            let Some(picture) = picture else {
                continue;
            };
            let pixels =
                sample_bytes(&picture).map_err(|error| DecoderError::new("failed to read an H.264 picture", error))?;
            frames.push(Picture {
                pixels,
                format: self.output,
            });
        }
    }
}

/// Selects the NV12 output type and returns its layout.
fn select_output(transform: &IMFTransform) -> windows::core::Result<OutputFormat> {
    for index in 0u32.. {
        // SAFETY: valid transform; enumeration ends with an error.
        let candidate: IMFMediaType = unsafe { transform.GetOutputAvailableType(0, index) }?;
        // SAFETY: valid media type and static key.
        if unsafe { candidate.GetGUID(&MF_MT_SUBTYPE) }? != MFVideoFormat_NV12 {
            continue;
        }
        // SAFETY: valid transform and media type.
        unsafe { transform.SetOutputType(0, &candidate, 0) }?;
        // SAFETY: valid media type and static key.
        let frame_size = unsafe { candidate.GetUINT64(&MF_MT_FRAME_SIZE) }?;
        let width = u32::try_from(frame_size >> 32).map_err(|_| windows::core::Error::empty())?;
        let height = u32::try_from(frame_size & 0xFFFF_FFFF).map_err(|_| windows::core::Error::empty())?;
        // SAFETY: valid media type and static key.
        let stride = unsafe { candidate.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .ok()
            .and_then(|stride| usize::try_from(stride).ok())
            .filter(|stride| *stride != 0 && *stride < 1 << 16)
            .unwrap_or_else(|| usize::try_from(width).unwrap_or(0));
        return Ok(OutputFormat { width, height, stride });
    }
    Err(windows::core::Error::empty())
}

fn output_sample(size: u32) -> windows::core::Result<IMFSample> {
    // SAFETY: creates an empty sample.
    let sample = unsafe { MFCreateSample() }?;
    // SAFETY: creates a buffer of the size the transform asks for.
    let buffer = unsafe { MFCreateMemoryBuffer(size.max(1)) }?;
    // SAFETY: valid sample and buffer.
    unsafe { sample.AddBuffer(&buffer) }?;
    Ok(sample)
}

fn sample_bytes(sample: &IMFSample) -> windows::core::Result<Vec<u8>> {
    // SAFETY: valid sample.
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
    let mut pointer = core::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: valid buffer; out pointers are valid locals.
    unsafe { buffer.Lock(&mut pointer, None, Some(&mut length)) }?;
    let data = usize::try_from(length)
        .ok()
        .filter(|_| !pointer.is_null())
        .map(|length| {
            // SAFETY: the locked buffer holds `length` readable bytes until `Unlock`.
            unsafe { core::slice::from_raw_parts(pointer, length) }.to_vec()
        });
    // SAFETY: balances the successful `Lock`.
    unsafe { buffer.Unlock() }?;
    data.ok_or_else(windows::core::Error::empty)
}

/// Splits an NV12 picture into the planes of a YUV 4:2:0 picture.
fn nv12_to_yuv420(nv12: &[u8], format: OutputFormat) -> DecoderResult<Yuv420Frame> {
    let OutputFormat { width, height, stride } = format;
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(DecoderError::msg("H.264 picture dimensions exceed 65535"));
    };
    let chroma_width = usize::from(width.div_ceil(2));
    let chroma_rows = usize::from(height.div_ceil(2));
    let luma_size = stride * usize::from(height);
    let Some(interleaved) = nv12.get(luma_size..luma_size + stride * chroma_rows) else {
        return Err(DecoderError::msg("the H.264 picture is smaller than its format"));
    };
    let mut u = Vec::with_capacity(chroma_width * chroma_rows);
    let mut v = Vec::with_capacity(chroma_width * chroma_rows);
    for row in interleaved.chunks_exact(stride) {
        for pair in row.chunks_exact(2).take(chroma_width) {
            u.push(pair[0]);
            v.push(pair[1]);
        }
    }
    Yuv420Frame::new(width, height, nv12[..luma_size].to_vec(), stride, u, v, chroma_width)
}

/// Converts an NV12 picture with `stride`-byte rows to tightly packed RGBA.
///
/// Returns `None` when `nv12` is too small for the picture.
fn nv12_to_rgba(nv12: &[u8], width: usize, height: usize, stride: usize, matrix: YuvMatrix) -> Option<Vec<u8>> {
    let luma_size = stride.checked_mul(height)?;
    let chroma_size = stride.checked_mul(height.div_ceil(2))?;
    if stride < width || nv12.len() < luma_size.checked_add(chroma_size)? {
        return None;
    }
    let (luma, chroma) = nv12.split_at(luma_size);
    let (scale, black, [v_to_r, u_to_g, v_to_g, u_to_b]) = coefficients(matrix);
    let mut rgba = Vec::with_capacity(width.checked_mul(height)?.checked_mul(4)?);
    for y in 0..height {
        let luma_row = &luma[y * stride..][..width];
        let chroma_row = &chroma[(y / 2) * stride..][..width.next_multiple_of(2).min(stride)];
        for (x, luma) in luma_row.iter().enumerate() {
            let pair = (x / 2) * 2;
            let (Some(u), Some(v)) = (chroma_row.get(pair), chroma_row.get(pair + 1)) else {
                return None;
            };
            let luma = (i32::from(*luma) - black) * scale + (1 << 15);
            let u = i32::from(*u) - 128;
            let v = i32::from(*v) - 128;
            rgba.push(clamp_channel((luma + v_to_r * v) >> 16));
            rgba.push(clamp_channel((luma - u_to_g * u - v_to_g * v) >> 16));
            rgba.push(clamp_channel((luma + u_to_b * u) >> 16));
            rgba.push(0xFF);
        }
    }
    Some(rgba)
}

fn clamp_channel(value: i32) -> u8 {
    // The clamped value is its own low byte.
    let [low, ..] = value.clamp(0, 255).to_le_bytes();
    low
}

fn flag_bits(flag: i32) -> u32 {
    u32::from_ne_bytes(flag.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nv12_converts_with_both_matrices() {
        // A 2x2 picture with a 4-byte stride: limited-range white, then one chroma pair.
        let nv12 = [235, 235, 0, 0, 235, 235, 0, 0, 128, 128, 0, 0];
        let rgba = nv12_to_rgba(&nv12, 2, 2, 4, YuvMatrix::Bt601Limited).unwrap();
        assert_eq!(rgba, [255, 255, 255, 255].repeat(4));

        let nv12 = [0, 0, 0, 0, 128, 128];
        let rgba = nv12_to_rgba(&nv12, 2, 2, 2, YuvMatrix::Bt709Full).unwrap();
        assert_eq!(rgba, [0, 0, 0, 255].repeat(4));

        assert!(nv12_to_rgba(&nv12[..5], 2, 2, 2, YuvMatrix::Bt709Full).is_none());
    }
}

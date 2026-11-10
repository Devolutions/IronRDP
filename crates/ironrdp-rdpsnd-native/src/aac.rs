//! AAC decoding with the Windows Media Foundation AAC decoder transform.
//!
//! A format carrying `HEAACWAVEINFO` extra bytes is handed to Media Foundation as is. Windows
//! servers offer `WAVE_FORMAT_AAC_MS` without extra bytes, so the decoder is then configured from
//! the first frame: ADTS frames describe themselves, and raw frames get an AAC-LC
//! `AudioSpecificConfig` built from the sample rate and channel count. The transform lives on a
//! worker thread that owns its COM apartment, and each call waits for the decoded PCM so the caller
//! can account for it in the playback latency.

use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use ironrdp_core::encode_vec;
use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};
use tracing::{debug, warn};
use windows::Win32::Media::Audio::WAVEFORMATEX;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MSAACDecMFT, IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AAC_PAYLOAD_TYPE, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_MT_USER_DATA, MF_VERSION, MFAudioFormat_AAC,
    MFAudioFormat_PCM, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFInitMediaTypeFromWaveFormatEx,
    MFMediaType_Audio, MFSTARTUP_NOSOCKET, MFShutdown, MFStartup, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};

use crate::error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};

/// Smallest output buffer offered to the transform, enough for one HE-AAC frame.
const MIN_OUTPUT_BUFFER: u32 = 2048 /* frames */ * 2 /* channels */ * 2 /* bytes per sample */;

/// Size of the `HEAACWAVEINFO` fields that follow `WAVEFORMATEX` (payload type, profile, structure
/// type and reserved fields), before the `AudioSpecificConfig`.
const HEAACWAVEINFO_SIZE: usize = 2 /* wPayloadType */
    + 2 /* wAudioProfileLevelIndication */
    + 2 /* wStructType */
    + 2 /* wReserved1 */
    + 4 /* dwReserved2 */;

/// `HEAACWAVEINFO.wPayloadType` for raw AAC frames.
const PAYLOAD_RAW: u16 = 0;

/// `HEAACWAVEINFO.wPayloadType` for ADTS frames.
const PAYLOAD_ADTS: u16 = 1;

/// MPEG-4 audio object type of AAC-LC.
const AAC_LC_OBJECT_TYPE: u16 = 2;

/// Highest sample rate the Media Foundation AAC decoder outputs.
const MAX_SAMPLE_RATE: u32 = 48_000;

/// MPEG-4 sampling frequency table, indexed by `samplingFrequencyIndex`.
const SAMPLING_FREQUENCIES: [u32; 12] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
];

type Reply = mpsc::SyncSender<Result<Vec<i16>, String>>;

/// Channel count and sample rate the decoder must output.
#[derive(Debug, Clone, Copy)]
struct PcmLayout {
    channels: u16,
    sample_rate: u32,
}

/// How the decoder input type is described to Media Foundation.
enum InputType {
    /// A complete `WAVEFORMATEX` with its `HEAACWAVEINFO` extra bytes.
    WaveFormat(Vec<u8>),
    /// Attributes built from the format and the first frame.
    Attributes {
        channels: u16,
        sample_rate: u32,
        payload_type: u16,
        user_data: Vec<u8>,
    },
}

/// The worker thread that owns the transform, and the channel feeding it.
struct Worker {
    commands: mpsc::Sender<(Vec<u8>, Reply)>,
    thread: JoinHandle<()>,
}

/// AAC decoder backed by Media Foundation, started on the first frame.
pub(crate) struct AacDecoder {
    format: AudioFormat,
    worker: Option<Worker>,
}

impl AacDecoder {
    /// Returns whether `format` is an AAC format this decoder accepts, and Media Foundation is present.
    pub(crate) fn supports(format: &AudioFormat) -> bool {
        format.format == WaveFormat::AAC_MS
            && matches!(format.n_channels, 1 | 2)
            // The decoder does not output rates above 48 kHz.
            && format.n_samples_per_sec <= MAX_SAMPLE_RATE
            && (has_wave_info(format) || frequency_index(format.n_samples_per_sec).is_some())
            && media_foundation_available()
    }

    pub(crate) fn new(format: &AudioFormat) -> RdpsndNativeResult<Self> {
        if !Self::supports(format) {
            return Err(RdpsndNativeError::new(
                "unsupported AAC format",
                RdpsndNativeErrorKind::UnsupportedFormat,
            ));
        }
        Ok(Self {
            format: format.clone(),
            worker: None,
        })
    }

    fn start(&mut self, first_frame: &[u8]) -> RdpsndNativeResult<()> {
        // ADTS frames start with a 12-bit syncword and a zero layer field.
        let adts = matches!(first_frame, [0xFF, second, ..] if second & 0xF6 == 0xF0);
        let input = if has_wave_info(&self.format) {
            InputType::WaveFormat(encode_vec(&self.format).map_err(|e| {
                RdpsndNativeError::new("encoding the AAC wave format", RdpsndNativeErrorKind::UnsupportedFormat)
                    .with_source(e)
            })?)
        } else {
            let payload_type = if adts { PAYLOAD_ADTS } else { PAYLOAD_RAW };
            let mut user_data = heaacwaveinfo(payload_type);
            if !adts {
                let index = frequency_index(self.format.n_samples_per_sec).expect("checked by supports");
                let config = (AAC_LC_OBJECT_TYPE << 11) | (index << 7) | (self.format.n_channels << 3);
                user_data.extend_from_slice(&config.to_be_bytes());
            }
            InputType::Attributes {
                channels: self.format.n_channels,
                sample_rate: self.format.n_samples_per_sec,
                payload_type,
                user_data,
            }
        };
        debug!(
            first_bytes = ?&first_frame[..first_frame.len().min(8)],
            adts,
            "Starting the AAC decoder"
        );

        let (commands, receiver) = mpsc::channel::<(Vec<u8>, Reply)>();
        let (ready, started) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("rdpsnd-aac".to_owned())
            .spawn({
                let output = PcmLayout {
                    channels: self.format.n_channels,
                    sample_rate: self.format.n_samples_per_sec,
                };
                move || run_worker(&input, output, &ready, &receiver)
            })
            .map_err(|e| {
                RdpsndNativeError::new("spawning the AAC decoder", RdpsndNativeErrorKind::Decoder).with_source(e)
            })?;

        match started.recv() {
            Ok(Ok(())) => {
                self.worker = Some(Worker {
                    commands,
                    thread: worker,
                });
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(
                    RdpsndNativeError::new("creating the AAC decoder", RdpsndNativeErrorKind::Decoder)
                        .with_source(error),
                )
            }
            Err(_) => {
                let _ = worker.join();
                Err(RdpsndNativeError::new(
                    "the AAC decoder exited during startup",
                    RdpsndNativeErrorKind::Decoder,
                ))
            }
        }
    }

    /// Decodes one AAC frame and appends the interleaved samples to `out`.
    pub(crate) fn decode(&mut self, data: &[u8], out: &mut Vec<i16>) -> RdpsndNativeResult<()> {
        if self.worker.is_none() {
            self.start(data)?;
        }
        let worker = self.worker.as_ref().expect("started above");
        let (reply, response) = mpsc::sync_channel(1);
        if worker.commands.send((data.to_vec(), reply)).is_err() {
            return Err(RdpsndNativeError::new(
                "the AAC decoder stopped",
                RdpsndNativeErrorKind::Decoder,
            ));
        }
        match response.recv() {
            Ok(Ok(samples)) => {
                out.extend_from_slice(&samples);
                Ok(())
            }
            Ok(Err(error)) => Err(
                RdpsndNativeError::new("decoding an AAC frame", RdpsndNativeErrorKind::Decoder)
                    .with_source(DecodeFailure(error)),
            ),
            Err(_) => Err(RdpsndNativeError::new(
                "the AAC decoder stopped",
                RdpsndNativeErrorKind::Decoder,
            )),
        }
    }
}

impl Drop for AacDecoder {
    fn drop(&mut self) {
        if let Some(Worker { commands, thread }) = self.worker.take() {
            // Closing the command channel ends the worker loop.
            drop(commands);
            let _ = thread.join();
        }
    }
}

#[derive(Debug)]
struct DecodeFailure(String);

impl core::fmt::Display for DecodeFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for DecodeFailure {}

/// Probes once whether the Media Foundation AAC decoder can be instantiated.
fn media_foundation_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let probe = thread::spawn(|| {
            let Ok(_thread) = MediaFoundationThread::initialize() else {
                return false;
            };
            // SAFETY: COM and Media Foundation are initialized on this thread.
            unsafe { CoCreateInstance::<_, IMFTransform>(&CLSID_MSAACDecMFT, None, CLSCTX_INPROC_SERVER) }.is_ok()
        });
        let available = probe.join().unwrap_or(false);
        debug!(available, "Probed the Media Foundation AAC decoder");
        available
    })
}

fn has_wave_info(format: &AudioFormat) -> bool {
    format
        .data
        .as_ref()
        .is_some_and(|data| HEAACWAVEINFO_SIZE <= data.len())
}

fn frequency_index(sample_rate: u32) -> Option<u16> {
    let index = SAMPLING_FREQUENCIES.iter().position(|rate| *rate == sample_rate)?;
    u16::try_from(index).ok()
}

/// The `HEAACWAVEINFO` fields that follow `WAVEFORMATEX`, for `MF_MT_USER_DATA`.
fn heaacwaveinfo(payload_type: u16) -> Vec<u8> {
    let mut info = Vec::with_capacity(HEAACWAVEINFO_SIZE + 2);
    info.extend_from_slice(&payload_type.to_le_bytes()); // wPayloadType
    info.extend_from_slice(&0xFEu16.to_le_bytes()); // wAudioProfileLevelIndication: unspecified
    info.extend_from_slice(&0u16.to_le_bytes()); // wStructType: AudioSpecificConfig follows
    info.extend_from_slice(&0u16.to_le_bytes()); // wReserved1
    info.extend_from_slice(&0u32.to_le_bytes()); // dwReserved2
    info
}

fn run_worker(
    input: &InputType,
    output: PcmLayout,
    ready: &mpsc::SyncSender<Result<(), windows::core::Error>>,
    commands: &mpsc::Receiver<(Vec<u8>, Reply)>,
) {
    let _thread = match MediaFoundationThread::initialize() {
        Ok(thread) => thread,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut transform = match Transform::create(input, output) {
        Ok(transform) => transform,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    while let Ok((data, reply)) = commands.recv() {
        let _ = reply.send(transform.decode(&data).map_err(|error| error.to_string()));
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

/// The decoder transform. Lives on the worker thread only.
struct Transform {
    transform: IMFTransform,
    output: PcmLayout,
    sample_time: i64,
}

impl Transform {
    fn create(input_type: &InputType, output: PcmLayout) -> windows::core::Result<Self> {
        // SAFETY: COM is initialized on the worker thread.
        let transform: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSAACDecMFT, None, CLSCTX_INPROC_SERVER) }?;

        // SAFETY: creates an empty media type.
        let input = unsafe { MFCreateMediaType() }?;
        match input_type {
            InputType::WaveFormat(waveformat) => {
                let size = u32::try_from(waveformat.len()).map_err(|_| windows::core::Error::empty())?;
                // SAFETY: `waveformat` is a complete WAVEFORMATEX followed by its cbSize extra bytes,
                // and WAVEFORMATEX is byte-packed, so any address is suitably aligned.
                unsafe { MFInitMediaTypeFromWaveFormatEx(&input, waveformat.as_ptr().cast::<WAVEFORMATEX>(), size) }?;
            }
            InputType::Attributes {
                channels,
                sample_rate,
                payload_type,
                user_data,
            } => {
                // SAFETY: valid media type and static keys.
                unsafe { input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio) }?;
                // SAFETY: as above.
                unsafe { input.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC) }?;
                // SAFETY: as above.
                unsafe { input.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, u32::from(*channels)) }?;
                // SAFETY: as above.
                unsafe { input.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, *sample_rate) }?;
                // SAFETY: as above.
                unsafe { input.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, u32::from(*payload_type)) }?;
                // SAFETY: as above; the blob is copied.
                unsafe { input.SetBlob(&MF_MT_USER_DATA, user_data) }?;
            }
        }
        // SAFETY: valid transform and media type.
        unsafe { transform.SetInputType(0, &input, 0) }?;

        select_pcm_output(&transform, output)?;
        // SAFETY: valid, configured transform.
        unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }?;
        // SAFETY: as above.
        unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }?;
        Ok(Self {
            transform,
            output,
            sample_time: 0,
        })
    }

    fn decode(&mut self, data: &[u8]) -> windows::core::Result<Vec<i16>> {
        let sample = self.input_sample(data)?;
        let mut pcm = Vec::new();
        // SAFETY: valid transform and sample.
        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_NOTACCEPTING => {
                self.drain(&mut pcm)?;
                // SAFETY: as above.
                unsafe { self.transform.ProcessInput(0, &sample, 0) }?;
            }
            Err(error) => return Err(error),
        }
        self.drain(&mut pcm)?;
        Ok(pcm)
    }

    fn input_sample(&mut self, data: &[u8]) -> windows::core::Result<IMFSample> {
        let length = u32::try_from(data.len()).map_err(|_| windows::core::Error::empty())?;
        // SAFETY: creates a buffer of the frame's size.
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
        // SAFETY: valid sample. Timestamps only need to increase; the decoder does not pace output.
        unsafe { sample.SetSampleTime(self.sample_time) }?;
        self.sample_time = self.sample_time.saturating_add(1);
        Ok(sample)
    }

    /// Appends the PCM the transform has ready.
    fn drain(&mut self, pcm: &mut Vec<i16>) -> windows::core::Result<()> {
        loop {
            // SAFETY: valid transform.
            let info = unsafe { self.transform.GetOutputStreamInfo(0) }?;
            let provides_samples_flag = u32::from_ne_bytes(MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0.to_ne_bytes());
            let provides_samples = info.dwFlags & provides_samples_flag != 0;
            let output_sample = if provides_samples {
                None
            } else {
                Some(output_sample(info.cbSize.max(MIN_OUTPUT_BUFFER))?)
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
            let decoded = core::mem::ManuallyDrop::into_inner(output.pSample);
            drop(core::mem::ManuallyDrop::into_inner(output.pEvents));
            match result {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    warn!("AAC decoder output format changed");
                    // A stream turning out to be HE-AAC doubles the rate or adds a channel; the
                    // playback stream was opened for the advertised layout, so that fails here.
                    select_pcm_output(&self.transform, self.output)?;
                    continue;
                }
                Err(error) => return Err(error),
            }
            if let Some(decoded) = decoded {
                let bytes = sample_bytes(&decoded)?;
                pcm.extend(
                    bytes
                        .chunks_exact(2)
                        .map(|sample| i16::from_le_bytes([sample[0], sample[1]])),
                );
            }
        }
    }
}

/// Selects the 16-bit PCM output type with the `output` channel count and sample rate.
///
/// Fails when the decoder only offers another layout, which the playback stream could not play.
fn select_pcm_output(transform: &IMFTransform, output: PcmLayout) -> windows::core::Result<()> {
    for index in 0u32.. {
        // SAFETY: valid transform; enumeration ends with an error.
        let candidate: IMFMediaType = unsafe { transform.GetOutputAvailableType(0, index) }?;
        // SAFETY: valid media type and static keys.
        let is_pcm16 = unsafe { candidate.GetGUID(&MF_MT_SUBTYPE) }? == MFAudioFormat_PCM
            && unsafe { candidate.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }.is_ok_and(|bits| bits == 16)
            && unsafe { candidate.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }
                .is_ok_and(|channels| channels == u32::from(output.channels))
            && unsafe { candidate.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }
                .is_ok_and(|rate| rate == output.sample_rate);
        if is_pcm16 {
            // SAFETY: valid transform and media type.
            return unsafe { transform.SetOutputType(0, &candidate, 0) };
        }
    }
    Err(windows::core::Error::empty())
}

fn output_sample(size: u32) -> windows::core::Result<IMFSample> {
    // SAFETY: creates an empty sample.
    let sample = unsafe { MFCreateSample() }?;
    // SAFETY: creates a buffer of the requested size.
    let buffer = unsafe { MFCreateMemoryBuffer(size) }?;
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

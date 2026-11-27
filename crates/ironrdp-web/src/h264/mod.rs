//! H.264 decoding for the graphics pipeline through the browser's [WebCodecs] `VideoDecoder`.
//!
//! WebCodecs returns pictures through a callback, so the decoder is asynchronous
//! ([`H264Decoder::submit`]): the planes of each picture are copied out in the background, queued,
//! and announced to the session loop, which then polls the graphics pipeline.
//! The pipeline converts the planes to RGB with the transform MS-RDPEGFX specifies.
//!
//! The JS objects are not `Send`, which [`H264Decoder`] must be, so they live in a thread-local
//! registry and [`WebCodecsDecoder`] only holds its key; callbacks find their decoder there too.
//!
//! [WebCodecs]: https://www.w3.org/TR/webcodecs/

mod stream;

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use ironrdp_core::MonotonicInstant;
use ironrdp_egfx::decode::{DecodeStatus, DecodedFrame, DecodedPicture, DecoderError, DecoderResult, H264Decoder};
use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use tracing::{debug, info, warn};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use self::stream::{Budget, Config, Failure, Pictures, Reservation, Sink, Stream};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = VideoDecoder)]
    type JsVideoDecoder;

    #[wasm_bindgen(constructor, js_class = "VideoDecoder", catch)]
    fn new(init: &Object) -> Result<JsVideoDecoder, JsValue>;

    #[wasm_bindgen(method, js_class = "VideoDecoder", catch)]
    fn configure(this: &JsVideoDecoder, config: &Object) -> Result<(), JsValue>;

    #[wasm_bindgen(method, js_class = "VideoDecoder", catch)]
    fn decode(this: &JsVideoDecoder, chunk: &JsEncodedVideoChunk) -> Result<(), JsValue>;

    #[wasm_bindgen(method, js_class = "VideoDecoder", catch)]
    fn flush(this: &JsVideoDecoder) -> Result<Promise, JsValue>;

    #[wasm_bindgen(method, js_class = "VideoDecoder", catch)]
    fn close(this: &JsVideoDecoder) -> Result<(), JsValue>;

    #[wasm_bindgen(static_method_of = JsVideoDecoder, js_class = "VideoDecoder", js_name = isConfigSupported, catch)]
    fn is_config_supported(config: &Object) -> Result<Promise, JsValue>;

    #[wasm_bindgen(js_name = EncodedVideoChunk)]
    type JsEncodedVideoChunk;

    #[wasm_bindgen(constructor, js_class = "EncodedVideoChunk", catch)]
    fn new(init: &Object) -> Result<JsEncodedVideoChunk, JsValue>;

    #[wasm_bindgen(js_name = VideoFrame)]
    type JsVideoFrame;

    #[wasm_bindgen(method, getter, js_class = "VideoFrame")]
    fn format(this: &JsVideoFrame) -> Option<String>;

    #[wasm_bindgen(method, getter, js_class = "VideoFrame", js_name = visibleRect)]
    fn visible_rect(this: &JsVideoFrame) -> Option<Object>;

    #[wasm_bindgen(method, getter, js_class = "VideoFrame")]
    fn timestamp(this: &JsVideoFrame) -> f64;

    #[wasm_bindgen(method, getter, js_class = "VideoFrame", js_name = colorSpace)]
    fn color_space(this: &JsVideoFrame) -> Option<Object>;

    #[wasm_bindgen(method, js_class = "VideoFrame", js_name = allocationSize, catch)]
    fn allocation_size(this: &JsVideoFrame, options: &Object) -> Result<f64, JsValue>;

    #[wasm_bindgen(method, js_class = "VideoFrame", js_name = copyTo, catch)]
    fn copy_to(this: &JsVideoFrame, destination: &Uint8Array, options: &Object) -> Result<Promise, JsValue>;

    #[wasm_bindgen(method, js_class = "VideoFrame", js_name = close)]
    fn close_frame(this: &JsVideoFrame);
}

/// Constrained Baseline, the profile AVC420 servers encode, at levels from 5.1 down.
const CODECS: [&str; 3] = ["avc1.42E033", "avc1.42E028", "avc1.42E01F"];

/// Upper bound on a picture side, the largest desktop RDP allows.
const MAX_PICTURE_SIDE: u32 = 8192;

/// Upper bound on the bytes of decoded pictures a session holds outside the graphics pipeline,
/// across all its decoders: copies in progress and pictures not polled yet.
const SESSION_PICTURE_BYTES: usize = 96 << 20;

/// A reading of the browser's monotonic clock.
pub(crate) fn now() -> MonotonicInstant {
    let milliseconds = web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now());
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Performance.now() is a non-negative millisecond count far below 2^64"
    )]
    MonotonicInstant::from_millis(milliseconds as u64)
}

/// The first codec string of [`CODECS`] the browser decodes, or `None` without WebCodecs or H.264 support.
pub(crate) async fn supported_codec() -> Option<&'static str> {
    if !Reflect::has(&js_sys::global(), &JsValue::from_str("VideoDecoder")).unwrap_or(false) {
        info!("The browser has no WebCodecs VideoDecoder; H.264 is off");
        return None;
    }
    for codec in CODECS {
        let config = Config {
            codec: codec.to_owned(),
            size: None,
        };
        let Ok(promise) = JsVideoDecoder::is_config_supported(&decoder_config(&config)) else {
            continue;
        };
        let supported = JsFuture::from(promise)
            .await
            .ok()
            .and_then(|support| Reflect::get(&support, &JsValue::from_str("supported")).ok())
            .is_some_and(|supported| supported.is_truthy());
        if supported {
            info!(codec, "The browser decodes H.264 with WebCodecs");
            return Some(codec);
        }
    }
    info!("The browser's VideoDecoder supports no Constrained Baseline H.264; H.264 is off");
    None
}

fn decoder_config(config: &Config) -> Object {
    let object = Object::new();
    set(&object, "codec", &JsValue::from_str(&config.codec));
    set(&object, "optimizeForLatency", &JsValue::TRUE);
    if let Some((width, height)) = config.size {
        set(&object, "codedWidth", &JsValue::from(width));
        set(&object, "codedHeight", &JsValue::from(height));
    }
    object
}

fn set(object: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(object, &JsValue::from_str(key), value);
}

fn get_u32(object: &JsValue, key: &str) -> Option<u32> {
    let value = Reflect::get(object, &JsValue::from_str(key)).ok()?.as_f64()?;
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "checked to be a whole number in range first"
    )]
    (value.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(&value)).then_some(value as u32)
}

/// The failure a `DOMException` reports, by its name.
fn failure_of(error: &JsValue) -> Failure {
    let name = Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default();
    Failure::from_name(&name)
}

struct OpenDecoder {
    decoder: JsVideoDecoder,
    _output: Closure<dyn FnMut(JsVideoFrame)>,
    _error: Closure<dyn FnMut(JsValue)>,
}

impl Drop for OpenDecoder {
    fn drop(&mut self) {
        // UNTESTED: a closed `VideoDecoder` calls neither callback again, so the closures can be
        // dropped with it; a callback after that throws in JS. No native test runs WebCodecs: when
        // changing this, check in a browser that closing and reopening on resize logs no error.
        let _ = self.decoder.close();
    }
}

/// The `VideoDecoder` of a registered decoder; its callbacks find the decoder by `id`.
struct JsSink {
    id: u32,
    decoder: Option<OpenDecoder>,
    /// Bumped by each open, so callbacks of a replaced `VideoDecoder` are ignored.
    opened: u64,
}

impl Sink for JsSink {
    fn open(&mut self, config: &Config) -> Result<(), Failure> {
        self.decoder = None;
        self.opened += 1;
        let (id, opened) = (self.id, self.opened);
        let output = Closure::<dyn FnMut(JsVideoFrame)>::new(move |frame: JsVideoFrame| on_output(id, opened, frame));
        let error = Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| on_error(id, opened, &error));
        let init = Object::new();
        set(&init, "output", output.as_ref());
        set(&init, "error", error.as_ref());
        let decoder = JsVideoDecoder::new(&init).map_err(|error| {
            warn!(?error, "VideoDecoder could not be created");
            Failure::Resource
        })?;
        let decoder = OpenDecoder {
            decoder,
            _output: output,
            _error: error,
        };
        if let Err(error) = decoder.decoder.configure(&decoder_config(config)) {
            warn!(?error, codec = config.codec, "VideoDecoder.configure failed");
            return Err(Failure::Unsupported);
        }
        debug!(codec = config.codec, size = ?config.size, "Opened a WebCodecs H.264 decoder");
        self.decoder = Some(decoder);
        Ok(())
    }

    fn decode(&mut self, chunk: &[u8], key: bool, timestamp: f64) -> Result<(), Failure> {
        let decoder = self.decoder.as_ref().ok_or(Failure::Corrupt)?;
        let init = Object::new();
        set(&init, "type", &JsValue::from_str(if key { "key" } else { "delta" }));
        set(&init, "timestamp", &JsValue::from_f64(timestamp));
        set(&init, "data", &Uint8Array::from(chunk));
        let chunk = JsEncodedVideoChunk::new(&init).map_err(|error| failure_of(&error))?;
        decoder.decoder.decode(&chunk).map_err(|error| {
            warn!(?error, "VideoDecoder.decode failed");
            failure_of(&error)
        })
    }

    fn flush(&mut self) -> Result<(), Failure> {
        let decoder = self.decoder.as_ref().ok_or(Failure::Corrupt)?;
        let promise = decoder.decoder.flush().map_err(|error| failure_of(&error))?;
        spawn_local(async move {
            if let Err(error) = JsFuture::from(promise).await {
                debug!(?error, "The WebCodecs flush ended early");
            }
        });
        Ok(())
    }

    fn close(&mut self) {
        self.decoder = None;
    }
}

struct Registered {
    sink: JsSink,
    stream: Stream,
    pictures: Pictures,
    budget: Rc<Budget>,
    notify: Rc<dyn Fn()>,
    /// An unsupported frame format was logged, so it is not logged for every frame.
    warned_format: bool,
    /// Expanding limited-range frames of a full-range stream was logged.
    warned_range: bool,
}

impl Registered {
    /// Gives queued chunks to the `VideoDecoder`, and fails the pictures a failure loses.
    fn pump(&mut self) {
        let before = self.stream.decoder_generation();
        let lost = self.stream.pump(&mut self.sink, now());
        self.after_stream(before, !lost.is_empty());
    }

    /// Fails every picture not returned yet when the `VideoDecoder` was closed or replaced.
    fn after_stream(&mut self, decoder_generation: u64, lost: bool) {
        if lost || self.stream.decoder_generation() != decoder_generation {
            self.pictures.fail_all("the WebCodecs decoder was closed");
            (self.notify)();
        }
    }

    fn submit(&mut self, data: &[u8]) -> DecoderResult<DecodeStatus> {
        let annex_b;
        let data = if ironrdp_egfx::pdu::is_avc_format(data) {
            annex_b = ironrdp_egfx::pdu::avc_to_annex_b(data);
            annex_b.as_slice()
        } else {
            data
        };
        let number = self.pictures.next_number()?;
        let before = self.stream.decoder_generation();
        let lost = self.stream.submit(&mut self.sink, data, number)?;
        self.after_stream(before, !lost.is_empty());
        self.pictures.submitted = number + 1;
        self.pump();
        Ok(DecodeStatus::Pending)
    }

    fn poll(&mut self) -> Option<DecoderResult<DecodedPicture>> {
        self.pump();
        self.pictures.poll()
    }

    fn flush(&mut self) {
        let before = self.stream.decoder_generation();
        let lost = self.stream.flush(&mut self.sink);
        self.after_stream(before, !lost.is_empty());
    }

    fn reset(&mut self) {
        self.stream.reset(&mut self.sink);
        self.pictures.reset();
    }

    /// Handles a picture the `VideoDecoder` output: starts copying its planes, or drops it.
    fn output(&mut self, frame: JsVideoFrame) {
        self.stream.on_output();
        let timestamp = frame.timestamp();
        if timestamp < 0.0 {
            frame.close_frame();
            self.pump();
            return;
        }
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "non-negative timestamps are the submission numbers"
        )]
        let number = timestamp as u64;
        let generation = self.pictures.generation;
        if self.pictures.awaits(generation, number) {
            match self.prepare_copy(&frame, number) {
                Ok(copy) => copy.start(self.sink.id, generation, number, frame),
                Err(error) => {
                    frame.close_frame();
                    self.pictures.finish(generation, number, Err(error), None);
                    (self.notify)();
                }
            }
        } else {
            frame.close_frame();
        }
        self.pump();
    }

    /// Checks the visible size and format of picture `number`, reserves its bytes in the session
    /// budget, and allocates the buffer for its planes.
    fn prepare_copy(&mut self, frame: &JsVideoFrame, number: u64) -> DecoderResult<Copy> {
        let format = frame.format().unwrap_or_default();
        if !matches!(format.as_str(), "I420" | "I420A" | "NV12") {
            if !self.warned_format {
                self.warned_format = true;
                warn!(
                    format,
                    "The browser decodes H.264 to a format without an MS-RDPEGFX conversion; dropping its pictures"
                );
            }
            return Err(DecoderError::msg(format!("unsupported VideoFrame format {format:?}")));
        }
        let rect = frame
            .visible_rect()
            .ok_or_else(|| DecoderError::msg("a VideoFrame without a visible rectangle"))?;
        let (Some(x), Some(y), Some(width), Some(height)) = (
            get_u32(&rect, "x"),
            get_u32(&rect, "y"),
            get_u32(&rect, "width"),
            get_u32(&rect, "height"),
        ) else {
            return Err(DecoderError::msg("an unreadable VideoFrame visible rectangle"));
        };
        if !(0 < width && width <= MAX_PICTURE_SIDE && 0 < height && height <= MAX_PICTURE_SIDE) {
            return Err(DecoderError::msg(format!("a {width}x{height} picture")));
        }
        let (Ok(width16), Ok(height16)) = (u16::try_from(width), u16::try_from(height)) else {
            return Err(DecoderError::msg("picture size past 65535"));
        };
        let copy_rect = Object::new();
        set(&copy_rect, "x", &JsValue::from(x));
        set(&copy_rect, "y", &JsValue::from(y));
        set(&copy_rect, "width", &JsValue::from(width));
        set(&copy_rect, "height", &JsValue::from(height));
        let options = Object::new();
        set(&options, "rect", &copy_rect);
        let size = frame
            .allocation_size(&options)
            .map_err(|e| DecoderError::msg(format!("VideoFrame.allocationSize: {e:?}")))?;
        // Four bytes per pixel bounds every 8-bit 4:2:0 layout, alpha plane included.
        let largest = f64::from(width) * f64::from(height) * 4.0;
        if !(0.0 < size && size <= largest) {
            return Err(DecoderError::msg(format!("a {size}-byte VideoFrame copy")));
        }
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked to be positive and at most 4 * 8192 * 8192 just before"
        )]
        let bytes = size as u32;
        // A browser may convert a full-range stream to limited range, as Chrome's VideoToolbox decoder
        // does, and says so in the frame's color space; MS-RDPEGFX needs the samples sent.
        let space = frame.color_space();
        let field = |key: &str| {
            space
                .as_ref()
                .and_then(|space| Reflect::get(space, &JsValue::from_str(key)).ok())
        };
        let frame_full_range = field("fullRange").and_then(|full| full.as_bool());
        let frame_matrix = field("matrix").and_then(|matrix| matrix.as_string());
        let stream_full_range = self.stream.declares_full_range(number);
        let expand_range = stream::expands_range(stream_full_range, frame_full_range, frame_matrix.as_deref());
        if stream_full_range && frame_full_range == Some(false) && !self.warned_range {
            self.warned_range = true;
            if expand_range {
                info!("The browser decodes the full-range H.264 stream to limited range; expanding it back");
            } else {
                warn!(
                    matrix = frame_matrix,
                    "The browser decodes the full-range H.264 stream to limited range with another matrix; colors will be off"
                );
            }
        }
        let reservation = self
            .budget
            .reserve(usize::try_from(bytes).unwrap_or(usize::MAX))
            .ok_or_else(|| DecoderError::msg("decoded pictures past their session memory budget"))?;
        Ok(Copy {
            buffer: Uint8Array::new_with_length(bytes),
            options,
            format,
            width: width16,
            height: height16,
            expand_range,
            reservation,
        })
    }
}

/// A plane copy about to start, with the memory it holds until it settles.
struct Copy {
    buffer: Uint8Array,
    options: Object,
    format: String,
    width: u16,
    height: u16,
    expand_range: bool,
    reservation: Reservation,
}

impl Copy {
    /// Copies the planes of `frame` in the background and files them as picture `number` of
    /// `generation` of decoder `id`.
    // UNTESTED: the copy owns its reservation until its promise settles, whatever happens to the
    // picture meanwhile, and the frame is closed exactly once after that. No native test runs
    // WebCodecs: when changing this, watch the browser's memory across resizes during a scroll.
    fn start(self, id: u32, generation: u64, number: u64, frame: JsVideoFrame) {
        let Self {
            buffer,
            options,
            format,
            width,
            height,
            expand_range,
            reservation,
        } = self;
        let copy = frame.copy_to(&buffer, &options);
        spawn_local(async move {
            let layout = match copy {
                Ok(promise) => JsFuture::from(promise).await,
                Err(error) => Err(error),
            };
            frame.close_frame();
            with_registered(id, |registered| {
                if !registered.pictures.awaits(generation, number) {
                    return;
                }
                let picture = layout
                    .map_err(|error| DecoderError::msg(format!("VideoFrame.copyTo: {error:?}")))
                    .and_then(|layout| {
                        let layout = plane_layout(&layout)
                            .ok_or_else(|| DecoderError::msg("an unreadable VideoFrame layout"))?;
                        stream::yuv420_planes(&buffer.to_vec(), &format, &layout, width, height, expand_range)
                    })
                    .map(DecodedPicture::from);
                registered
                    .pictures
                    .finish(generation, number, picture, Some(reservation));
                (registered.notify)();
            });
        });
    }
}

/// The plane layout `copyTo` reported: offset and stride of each plane.
fn plane_layout(layout: &JsValue) -> Option<Vec<(usize, usize)>> {
    Array::from(layout)
        .iter()
        .map(|plane| {
            let offset = usize::try_from(get_u32(&plane, "offset")?).ok()?;
            let stride = usize::try_from(get_u32(&plane, "stride")?).ok()?;
            Some((offset, stride))
        })
        .collect()
}

fn on_output(id: u32, opened: u64, frame: JsVideoFrame) {
    let mut frame = Some(frame);
    with_registered(id, |registered| {
        if registered.sink.opened == opened
            && let Some(frame) = frame.take()
        {
            registered.output(frame);
        }
    });
    if let Some(frame) = frame {
        frame.close_frame();
    }
}

fn on_error(id: u32, opened: u64, error: &JsValue) {
    with_registered(id, |registered| {
        if registered.sink.opened != opened {
            return;
        }
        let failure = failure_of(error);
        warn!(
            ?error,
            ?failure,
            "The WebCodecs H.264 decoder failed; waiting for an IDR picture"
        );
        registered.stream.fail(&mut registered.sink, failure);
        registered.pictures.fail_all("the WebCodecs decoder failed");
        (registered.notify)();
    });
}

thread_local! {
    // UNTESTED: a `WebCodecsDecoder` is used on the thread that created it, which the wasm32 build
    // guarantees since it has a single thread; a decoder moved to another thread would find no
    // registry entry and fail every picture. Check this before building with threads.
    static DECODERS: RefCell<HashMap<u32, Registered>> = RefCell::new(HashMap::new());
    static BUDGETS: RefCell<HashMap<u32, Rc<Budget>>> = RefCell::new(HashMap::new());
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

fn with_registered<R>(id: u32, f: impl FnOnce(&mut Registered) -> R) -> Option<R> {
    DECODERS.with(|decoders| {
        let mut decoders = decoders.try_borrow_mut().ok()?;
        decoders.get_mut(&id).map(f)
    })
}

fn next_id() -> u32 {
    NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    })
}

/// The memory budget for the decoded pictures of one session, shared by all its decoders.
/// Dropping it unregisters the budget; decoders still open keep it until they are dropped.
pub(crate) struct SessionBudget {
    id: u32,
}

impl SessionBudget {
    pub(crate) fn new() -> Self {
        let id = next_id();
        BUDGETS.with(|budgets| budgets.borrow_mut().insert(id, Budget::new(SESSION_PICTURE_BYTES)));
        Self { id }
    }
}

impl Drop for SessionBudget {
    fn drop(&mut self) {
        let _ = BUDGETS.try_with(|budgets| budgets.borrow_mut().remove(&self.id));
    }
}

/// An H.264 decoder running on the browser's WebCodecs `VideoDecoder`.
pub(crate) struct WebCodecsDecoder {
    id: u32,
}

impl WebCodecsDecoder {
    /// Decodes with the stream's own configuration, falling back on `probed_codec`, a string
    /// [`supported_codec`] returned, charges decoded pictures to `budget`, and calls `notify`
    /// whenever a picture is ready for [`H264Decoder::poll_decoded`].
    pub(crate) fn new(probed_codec: &str, budget: &SessionBudget, notify: Rc<dyn Fn()>) -> Self {
        let id = next_id();
        let budget = BUDGETS
            .with(|budgets| budgets.borrow().get(&budget.id).map(Rc::clone))
            .unwrap_or_else(|| Budget::new(SESSION_PICTURE_BYTES));
        DECODERS.with(|decoders| {
            decoders.borrow_mut().insert(
                id,
                Registered {
                    sink: JsSink {
                        id,
                        decoder: None,
                        opened: 0,
                    },
                    stream: Stream::new(probed_codec),
                    pictures: Pictures::default(),
                    budget,
                    notify,
                    warned_format: false,
                    warned_range: false,
                },
            )
        });
        Self { id }
    }
}

impl Drop for WebCodecsDecoder {
    fn drop(&mut self) {
        DECODERS.with(|decoders| decoders.borrow_mut().remove(&self.id));
    }
}

impl H264Decoder for WebCodecsDecoder {
    fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
        Err(DecoderError::msg("WebCodecs decodes asynchronously"))
    }

    fn submit(&mut self, data: &[u8]) -> DecoderResult<DecodeStatus> {
        with_registered(self.id, |registered| registered.submit(data))
            .unwrap_or_else(|| Err(DecoderError::msg("the decoder is gone")))
    }

    fn poll_decoded(&mut self) -> Option<DecoderResult<DecodedPicture>> {
        with_registered(self.id, Registered::poll).flatten()
    }

    fn flush(&mut self) {
        with_registered(self.id, Registered::flush);
    }

    fn reset(&mut self) {
        debug!("Resetting the WebCodecs H.264 decoder");
        with_registered(self.id, Registered::reset);
    }
}

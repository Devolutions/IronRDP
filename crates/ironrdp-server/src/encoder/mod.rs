use core::fmt;
use core::num::NonZeroU16;
use core::time::Duration;
use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use ironrdp_acceptor::DesktopSize;
use ironrdp_graphics::diff::{find_different_rects_sub, Rect};
use ironrdp_pdu::encode_vec;
use ironrdp_pdu::fast_path::UpdateCode;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute, PointerPositionAttribute};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};

use self::bitmap::BitmapEncoder;
use self::rfx::RfxEncoder;
use super::BitmapUpdate;
use crate::{time_warn, ColorPointer, DisplayUpdate, Framebuffer, RGBAPointer};

mod bitmap;
mod fast_path;
pub(crate) mod rfx;

pub(crate) use fast_path::*;

const VIDEO_HINT_FPS: usize = 5;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
enum CodecId {
    None = 0x0,
}

#[cfg_attr(feature = "__bench", visibility::make(pub))]
#[derive(Debug)]
pub(crate) struct UpdateEncoderCodecs {
    remotefx: Option<(EntropyBits, u8)>,
}

impl UpdateEncoderCodecs {
    #[cfg_attr(feature = "__bench", visibility::make(pub))]
    pub(crate) fn new() -> Self {
        Self { remotefx: None }
    }

    #[cfg_attr(feature = "__bench", visibility::make(pub))]
    pub(crate) fn set_remotefx(&mut self, remotefx: Option<(EntropyBits, u8)>) {
        self.remotefx = remotefx
    }
}

impl Default for UpdateEncoderCodecs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(feature = "__bench", visibility::make(pub))]
pub(crate) struct UpdateEncoder {
    desktop_size: DesktopSize,
    framebuffer: Option<Framebuffer>,
    bitmap_updater: BitmapUpdater,
    video_updater: Option<BitmapUpdater>,
    region_update_times: HashMap<Rect, Vec<Instant>>,
}

impl fmt::Debug for UpdateEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateEncoder")
            .field("bitmap_update", &self.bitmap_updater)
            .field("video_updater", &self.video_updater)
            .finish()
    }
}

impl UpdateEncoder {
    #[cfg_attr(feature = "__bench", visibility::make(pub))]
    pub(crate) fn new(desktop_size: DesktopSize, surface_flags: CmdFlags, codecs: UpdateEncoderCodecs) -> Self {
        let (bitmap_updater, video_updater) = if surface_flags.contains(CmdFlags::SET_SURFACE_BITS) {
            let mut bitmap = BitmapUpdater::None(NoneHandler);
            let mut video = None;

            if let Some((algo, id)) = codecs.remotefx {
                bitmap = BitmapUpdater::RemoteFx(RemoteFxHandler::new(algo, id, desktop_size));
                video = Some(bitmap.clone());
            }

            (bitmap, video)
        } else {
            (BitmapUpdater::Bitmap(BitmapHandler::new()), None)
        };

        Self {
            desktop_size,
            framebuffer: None,
            bitmap_updater,
            video_updater,
            region_update_times: HashMap::new(),
        }
    }

    #[cfg_attr(feature = "__bench", visibility::make(pub))]
    pub(crate) fn update(&mut self, update: DisplayUpdate) -> EncoderIter<'_> {
        EncoderIter {
            encoder: self,
            state: State::Start(update),
        }
    }

    pub(crate) fn set_desktop_size(&mut self, size: DesktopSize) {
        self.desktop_size = size;
        self.bitmap_updater.set_desktop_size(size);
    }

    #[allow(clippy::unused_self)]
    fn rgba_pointer(&self, ptr: RGBAPointer) -> Result<UpdateFragmenter> {
        let xor_mask = ptr.data;

        let hot_spot = Point16 {
            x: ptr.hot_x,
            y: ptr.hot_y,
        };
        let color_pointer = ColorPointerAttribute {
            cache_index: 0,
            hot_spot,
            width: ptr.width,
            height: ptr.height,
            xor_mask: &xor_mask,
            and_mask: &[],
        };
        let ptr = PointerAttribute {
            xor_bpp: 32,
            color_pointer,
        };
        Ok(UpdateFragmenter::new(UpdateCode::NewPointer, encode_vec(&ptr)?))
    }

    #[allow(clippy::unused_self)]
    fn color_pointer(&self, ptr: ColorPointer) -> Result<UpdateFragmenter> {
        let hot_spot = Point16 {
            x: ptr.hot_x,
            y: ptr.hot_y,
        };
        let ptr = ColorPointerAttribute {
            cache_index: 0,
            hot_spot,
            width: ptr.width,
            height: ptr.height,
            xor_mask: &ptr.xor_mask,
            and_mask: &ptr.and_mask,
        };
        Ok(UpdateFragmenter::new(UpdateCode::ColorPointer, encode_vec(&ptr)?))
    }

    #[allow(clippy::unused_self)]
    fn default_pointer(&self) -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::DefaultPointer, vec![]))
    }

    #[allow(clippy::unused_self)]
    fn hide_pointer(&self) -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::HiddenPointer, vec![]))
    }

    #[allow(clippy::unused_self)]
    fn pointer_position(&self, pos: PointerPositionAttribute) -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::PositionPointer, encode_vec(&pos)?))
    }

    // This is a very naive heuristic for detecting video regions
    // based on the number of updates in the last second.
    // Feel free to improve it! :)
    fn diff_hints(&mut self, now: Instant, off_x: usize, off_y: usize, regions: Vec<Rect>) -> Vec<HintRect> {
        // keep the updates from the last second
        for (_region, ts) in self.region_update_times.iter_mut() {
            ts.retain(|ts| now - *ts < Duration::from_millis(1000));
        }
        self.region_update_times.retain(|_, times| !times.is_empty());

        let mut diffs = Vec::new();
        for rect in regions {
            let rect_root = rect.clone().add_xy(off_x, off_y);
            let entry = self.region_update_times.entry(rect_root).or_default();
            entry.push(now);

            let hint = if entry.len() >= VIDEO_HINT_FPS {
                HintType::Video
            } else {
                HintType::Image
            };

            let diff = HintRect::new(rect, hint);
            diffs.push(diff);
        }

        diffs
    }

    fn bitmap_diffs(&mut self, bitmap: &BitmapUpdate) -> Vec<Rect> {
        const USE_DIFFS: bool = true;

        if let Some(Framebuffer {
            data: Some(data),
            stride,
            width,
            height,
            ..
        }) = USE_DIFFS.then_some(self.framebuffer.as_ref()).flatten()
        {
            find_different_rects_sub::<4>(
                data,
                *stride,
                width.get().into(),
                height.get().into(),
                &bitmap.data,
                bitmap.stride,
                bitmap.width.get().into(),
                bitmap.height.get().into(),
                bitmap.x.into(),
                bitmap.y.into(),
            )
        } else {
            vec![Rect {
                x: 0,
                y: 0,
                width: bitmap.width.get().into(),
                height: bitmap.height.get().into(),
            }]
        }
    }

    fn bitmap_update_framebuffer(&mut self, bitmap: BitmapUpdate, diffs: &[Rect]) {
        if bitmap.x == 0
            && bitmap.y == 0
            && bitmap.width.get() == self.desktop_size.width
            && bitmap.height.get() == self.desktop_size.height
        {
            match bitmap.try_into() {
                Ok(framebuffer) => self.framebuffer = Some(framebuffer),
                Err(err) => warn!("Failed to convert bitmap to framebuffer: {}", err),
            }
        } else if let Some(fb) = self.framebuffer.as_mut() {
            fb.update_diffs(&bitmap, diffs);
        }
    }

    async fn bitmap(&mut self, bitmap: BitmapUpdate, hint: HintType) -> Result<UpdateFragmenter> {
        let updater = match hint {
            HintType::Image => &self.bitmap_updater,
            HintType::Video => {
                trace!(?bitmap, "Encoding with video hint");
                self.video_updater.as_ref().unwrap_or(&self.bitmap_updater)
            }
        };
        // Clone to satisfy spawn_blocking 'static requirement
        // this should be cheap, even if using bitmap, since vec![] will be empty
        let mut updater = updater.clone();
        tokio::task::spawn_blocking(move || time_warn!("Encoding bitmap", 10, updater.handle(&bitmap)))
            .await
            .unwrap()
    }
}

#[derive(Copy, Clone, Debug)]
enum HintType {
    Image,
    Video,
}

#[derive(Debug)]
struct HintRect {
    rect: Rect,
    hint: HintType,
}

impl HintRect {
    fn new(rect: Rect, hint: HintType) -> Self {
        Self { rect, hint }
    }
}

#[derive(Debug, Default)]
enum State {
    Start(DisplayUpdate),
    BitmapDiffs {
        diffs: Vec<HintRect>,
        bitmap: BitmapUpdate,
        pos: usize,
    },
    #[default]
    Ended,
}

#[cfg_attr(feature = "__bench", visibility::make(pub))]
pub(crate) struct EncoderIter<'a> {
    encoder: &'a mut UpdateEncoder,
    state: State,
}

impl EncoderIter<'_> {
    #[cfg_attr(feature = "__bench", visibility::make(pub))]
    pub(crate) async fn next(&mut self) -> Option<Result<UpdateFragmenter>> {
        loop {
            let state = core::mem::take(&mut self.state);
            let encoder = &mut self.encoder;

            let res = match state {
                State::Start(update) => match update {
                    DisplayUpdate::Bitmap(bitmap) => {
                        let diffs = encoder.bitmap_diffs(&bitmap);
                        let diffs =
                            encoder.diff_hints(Instant::now(), usize::from(bitmap.x), usize::from(bitmap.y), diffs);
                        self.state = State::BitmapDiffs { diffs, bitmap, pos: 0 };
                        continue;
                    }
                    DisplayUpdate::PointerPosition(pos) => encoder.pointer_position(pos),
                    DisplayUpdate::RGBAPointer(ptr) => encoder.rgba_pointer(ptr),
                    DisplayUpdate::ColorPointer(ptr) => encoder.color_pointer(ptr),
                    DisplayUpdate::HidePointer => encoder.hide_pointer(),
                    DisplayUpdate::DefaultPointer => encoder.default_pointer(),
                    DisplayUpdate::Resize(_) => return None,
                },
                State::BitmapDiffs { diffs, bitmap, pos } => {
                    let Some(diff) = diffs.get(pos) else {
                        let diffs = diffs.into_iter().map(|diff| diff.rect).collect::<Vec<_>>();
                        encoder.bitmap_update_framebuffer(bitmap, &diffs);
                        self.state = State::Ended;
                        return None;
                    };

                    let Rect { x, y, width, height } = diff.rect;
                    let Some(sub) = bitmap.sub(
                        u16::try_from(x).unwrap(),
                        u16::try_from(y).unwrap(),
                        NonZeroU16::new(u16::try_from(width).unwrap()).unwrap(),
                        NonZeroU16::new(u16::try_from(height).unwrap()).unwrap(),
                    ) else {
                        warn!("Failed to extract bitmap subregion");
                        return None;
                    };
                    let hint = diff.hint;
                    self.state = State::BitmapDiffs {
                        diffs,
                        bitmap,
                        pos: pos + 1,
                    };
                    encoder.bitmap(sub, hint).await
                }
                State::Ended => return None,
            };

            return Some(res);
        }
    }
}

#[derive(Debug, Clone)]
enum BitmapUpdater {
    None(NoneHandler),
    Bitmap(BitmapHandler),
    RemoteFx(RemoteFxHandler),
}

impl BitmapUpdater {
    fn handle(&mut self, bitmap: &BitmapUpdate) -> Result<UpdateFragmenter> {
        match self {
            Self::None(up) => up.handle(bitmap),
            Self::Bitmap(up) => up.handle(bitmap),
            Self::RemoteFx(up) => up.handle(bitmap),
        }
    }

    fn set_desktop_size(&mut self, size: DesktopSize) {
        if let Self::RemoteFx(up) = self {
            up.set_desktop_size(size)
        }
    }
}

trait BitmapUpdateHandler {
    fn handle(&mut self, bitmap: &BitmapUpdate) -> Result<UpdateFragmenter>;
}

#[derive(Clone, Debug)]
struct NoneHandler;

impl BitmapUpdateHandler for NoneHandler {
    fn handle(&mut self, bitmap: &BitmapUpdate) -> Result<UpdateFragmenter> {
        let stride = usize::from(bitmap.format.bytes_per_pixel()) * usize::from(bitmap.width.get());
        let mut data = Vec::with_capacity(stride * usize::from(bitmap.height.get()));
        for row in bitmap.data.chunks(bitmap.stride).rev() {
            data.extend_from_slice(&row[..stride]);
        }
        set_surface(bitmap, CodecId::None as u8, &data)
    }
}

#[derive(Clone)]
struct BitmapHandler {
    bitmap: BitmapEncoder,
}

impl fmt::Debug for BitmapHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BitmapHandler").finish()
    }
}

impl BitmapHandler {
    fn new() -> Self {
        Self {
            bitmap: BitmapEncoder::new(),
        }
    }
}

impl BitmapUpdateHandler for BitmapHandler {
    fn handle(&mut self, bitmap: &BitmapUpdate) -> Result<UpdateFragmenter> {
        let mut buffer = vec![0; bitmap.data.len() * 2]; // TODO: estimate bitmap encoded size
        let len = loop {
            match self.bitmap.encode(bitmap, buffer.as_mut_slice()) {
                Err(e) => match e.kind() {
                    ironrdp_core::EncodeErrorKind::NotEnoughBytes { .. } => {
                        buffer.resize(buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", buffer.len() * 2);
                    }

                    _ => Err(e).context("bitmap encode error")?,
                },
                Ok(len) => break len,
            }
        };

        buffer.truncate(len);
        Ok(UpdateFragmenter::new(UpdateCode::Bitmap, buffer))
    }
}

#[derive(Debug, Clone)]
struct RemoteFxHandler {
    remotefx: RfxEncoder,
    codec_id: u8,
    desktop_size: Option<DesktopSize>,
}

impl RemoteFxHandler {
    fn new(algo: EntropyBits, codec_id: u8, desktop_size: DesktopSize) -> Self {
        Self {
            remotefx: RfxEncoder::new(algo),
            desktop_size: Some(desktop_size),
            codec_id,
        }
    }

    fn set_desktop_size(&mut self, size: DesktopSize) {
        self.desktop_size = Some(size);
    }
}

impl BitmapUpdateHandler for RemoteFxHandler {
    fn handle(&mut self, bitmap: &BitmapUpdate) -> Result<UpdateFragmenter> {
        let mut buffer = vec![0; bitmap.data.len()];
        let len = loop {
            match self
                .remotefx
                .encode(bitmap, buffer.as_mut_slice(), self.desktop_size.take())
            {
                Err(e) => match e.kind() {
                    ironrdp_core::EncodeErrorKind::NotEnoughBytes { .. } => {
                        buffer.resize(buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", buffer.len() * 2);
                    }
                    _ => Err(e).context("RemoteFX encode error")?,
                },
                Ok(len) => break len,
            }
        };

        set_surface(bitmap, self.codec_id, &buffer[..len])
    }
}

fn set_surface(bitmap: &BitmapUpdate, codec_id: u8, data: &[u8]) -> Result<UpdateFragmenter> {
    let destination = ExclusiveRectangle {
        left: bitmap.x,
        top: bitmap.y,
        right: bitmap.x + bitmap.width.get(),
        bottom: bitmap.y + bitmap.height.get(),
    };
    let extended_bitmap_data = ExtendedBitmapDataPdu {
        bpp: bitmap.format.bytes_per_pixel() * 8,
        width: bitmap.width.get(),
        height: bitmap.height.get(),
        codec_id,
        header: None,
        data,
    };
    let pdu = SurfaceBitsPdu {
        destination,
        extended_bitmap_data,
    };
    let cmd = SurfaceCommand::SetSurfaceBits(pdu);
    Ok(UpdateFragmenter::new(UpdateCode::SurfaceCommands, encode_vec(&cmd)?))
}

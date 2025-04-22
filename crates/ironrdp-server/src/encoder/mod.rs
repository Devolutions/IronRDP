use core::fmt;

use anyhow::{Context, Result};
use ironrdp_acceptor::DesktopSize;
use ironrdp_pdu::encode_vec;
use ironrdp_pdu::fast_path::UpdateCode;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute, PointerPositionAttribute};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};

use self::bitmap::BitmapEncoder;
use self::rfx::RfxEncoder;
use super::BitmapUpdate;
use crate::macros::time_warn;
use crate::{ColorPointer, DisplayUpdate, Framebuffer, RGBAPointer};

mod bitmap;
mod fast_path;
pub(crate) mod rfx;

pub(crate) use fast_path::*;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
enum CodecId {
    None = 0x0,
}

pub(crate) struct UpdateEncoder {
    desktop_size: DesktopSize,
    // FIXME: draw updates on the framebuffer
    framebuffer: Option<Framebuffer>,
    bitmap_updater: BitmapUpdater,
}

impl fmt::Debug for UpdateEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateEncoder")
            .field("bitmap_update", &self.bitmap_updater)
            .finish()
    }
}

impl UpdateEncoder {
    pub(crate) fn new(desktop_size: DesktopSize, surface_flags: CmdFlags, remotefx: Option<(EntropyBits, u8)>) -> Self {
        let bitmap_updater = if !surface_flags.contains(CmdFlags::SET_SURFACE_BITS) {
            BitmapUpdater::Bitmap(BitmapHandler::new())
        } else if remotefx.is_some() {
            let (algo, id) = remotefx.unwrap();
            BitmapUpdater::RemoteFx(RemoteFxHandler::new(algo, id, desktop_size))
        } else {
            BitmapUpdater::None(NoneHandler)
        };

        Self {
            desktop_size,
            framebuffer: None,
            bitmap_updater,
        }
    }

    pub(crate) fn update(&mut self, update: DisplayUpdate) -> EncoderIter<'_> {
        EncoderIter {
            encoder: self,
            update: Some(update),
        }
    }

    pub(crate) fn set_desktop_size(&mut self, size: DesktopSize) {
        self.desktop_size = size;
        self.bitmap_updater.set_desktop_size(size);
    }

    fn rgba_pointer(ptr: RGBAPointer) -> Result<UpdateFragmenter> {
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

    fn color_pointer(ptr: ColorPointer) -> Result<UpdateFragmenter> {
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

    fn default_pointer() -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::DefaultPointer, vec![]))
    }

    fn hide_pointer() -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::HiddenPointer, vec![]))
    }

    fn pointer_position(pos: PointerPositionAttribute) -> Result<UpdateFragmenter> {
        Ok(UpdateFragmenter::new(UpdateCode::PositionPointer, encode_vec(&pos)?))
    }

    async fn bitmap(&mut self, bitmap: BitmapUpdate) -> Result<UpdateFragmenter> {
        // Clone to satisfy spawn_blocking 'static requirement
        // this should be cheap, even if using bitmap, since vec![] will be empty
        let mut updater = self.bitmap_updater.clone();
        let (res, bitmap) =
            tokio::task::spawn_blocking(move || time_warn!("Encoding bitmap", 10, (updater.handle(&bitmap), bitmap)))
                .await
                .unwrap();
        if bitmap.x == 0
            && bitmap.y == 0
            && bitmap.width.get() == self.desktop_size.width
            && bitmap.height.get() == self.desktop_size.height
        {
            match bitmap.try_into() {
                Ok(framebuffer) => self.framebuffer = Some(framebuffer),
                Err(err) => warn!("Failed to convert bitmap to framebuffer: {}", err),
            }
        }
        res
    }
}

pub(crate) struct EncoderIter<'a> {
    encoder: &'a mut UpdateEncoder,
    update: Option<DisplayUpdate>,
}

impl EncoderIter<'_> {
    pub(crate) async fn next(&mut self) -> Option<Result<UpdateFragmenter>> {
        let update = self.update.take()?;
        let encoder = &mut self.encoder;

        let res = match update {
            DisplayUpdate::Bitmap(bitmap) => encoder.bitmap(bitmap).await,
            DisplayUpdate::PointerPosition(pos) => UpdateEncoder::pointer_position(pos),
            DisplayUpdate::RGBAPointer(ptr) => UpdateEncoder::rgba_pointer(ptr),
            DisplayUpdate::ColorPointer(ptr) => UpdateEncoder::color_pointer(ptr),
            DisplayUpdate::HidePointer => UpdateEncoder::hide_pointer(),
            DisplayUpdate::DefaultPointer => UpdateEncoder::default_pointer(),
            DisplayUpdate::Resize(_) => return None,
        };

        Some(res)
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

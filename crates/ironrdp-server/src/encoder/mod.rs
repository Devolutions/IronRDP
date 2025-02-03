mod bitmap;
pub(crate) mod rfx;

use core::cmp;

use anyhow::{Context, Result};
use ironrdp_core::{Encode, WriteCursor};
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute, PointerPositionAttribute};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};

use self::bitmap::BitmapEncoder;
use self::rfx::RfxEncoder;
use super::BitmapUpdate;
use crate::{ColorPointer, RGBAPointer};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
enum CodecId {
    None = 0x0,
}

// this is the maximum amount of data (not including headers) we can send in a single TS_FP_UPDATE_PDU
const MAX_FASTPATH_UPDATE_SIZE: usize = 16_374;

const FASTPATH_HEADER_SIZE: usize = 6;

pub(crate) struct UpdateEncoder {
    pdu_encoder: PduEncoder,
    bitmap_updater: BitmapUpdater,
}

impl UpdateEncoder {
    pub(crate) fn new(surface_flags: CmdFlags, remotefx: Option<(EntropyBits, u8)>) -> Self {
        let pdu_encoder = PduEncoder::new();
        let bitmap_updater = if !surface_flags.contains(CmdFlags::SET_SURFACE_BITS) {
            BitmapUpdater::Bitmap(BitmapHandler::new())
        } else if remotefx.is_some() {
            let (algo, id) = remotefx.unwrap();
            BitmapUpdater::RemoteFx(RemoteFxHandler::new(algo, id))
        } else {
            BitmapUpdater::None(NoneHandler)
        };

        Self {
            pdu_encoder,
            bitmap_updater,
        }
    }

    pub(crate) fn rgba_pointer(&mut self, ptr: RGBAPointer) -> Result<UpdateFragmenter<'_>> {
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
        let buf = self.pdu_encoder.encode(ptr)?;
        Ok(UpdateFragmenter::new(UpdateCode::NewPointer, buf))
    }

    pub(crate) fn color_pointer(&mut self, ptr: ColorPointer) -> Result<UpdateFragmenter<'_>> {
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
        let buf = self.pdu_encoder.encode(ptr)?;
        Ok(UpdateFragmenter::new(UpdateCode::ColorPointer, buf))
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn default_pointer(&mut self) -> Result<UpdateFragmenter<'_>> {
        Ok(UpdateFragmenter::new(UpdateCode::DefaultPointer, &[]))
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn hide_pointer(&mut self) -> Result<UpdateFragmenter<'_>> {
        Ok(UpdateFragmenter::new(UpdateCode::HiddenPointer, &[]))
    }

    pub(crate) fn pointer_position(&mut self, pos: PointerPositionAttribute) -> Result<UpdateFragmenter<'_>> {
        let buf = self.pdu_encoder.encode(pos)?;
        Ok(UpdateFragmenter::new(UpdateCode::PositionPointer, buf))
    }

    pub(crate) fn bitmap(&mut self, bitmap: BitmapUpdate) -> Result<UpdateFragmenter<'_>> {
        self.bitmap_updater.handle(bitmap, &mut self.pdu_encoder)
    }

    pub(crate) fn fragmenter_from_owned(&self, res: UpdateFragmenterOwned) -> UpdateFragmenter<'_> {
        UpdateFragmenter {
            code: res.code,
            index: res.index,
            data: &self.pdu_encoder.buffer[0..res.len],
        }
    }
}

enum BitmapUpdater {
    None(NoneHandler),
    Bitmap(BitmapHandler),
    RemoteFx(RemoteFxHandler),
}

impl BitmapUpdater {
    fn handle<'a>(&mut self, bitmap: BitmapUpdate, encoder: &'a mut PduEncoder) -> Result<UpdateFragmenter<'a>> {
        match self {
            Self::None(up) => up.handle(bitmap, encoder),
            Self::Bitmap(up) => up.handle(bitmap, encoder),
            Self::RemoteFx(up) => up.handle(bitmap, encoder),
        }
    }
}

trait BitmapUpdateHandler {
    fn handle<'a>(&mut self, bitmap: BitmapUpdate, encoder: &'a mut PduEncoder) -> Result<UpdateFragmenter<'a>>;
}

struct NoneHandler;

impl BitmapUpdateHandler for NoneHandler {
    fn handle<'a>(&mut self, bitmap: BitmapUpdate, encoder: &'a mut PduEncoder) -> Result<UpdateFragmenter<'a>> {
        let stride = usize::from(bitmap.format.bytes_per_pixel()) * usize::from(bitmap.width.get());
        let mut data = Vec::with_capacity(stride * usize::from(bitmap.height.get()));
        for row in bitmap.data.chunks(bitmap.stride).rev() {
            data.extend_from_slice(&row[..stride]);
        }

        encoder.set_surface(bitmap, CodecId::None as u8, &data)
    }
}

struct BitmapHandler {
    bitmap: BitmapEncoder,
}

impl BitmapHandler {
    fn new() -> Self {
        Self {
            bitmap: BitmapEncoder::new(),
        }
    }
}

impl BitmapUpdateHandler for BitmapHandler {
    fn handle<'a>(&mut self, bitmap: BitmapUpdate, encoder: &'a mut PduEncoder) -> Result<UpdateFragmenter<'a>> {
        let len = loop {
            match self.bitmap.encode(&bitmap, encoder.buffer.as_mut_slice()) {
                Err(e) => match e.kind() {
                    ironrdp_core::EncodeErrorKind::NotEnoughBytes { .. } => {
                        encoder.buffer.resize(encoder.buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", encoder.buffer.len() * 2);
                    }

                    _ => Err(e).context("bitmap encode error")?,
                },
                Ok(len) => break len,
            }
        };

        Ok(UpdateFragmenter::new(UpdateCode::Bitmap, &encoder.buffer[..len]))
    }
}

struct RemoteFxHandler {
    remotefx: RfxEncoder,
    codec_id: u8,
}

impl RemoteFxHandler {
    fn new(algo: EntropyBits, codec_id: u8) -> Self {
        Self {
            remotefx: RfxEncoder::new(algo),
            codec_id,
        }
    }
}

impl BitmapUpdateHandler for RemoteFxHandler {
    fn handle<'a>(&mut self, bitmap: BitmapUpdate, encoder: &'a mut PduEncoder) -> Result<UpdateFragmenter<'a>> {
        let mut buffer = vec![0; bitmap.data.len()];
        let len = loop {
            match self.remotefx.encode(&bitmap, buffer.as_mut_slice()) {
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

        encoder.set_surface(bitmap, self.codec_id, &buffer[..len])
    }
}

struct PduEncoder {
    buffer: Vec<u8>,
}

impl PduEncoder {
    fn new() -> Self {
        Self { buffer: vec![0; 16384] }
    }

    fn encode(&mut self, pdu: impl Encode) -> Result<&[u8]> {
        let pos = loop {
            let mut cursor = WriteCursor::new(self.buffer.as_mut_slice());
            match pdu.encode(&mut cursor) {
                Err(e) => match e.kind() {
                    ironrdp_core::EncodeErrorKind::NotEnoughBytes { .. } => {
                        self.buffer.resize(self.buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", self.buffer.len() * 2);
                    }

                    _ => Err(e).context("PDU encode error")?,
                },
                Ok(()) => break cursor.pos(),
            }
        };

        Ok(&self.buffer[..pos])
    }

    fn set_surface(&mut self, bitmap: BitmapUpdate, codec_id: u8, data: &[u8]) -> Result<UpdateFragmenter<'_>> {
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
        let buf = self.encode(cmd)?;
        Ok(UpdateFragmenter::new(UpdateCode::SurfaceCommands, buf))
    }
}

pub(crate) struct UpdateFragmenterOwned {
    code: UpdateCode,
    index: usize,
    len: usize,
}

pub(crate) struct UpdateFragmenter<'a> {
    code: UpdateCode,
    index: usize,
    data: &'a [u8],
}

impl<'a> UpdateFragmenter<'a> {
    pub(crate) fn new(code: UpdateCode, data: &'a [u8]) -> Self {
        Self { code, index: 0, data }
    }

    pub(crate) fn into_owned(self) -> UpdateFragmenterOwned {
        UpdateFragmenterOwned {
            code: self.code,
            index: self.index,
            len: self.data.len(),
        }
    }

    pub(crate) fn size_hint(&self) -> usize {
        FASTPATH_HEADER_SIZE + cmp::min(self.data.len(), MAX_FASTPATH_UPDATE_SIZE)
    }

    pub(crate) fn next(&mut self, dst: &mut [u8]) -> Option<usize> {
        let (consumed, written) = self.encode_next(dst)?;
        self.data = &self.data[consumed..];
        self.index = self.index.checked_add(1)?;
        Some(written)
    }

    fn encode_next(&mut self, dst: &mut [u8]) -> Option<(usize, usize)> {
        match self.data.len() {
            0 => None,

            1..=MAX_FASTPATH_UPDATE_SIZE => {
                let frag = if self.index > 0 {
                    Fragmentation::Last
                } else {
                    Fragmentation::Single
                };

                self.encode_fastpath(frag, self.data, dst)
                    .map(|written| (self.data.len(), written))
            }

            _ => {
                let frag = if self.index > 0 {
                    Fragmentation::Next
                } else {
                    Fragmentation::First
                };

                self.encode_fastpath(frag, &self.data[..MAX_FASTPATH_UPDATE_SIZE], dst)
                    .map(|written| (MAX_FASTPATH_UPDATE_SIZE, written))
            }
        }
    }

    fn encode_fastpath(&self, frag: Fragmentation, data: &[u8], dst: &mut [u8]) -> Option<usize> {
        let mut cursor = WriteCursor::new(dst);

        let update = FastPathUpdatePdu {
            fragmentation: frag,
            update_code: self.code,
            compression_flags: None,
            compression_type: None,
            data,
        };

        let header = FastPathHeader::new(EncryptionFlags::empty(), update.size());

        header.encode(&mut cursor).ok()?;
        update.encode(&mut cursor).ok()?;

        Some(cursor.pos())
    }
}

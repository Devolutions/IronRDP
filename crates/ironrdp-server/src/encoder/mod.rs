pub(crate) mod bitmap;
pub(crate) mod rfx;

use anyhow::{Context, Result};
use std::{cmp, mem};

use ironrdp_pdu::cursor::WriteCursor;
use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits};
use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};
use ironrdp_pdu::PduEncode;

use crate::PixelOrder;

use self::bitmap::BitmapEncoder;
use self::rfx::RfxEncoder;
use super::BitmapUpdate;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
enum CodecId {
    None = 0x0,
}

// this is the maximum amount of data (not including headers) we can send in a single TS_FP_UPDATE_PDU
const MAX_FASTPATH_UPDATE_SIZE: usize = 16_374;

const FASTPATH_HEADER_SIZE: usize = 6;

pub(crate) struct UpdateEncoder {
    buffer: Vec<u8>,
    bitmap: BitmapEncoder,
    remotefx: Option<(RfxEncoder, u8)>,
    update: for<'a> fn(&'a mut UpdateEncoder, BitmapUpdate) -> Result<UpdateFragmenter<'a>>,
}

impl UpdateEncoder {
    pub(crate) fn new(surface_flags: CmdFlags, remotefx: Option<(EntropyBits, u8)>) -> Self {
        let update = if !surface_flags.contains(CmdFlags::SET_SURFACE_BITS) {
            Self::bitmap_update
        } else if remotefx.is_some() {
            Self::remotefx_update
        } else {
            Self::none_update
        };

        Self {
            buffer: vec![0; 16384],
            bitmap: BitmapEncoder::new(),
            remotefx: remotefx.map(|(algo, id)| (RfxEncoder::new(algo), id)),
            update,
        }
    }

    fn encode_pdu(&mut self, pdu: impl PduEncode) -> Result<usize> {
        loop {
            let mut cursor = WriteCursor::new(self.buffer.as_mut_slice());
            match pdu.encode(&mut cursor) {
                Err(e) => match e.kind() {
                    ironrdp_pdu::PduErrorKind::NotEnoughBytes { .. } => {
                        self.buffer.resize(self.buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", self.buffer.len() * 2);
                    }

                    _ => Err(e).context("PDU encode error")?,
                },
                Ok(()) => return Ok(cursor.pos()),
            }
        }
    }

    pub(crate) fn bitmap(&mut self, bitmap: BitmapUpdate) -> Result<UpdateFragmenter<'_>> {
        let update = self.update;

        update(self, bitmap)
    }

    fn bitmap_update(&mut self, bitmap: BitmapUpdate) -> Result<UpdateFragmenter<'_>> {
        let len = loop {
            match self.bitmap.encode(&bitmap, self.buffer.as_mut_slice()) {
                Err(e) => match e.kind() {
                    ironrdp_pdu::PduErrorKind::NotEnoughBytes { .. } => {
                        self.buffer.resize(self.buffer.len() * 2, 0);
                        debug!("encoder buffer resized to: {}", self.buffer.len() * 2);
                    }

                    _ => Err(e).context("bitmap encode error")?,
                },
                Ok(len) => break len,
            }
        };

        Ok(UpdateFragmenter::new(UpdateCode::Bitmap, &self.buffer[..len]))
    }

    fn set_surface(&mut self, bitmap: BitmapUpdate, codec_id: u8, data: Vec<u8>) -> Result<UpdateFragmenter<'_>> {
        let destination = ExclusiveRectangle {
            left: bitmap.left,
            top: bitmap.top,
            right: bitmap.left + bitmap.width.get(),
            bottom: bitmap.top + bitmap.height.get(),
        };
        let extended_bitmap_data = ExtendedBitmapDataPdu {
            bpp: bitmap.format.bytes_per_pixel() * 8,
            width: bitmap.width.get(),
            height: bitmap.height.get(),
            codec_id,
            header: None,
            data: &data,
        };
        let pdu = SurfaceBitsPdu {
            destination,
            extended_bitmap_data,
        };
        let cmd = SurfaceCommand::SetSurfaceBits(pdu);
        let len = self.encode_pdu(cmd)?;
        Ok(UpdateFragmenter::new(UpdateCode::SurfaceCommands, &self.buffer[..len]))
    }

    fn remotefx_update(&mut self, bitmap: BitmapUpdate) -> Result<UpdateFragmenter<'_>> {
        let (remotefx, codec_id) = self.remotefx.as_mut().unwrap();
        let codec_id = *codec_id;
        let data = remotefx.encode(&bitmap).context("RemoteFX encoding")?;

        self.set_surface(bitmap, codec_id, data)
    }

    fn none_update(&mut self, mut bitmap: BitmapUpdate) -> Result<UpdateFragmenter<'_>> {
        let data = match bitmap.order {
            PixelOrder::BottomToTop => mem::take(&mut bitmap.data),
            PixelOrder::TopToBottom => {
                let row_len = usize::from(bitmap.width.get()) * usize::from(bitmap.format.bytes_per_pixel());
                let mut data = Vec::with_capacity(bitmap.data.len());
                for row in bitmap.data.chunks(row_len).rev() {
                    data.extend_from_slice(row);
                }
                data
            }
        };

        self.set_surface(bitmap, CodecId::None as u8, data)
    }
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

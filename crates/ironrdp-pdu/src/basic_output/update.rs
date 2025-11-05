use ironrdp_core::{
    ensure_size, invalid_field_err, read_padding, Decode, DecodeResult, Encode, EncodeResult, IntoOwned, ReadCursor,
    WriteCursor,
};

use crate::basic_output::bitmap::{BitmapData, BitmapUpdateData, CompressedDataHeader, Compression};
use crate::geometry::InclusiveRectangle;

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UpdateType {
    Orders = 0x0000,
    Bitmap = 0x0001,
    Palette = 0x0002,
    Synchronize = 0x0003,
}

impl UpdateType {
    fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0000 => Some(Self::Orders),
            0x0001 => Some(Self::Bitmap),
            0x0002 => Some(Self::Palette),
            0x0003 => Some(Self::Synchronize),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update<'a> {
    Bitmap(BitmapUpdateData<'a>),
    Orders(&'a [u8]),
    Palette(&'a [u8]),
    Synchronize,
}

/// Owned representation for slow-path Bitmap update to avoid exposing lifetimes in ShareDataPdu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapDataOwned {
    pub rectangle: InclusiveRectangle,
    pub width: u16,
    pub height: u16,
    pub bits_per_pixel: u16,
    pub compression_flags: Compression,
    pub compressed_data_header: Option<CompressedDataHeader>,
    pub bitmap_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitmapUpdateOwned {
    pub rectangles: Vec<BitmapDataOwned>,
}

impl IntoOwned for BitmapData<'_> {
    type Owned = BitmapDataOwned;

    fn into_owned(self) -> Self::Owned {
        let BitmapData {
            rectangle,
            width,
            height,
            bits_per_pixel,
            compression_flags,
            compressed_data_header,
            bitmap_data,
        } = self;

        BitmapDataOwned {
            rectangle,
            width,
            height,
            bits_per_pixel,
            compression_flags,
            compressed_data_header,
            bitmap_data: bitmap_data.to_vec(),
        }
    }
}

impl IntoOwned for BitmapUpdateData<'_> {
    type Owned = BitmapUpdateOwned;

    fn into_owned(self) -> Self::Owned {
        let rectangles = self.rectangles.into_iter().map(IntoOwned::into_owned).collect();
        BitmapUpdateOwned { rectangles }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareUpdate {
    Bitmap(BitmapUpdateOwned),
    Orders(Vec<u8>),
    Palette(Vec<u8>),
    Synchronize,
}

impl IntoOwned for Update<'_> {
    type Owned = ShareUpdate;

    fn into_owned(self) -> Self::Owned {
        match self {
            Update::Bitmap(bmp) => ShareUpdate::Bitmap(IntoOwned::into_owned(bmp)),
            Update::Orders(buf) => ShareUpdate::Orders(buf.to_vec()),
            Update::Palette(buf) => ShareUpdate::Palette(buf.to_vec()),
            Update::Synchronize => ShareUpdate::Synchronize,
        }
    }
}

impl<'de> Decode<'de> for Update<'de> {
    /// Decodes slow-path Update payload (PDUTYPE2_UPDATE) that follows the Share Data header.
    ///
    /// Layout:
    /// - updateType: u16
    /// - pad2octets: u16
    /// - updateData: variant by updateType
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        // updateType (u16)
        ensure_size!(in: src, size: 2);

        let Some(update_type) = UpdateType::from_u16(src.read_u16()) else {
            return Err(invalid_field_err!("updateType", "invalid slow-path update type"));
        };

        // pad2octets (u16) — historically present, but some servers omit it.
        // If the next two bytes are zero, consume them; otherwise, treat as absent.
        if src.len() >= 2 && src.peek_u16() == 0 {
            read_padding!(src, 2);
        }

        match update_type {
            UpdateType::Bitmap => Ok(Update::Bitmap(BitmapUpdateData::decode(src)?)),
            UpdateType::Orders => Ok(Update::Orders(src.read_remaining())),
            UpdateType::Palette => Ok(Update::Palette(src.read_remaining())),
            UpdateType::Synchronize => Ok(Update::Synchronize),
        }
    }
}

impl Encode for ShareUpdate {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            ShareUpdate::Bitmap(bmp) => {
                dst.write_u16(UpdateType::Bitmap as u16);
                // pad2octets (u16)
                dst.write_u16(0);

                let rectangles: Vec<BitmapData<'_>> = bmp
                    .rectangles
                    .iter()
                    .map(|r| BitmapData {
                        rectangle: r.rectangle.clone(),
                        width: r.width,
                        height: r.height,
                        bits_per_pixel: r.bits_per_pixel,
                        compression_flags: r.compression_flags,
                        compressed_data_header: r.compressed_data_header.clone(),
                        bitmap_data: r.bitmap_data.as_slice(),
                    })
                    .collect();

                BitmapUpdateData { rectangles }.encode(dst)
            }
            ShareUpdate::Orders(buf) => {
                dst.write_u16(UpdateType::Orders as u16);
                // pad2octets (u16)
                dst.write_u16(0);
                dst.write_slice(buf.as_slice());
                Ok(())
            }
            ShareUpdate::Palette(buf) => {
                dst.write_u16(UpdateType::Palette as u16);
                // pad2octets (u16)
                dst.write_u16(0);
                dst.write_slice(buf.as_slice());
                Ok(())
            }
            ShareUpdate::Synchronize => {
                dst.write_u16(UpdateType::Synchronize as u16);
                // pad2octets (u16)
                dst.write_u16(0);
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        "ShareUpdate"
    }

    fn size(&self) -> usize {
        match self {
            ShareUpdate::Bitmap(bmp) => {
                let rectangles: Vec<BitmapData<'_>> = bmp
                    .rectangles
                    .iter()
                    .map(|r| BitmapData {
                        rectangle: r.rectangle.clone(),
                        width: r.width,
                        height: r.height,
                        bits_per_pixel: r.bits_per_pixel,
                        compression_flags: r.compression_flags,
                        compressed_data_header: r.compressed_data_header.clone(),
                        bitmap_data: r.bitmap_data.as_slice(),
                    })
                    .collect();
                2 /* updateType */ + 2 /* pad2octets */ + BitmapUpdateData { rectangles }.size()
            }
            ShareUpdate::Orders(buf) | ShareUpdate::Palette(buf) => 2 /* updateType */ + 2 /* pad */ + buf.len(),
            ShareUpdate::Synchronize => {
                2 /* updateType */ + 2 /* pad */
            }
        }
    }
}

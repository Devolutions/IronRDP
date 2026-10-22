//! Geometry Tracking Virtual Channel Extension PDUs defined by [MS-RDPEGT].
//!
//! [MS-RDPEGT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/

use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

/// Size of the fields shared by every packet.
const COMMON_SIZE: usize = 4 /* cbGeometryData */
    + 4 // Version
    + 8 // MappingId
    + 4 // UpdateType
    + 4; // Flags
/// Size of a packet without its geometry buffer.
const FIXED_SIZE: usize = COMMON_SIZE
    + 8 // TopLevelId
    + RECT_SIZE // Left, Top, Right, Bottom
    + RECT_SIZE // TopLevelLeft, TopLevelTop, TopLevelRight, TopLevelBottom
    + 4 // GeometryType
    + 4; // cbGeometryBuffer
const RECT_SIZE: usize = 4 /* left */ + 4 /* top */ + 4 /* right */ + 4 /* bottom */;
const REGION_HEADER_SIZE: usize = 4 /* dwSize */ + 4 /* iType */ + 4 /* nCount */ + 4 /* nRgnSize */ + RECT_SIZE;

const UPDATE_TYPE_UPDATE: u32 = 0x01;
const UPDATE_TYPE_CLEAR: u32 = 0x02;
const GEOMETRY_TYPE_REGION: u32 = 0x02;
const RDH_RECTANGLES: u32 = 0x01;

/// Protocol version sent by RDP 8 servers.
pub const VERSION: u32 = 0x01;
/// Largest accepted number of rectangles in a region.
pub const MAX_REGION_RECTS: usize = 4096;

/// A rectangle with exclusive right and bottom edges, as in a Windows `RECT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeometryRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl GeometryRect {
    pub fn width(&self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    pub fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// Moves the rectangle by `dx`, `dy`.
    #[must_use]
    pub fn offset(&self, dx: i32, dy: i32) -> Self {
        Self {
            left: self.left.saturating_add(dx),
            top: self.top.saturating_add(dy),
            right: self.right.saturating_add(dx),
            bottom: self.bottom.saturating_add(dy),
        }
    }

    /// Intersection of two rectangles, or `None` when they do not overlap.
    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let rect = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (!rect.is_empty()).then_some(rect)
    }

    fn decode_ordered(field: &'static str, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let rect = Self {
            left: src.read_i32(),
            top: src.read_i32(),
            right: src.read_i32(),
            bottom: src.read_i32(),
        };
        if rect.right < rect.left || rect.bottom < rect.top {
            return Err(invalid_field_err!(field, "rectangle edges are out of order", in: src));
        }
        Ok(rect)
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) {
        dst.write_i32(self.left);
        dst.write_i32(self.top);
        dst.write_i32(self.right);
        dst.write_i32(self.bottom);
    }
}

/// Visible parts of a tracked rectangle, from the `RGNDATA` in **pGeometryBuffer**.
///
/// Rectangles are relative to the tracked rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionData {
    pub bound: GeometryRect,
    pub rects: Vec<GeometryRect>,
}

impl RegionData {
    fn size(&self) -> usize {
        REGION_HEADER_SIZE + self.rects.len() * RECT_SIZE
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.rects.len() > MAX_REGION_RECTS {
            return Err(invalid_field_err!("nCount", "region has too many rectangles"));
        }
        dst.write_u32(32);
        dst.write_u32(RDH_RECTANGLES);
        dst.write_u32(
            u32::try_from(self.rects.len()).map_err(|_| invalid_field_err!("nCount", "too many rectangles"))?,
        );
        dst.write_u32(0);
        self.bound.encode(dst);
        for rect in &self.rects {
            rect.encode(dst);
        }
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: REGION_HEADER_SIZE);
        if src.read_u32() != 32 {
            return Err(invalid_field_err!("dwSize", "RGNDATAHEADER size must be 32", in: src));
        }
        if src.read_u32() != RDH_RECTANGLES {
            return Err(invalid_field_err!("iType", "only RDH_RECTANGLES regions are supported", in: src));
        }
        let count = usize::try_from(src.read_u32())
            .map_err(|_| invalid_field_err!("nCount", "rectangle count overflows", in: src))?;
        let _region_size = src.read_u32();
        let bound = GeometryRect::decode_ordered("rcBound", src)?;
        if count > MAX_REGION_RECTS {
            return Err(invalid_field_err!("nCount", "region has too many rectangles", in: src));
        }
        ensure_size!(in: src, size: count * RECT_SIZE);
        let mut rects = Vec::with_capacity(count);
        for _ in 0..count {
            rects.push(GeometryRect::decode_ordered("Buffer", src)?);
        }
        Ok(Self { bound, rects })
    }
}

/// Geometry carried by a `GEOMETRY_UPDATE` packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedGeometry {
    /// Handle of the top-level window in window tracking mode, otherwise zero.
    pub top_level_id: u64,
    /// Tracked rectangle, relative to the top-level rectangle.
    pub rect: GeometryRect,
    /// Top-level rectangle, in virtual desktop coordinates.
    pub top_level_rect: GeometryRect,
    /// Visible parts of the tracked rectangle, when the server sent any.
    pub region: Option<RegionData>,
}

impl MappedGeometry {
    /// Tracked rectangle in virtual desktop coordinates.
    pub fn desktop_rect(&self) -> GeometryRect {
        self.rect.offset(self.top_level_rect.left, self.top_level_rect.top)
    }

    /// Visible parts of the tracked rectangle, in virtual desktop coordinates.
    ///
    /// Following [2.2.1.1], the region is ignored when it is empty or, in window tracking mode,
    /// when it does not intersect its bounding rectangle; the whole tracked rectangle is then
    /// visible.
    ///
    /// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/
    pub fn visible_rects(&self) -> Vec<GeometryRect> {
        let desktop = self.desktop_rect();
        match &self.region {
            Some(region)
                if !region.rects.is_empty()
                    && (self.top_level_id == 0
                        || region.rects.iter().any(|rect| rect.intersect(&region.bound).is_some())) =>
            {
                region
                    .rects
                    .iter()
                    .filter_map(|rect| rect.offset(desktop.left, desktop.top).intersect(&desktop))
                    .collect()
            }
            _ if desktop.is_empty() => Vec::new(),
            _ => vec![desktop],
        }
    }
}

/// The [2.2.1.1] MAPPED_GEOMETRY_PACKET, the only message of the protocol.
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedGeometryPacket {
    pub version: u32,
    pub mapping_id: u64,
    /// `Some` for `GEOMETRY_UPDATE`, `None` for `GEOMETRY_CLEAR`.
    pub geometry: Option<MappedGeometry>,
}

impl Encode for MappedGeometryPacket {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let size = self.size();
        ensure_size!(in: dst, size: size);
        dst.write_u32(u32::try_from(size).map_err(|_| invalid_field_err!("cbGeometryData", "packet is too large"))?);
        dst.write_u32(self.version);
        dst.write_u64(self.mapping_id);
        match &self.geometry {
            None => {
                dst.write_u32(UPDATE_TYPE_CLEAR);
                // The remaining fields are not valid for GEOMETRY_CLEAR and are zeroed.
                dst.write_slice(&[0; FIXED_SIZE - COMMON_SIZE + 4]);
            }
            Some(geometry) => {
                dst.write_u32(UPDATE_TYPE_UPDATE);
                dst.write_u32(0);
                dst.write_u64(geometry.top_level_id);
                geometry.rect.encode(dst);
                geometry.top_level_rect.encode(dst);
                dst.write_u32(GEOMETRY_TYPE_REGION);
                match &geometry.region {
                    None => dst.write_u32(0),
                    Some(region) => {
                        dst.write_u32(
                            u32::try_from(region.size())
                                .map_err(|_| invalid_field_err!("cbGeometryBuffer", "region is too large"))?,
                        );
                        region.encode(dst)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "MAPPED_GEOMETRY_PACKET"
    }

    fn size(&self) -> usize {
        FIXED_SIZE
            + self
                .geometry
                .as_ref()
                .and_then(|geometry| geometry.region.as_ref())
                .map_or(0, RegionData::size)
    }
}

impl<'de> Decode<'de> for MappedGeometryPacket {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: COMMON_SIZE);
        let size = usize::try_from(src.read_u32())
            .map_err(|_| invalid_field_err!("cbGeometryData", "packet length overflows", in: src))?;
        // The packet may be followed by a reserved byte that `cbGeometryData` does not count.
        if size < COMMON_SIZE || size - 4 > src.len() {
            return Err(invalid_field_err!(
                "cbGeometryData",
                "packet length does not match the received data",
                in: src
            ));
        }
        let mut src = ReadCursor::new(src.read_slice(size - 4));
        let version = src.read_u32();
        let mapping_id = src.read_u64();
        let update_type = src.read_u32();
        let _flags = src.read_u32();
        let geometry = match update_type {
            UPDATE_TYPE_CLEAR => None,
            UPDATE_TYPE_UPDATE => {
                ensure_size!(in: src, size: FIXED_SIZE - COMMON_SIZE);
                let top_level_id = src.read_u64();
                let rect = GeometryRect::decode_ordered("Left/Top/Right/Bottom", &mut src)?;
                let top_level_rect = GeometryRect::decode_ordered("TopLevelLeft/TopLevelTop", &mut src)?;
                if src.read_u32() != GEOMETRY_TYPE_REGION {
                    return Err(invalid_field_err!("GeometryType", "geometry type must be 0x02", in: src));
                }
                let buffer_size = usize::try_from(src.read_u32())
                    .map_err(|_| invalid_field_err!("cbGeometryBuffer", "buffer length overflows", in: src))?;
                ensure_size!(in: src, size: buffer_size);
                let region = if buffer_size == 0 {
                    None
                } else {
                    let mut buffer = ReadCursor::new(src.read_slice(buffer_size));
                    Some(RegionData::decode(&mut buffer)?)
                };
                Some(MappedGeometry {
                    top_level_id,
                    rect,
                    top_level_rect,
                    region,
                })
            }
            _ => {
                return Err(invalid_field_err!("UpdateType", "unknown geometry update type", in: src));
            }
        };
        Ok(Self {
            version,
            mapping_id,
            geometry,
        })
    }
}

impl DvcEncode for MappedGeometryPacket {}

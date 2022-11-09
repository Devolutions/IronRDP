#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::rdp::CapabilitySetsError;
use crate::PduParsing;

const ORDER_LENGTH: usize = 84;
const ORD_LEVEL_1_ORDERS: u16 = 1;
const SUPPORT_ARRAY_LEN: usize = 32;
const DESKTOP_SAVE_Y_GRAN_VAL: u16 = 20;

#[derive(Copy, Clone)]
pub enum OrderSupportIndex {
    DstBlt = 0x00,
    PatBlt = 0x01,
    ScrBlt = 0x02,
    MemBlt = 0x03,
    Mem3Blt = 0x04,
    DrawnInEGrid = 0x07,
    LineTo = 0x08,
    MultiDrawnInEGrid = 0x09,
    SaveBitmap = 0x0B,
    MultiDstBlt = 0x0F,
    MultiPatBlt = 0x10,
    MultiScrBlt = 0x11,
    MultiOpaqueRect = 0x12,
    Fast = 0x13,
    PolygonSC = 0x14,
    PolygonCB = 0x15,
    Polyline = 0x16,
    FastGlyph = 0x18,
    EllipseSC = 0x19,
    EllipseCB = 0x1A,
    Index = 0x1B,
}

bitflags! {
    pub struct OrderFlags: u16 {
        const NEGOTIATE_ORDER_SUPPORT = 0x0002;
        const ZERO_BOUNDS_DELTAS_SUPPORT = 0x0008;
        const COLOR_INDEX_SUPPORT = 0x0020;
        const SOLID_PATTERN_BRUSH_ONLY = 0x0040;
        const ORDER_FLAGS_EXTRA_FLAGS = 0x0080;
    }
}

bitflags! {
    pub struct OrderSupportExFlags: u16 {
        const CACHE_BITMAP_REV3_SUPPORT = 2;
        const ALTSEC_FRAME_MARKER_SUPPORT = 4;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Order {
    pub order_flags: OrderFlags,
    order_support: [u8; SUPPORT_ARRAY_LEN],
    pub order_support_ex_flags: OrderSupportExFlags,
    pub desktop_save_size: u32,
    pub text_ansi_code_page: u16,
}

impl Order {
    pub fn new(
        order_flags: OrderFlags,
        order_support_ex_flags: OrderSupportExFlags,
        desktop_save_size: u32,
        text_ansi_code_page: u16,
    ) -> Self {
        Self {
            order_flags,
            order_support: [0; SUPPORT_ARRAY_LEN],
            order_support_ex_flags,
            desktop_save_size,
            text_ansi_code_page,
        }
    }

    pub fn set_support_flag(&mut self, flag: OrderSupportIndex, value: bool) {
        self.order_support[flag as usize] = u8::from(value)
    }

    pub fn get_support_flag(&mut self, flag: OrderSupportIndex) -> bool {
        self.order_support[flag as usize] == 1
    }
}

impl PduParsing for Order {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _terminal_descriptor = buffer.read_u128::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _desktop_save_x_granularity = buffer.read_u16::<LittleEndian>()?;
        let _desktop_save_y_granularity = buffer.read_u16::<LittleEndian>()?;
        let _padding = buffer.read_u16::<LittleEndian>()?;
        let _max_order_level = buffer.read_u16::<LittleEndian>()?;
        let _num_fonts = buffer.read_u16::<LittleEndian>()?;

        let order_flags = OrderFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);

        let mut order_support = [0u8; SUPPORT_ARRAY_LEN];
        buffer.read_exact(&mut order_support)?;

        let _text_flags = buffer.read_u16::<LittleEndian>()?;

        let order_support_ex_flags = OrderSupportExFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);

        let _padding = buffer.read_u32::<LittleEndian>()?;
        let desktop_save_size = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u16::<LittleEndian>()?;
        let _padding = buffer.read_u16::<LittleEndian>()?;
        let text_ansi_code_page = buffer.read_u16::<LittleEndian>()?;
        let _padding = buffer.read_u16::<LittleEndian>()?;

        Ok(Order {
            order_flags,
            order_support,
            order_support_ex_flags,
            desktop_save_size,
            text_ansi_code_page,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u128::<LittleEndian>(0)?;

        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(1)?; // desktopSaveXGranularity
        buffer.write_u16::<LittleEndian>(DESKTOP_SAVE_Y_GRAN_VAL)?;
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(ORD_LEVEL_1_ORDERS)?; // maximumOrderLevel
        buffer.write_u16::<LittleEndian>(0)?; // numberFonts
        buffer.write_u16::<LittleEndian>(self.order_flags.bits())?;
        buffer.write_all(&self.order_support)?;
        buffer.write_u16::<LittleEndian>(0)?; // textFlags
        buffer.write_u16::<LittleEndian>(self.order_support_ex_flags.bits())?;
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(self.desktop_save_size)?;
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(0)?; // padding
        buffer.write_u16::<LittleEndian>(self.text_ansi_code_page)?;
        buffer.write_u16::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        ORDER_LENGTH
    }
}

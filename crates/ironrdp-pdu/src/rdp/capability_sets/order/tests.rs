use lazy_static::lazy_static;

use super::*;
use crate::{decode, encode_vec};

const ORDER_BUFFER: [u8; 84] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, // pad4octetsA
    0x01, 0x00, // desktopSaveXGranularity
    0x14, 0x00, // desktopSaveYGranularity
    0x00, 0x00, // pad2octetsA
    0x01, 0x00, // maximumOrderLevel
    0x00, 0x00, // numberFonts
    0x22, 0x00, // orderFlags
    0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, // orderSupport
    0x00, 0x00, // textFlags
    0x02, 0x00, // orderSupportExFlags
    0x00, 0x00, 0x00, 0x00, // pad4octetsB
    0x00, 0x84, 0x03, 0x00, // desktopSaveSize
    0x00, 0x00, // pad2octetsC
    0x00, 0x00, // pad2octetsD
    0x00, 0x00, // testANSICodePage
    0x00, 0x00, // pad2octetsE
];

lazy_static! {
    pub static ref ORDER: Order = Order {
        order_flags: OrderFlags::COLOR_INDEX_SUPPORT | OrderFlags::NEGOTIATE_ORDER_SUPPORT,
        order_support: {
            let mut array = [0u8; 32];

            array[OrderSupportIndex::DstBlt as usize] = 1;
            array[OrderSupportIndex::PatBlt as usize] = 1;
            array[OrderSupportIndex::ScrBlt as usize] = 1;
            array[OrderSupportIndex::MemBlt as usize] = 1;
            array[OrderSupportIndex::Mem3Blt as usize] = 1;
            array[OrderSupportIndex::DrawnInEGrid as usize] = 1;
            array[OrderSupportIndex::LineTo as usize] = 1;
            array[OrderSupportIndex::MultiDrawnInEGrid as usize] = 1;
            array[OrderSupportIndex::SaveBitmap as usize] = 1;
            array[OrderSupportIndex::MultiDstBlt as usize] = 1;
            array[OrderSupportIndex::MultiPatBlt as usize] = 1;
            array[OrderSupportIndex::MultiScrBlt as usize] = 1;
            array[OrderSupportIndex::MultiOpaqueRect as usize] = 1;
            array[OrderSupportIndex::Fast as usize] = 1;
            array[OrderSupportIndex::PolygonSC as usize] = 1;
            array[OrderSupportIndex::PolygonCB as usize] = 1;
            array[OrderSupportIndex::Polyline as usize] = 1;
            array[OrderSupportIndex::FastGlyph as usize] = 1;
            array[OrderSupportIndex::EllipseSC as usize] = 1;
            array[OrderSupportIndex::EllipseCB as usize] = 1;
            array[OrderSupportIndex::Index as usize] = 1;

            array
        },

        order_support_ex_flags: OrderSupportExFlags::CACHE_BITMAP_REV3_SUPPORT,
        desktop_save_size: 230_400,
        text_ansi_code_page: 0,
    };
}

#[test]
fn from_buffer_correctly_parses_order_capset() {
    let buffer = ORDER_BUFFER.as_ref();

    assert_eq!(*ORDER, decode(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_order_capset() {
    let capset = ORDER.clone();

    let buffer = encode_vec(&capset).unwrap();

    assert_eq!(buffer, ORDER_BUFFER.as_ref());
}

#[test]
fn buffer_length_is_correct_for_order_capset() {
    let correct_buffer_length = ORDER_BUFFER.len();

    assert_eq!(correct_buffer_length, ORDER.size());
}

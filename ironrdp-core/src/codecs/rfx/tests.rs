use lazy_static::lazy_static;

use super::data_messages::*;
use super::header_messages::*;
use crate::PduBufferParsing;

const SYNC_PDU_BUFFER: [u8; 12] = [
    0xc0, 0xcc, // TS_RFX_SYNC::BlockT::blockType = WBT_SYNC
    0x0c, 0x00, 0x00, 0x00, // TS_RFX_SYNC::BlockT::blockLen = 12
    0xca, 0xac, 0xcc, 0xca, // TS_RFX_SYNC::magic = WF_MAGIC
    0x00, 0x01, // TS_RFX_SYNC::version = 0x0100
];
const SYNC_PDU_BUFFER_WITH_ZERO_DATA_LENGTH: [u8; 12] = [
    0xc0, 0xcc, // TS_RFX_SYNC::BlockT::blockType = WBT_SYNC
    0x00, 0x00, 0x00, 0x00, // TS_RFX_SYNC::BlockT::blockLen = 0
    0xca, 0xac, 0xcc, 0xca, // TS_RFX_SYNC::magic = WF_MAGIC
    0x00, 0x01, // TS_RFX_SYNC::version = 0x0100
];
const SYNC_PDU_BUFFER_WITH_BIG_DATA_LENGTH: [u8; 12] = [
    0xc0, 0xcc, // TS_RFX_SYNC::BlockT::blockType = WBT_SYNC
    0xff, 0x00, 0x00, 0x00, // TS_RFX_SYNC::BlockT::blockLen = 0xff
    0xca, 0xac, 0xcc, 0xca, // TS_RFX_SYNC::magic = WF_MAGIC
    0x00, 0x01, // TS_RFX_SYNC::version = 0x0100
];
const SYNC_PDU_BUFFER_WITH_SMALL_BUFFER: [u8; 10] = [
    0xc6, 0xcc, // TS_RFX_SYNC::BlockT::blockType = WBT_REGION
    0x0c, 0x00, 0x00, 0x00, // TS_RFX_SYNC::BlockT::blockLen = 0x0c
    0x01, 0x00, 0x00, 0x00,
];
const CODEC_VERSIONS_PDU_BUFFER: [u8; 10] = [
    0xc1, 0xcc, // TS_RFX_CODEC_VERSIONS::BlockT::blockType = WBT_CODEC_VERSION
    0x0a, 0x00, 0x00, 0x00, // TS_RFX_CODEC_VERSIONS::BlockT::blockLen = 10
    0x01, // TS_RFX_CODEC_VERSIONS::numCodecs = 1
    0x01, // TS_RFX_CODEC_VERSIONS::TS_RFX_CODEC_VERSIONT::codecId = 1
    0x00, 0x01, // TS_RFX_CODEC_VERSIONS::TS_RFX_CODEC_VERSIONT::version 0x0100
];
const CHANNELS_PDU_BUFFER: [u8; 17] = [
    0xc2, 0xcc, // TS_RFX_CHANNELS::BLockT::blockType = WBT_CHANNELS
    0x11, 0x00, 0x00, 0x00, // TS_RFX_CHANNELS::BlockT::blockLen = 17
    0x02, // TS_RFX_CHANNELS::numChannels = 2
    0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::channelId = 0
    0x40, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::width = 64
    0x40, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::height = 64
    0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::channelId = 0
    0x20, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::width = 32
    0x20, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::height = 32
];
const CHANNELS_PDU_BUFFER_WITH_INVALID_DATA_LENGTH: [u8; 17] = [
    0xc2, 0xcc, // TS_RFX_CHANNELS::BLockT::blockType = WBT_CHANNELS
    0x11, 0x00, 0x00, 0x00, // TS_RFX_CHANNELS::BlockT::blockLen = 17
    0x0a, // TS_RFX_CHANNELS::numChannels = 0x0a
    0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::channelId = 0
    0x40, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::width = 64
    0x40, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::height = 64
    0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::channelId = 0
    0x20, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::width = 32
    0x20, 0x00, // TS_RFX_CHANNELS::TS_RFX_CHANNELT::height = 32
];
const CONTEXT_PDU_BUFFER: [u8; 13] = [
    0xc3, 0xcc, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockType = WBT_CONTEXT
    0x0d, 0x00, 0x00, 0x00, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockLen = 13
    0x01, // TS_RFX_CONTEXT::CodecChannelT::codecId = 1
    0xff, // TS_RFX_CONTEXT::CodecChannelT::channelId = 255
    0x00, // TS_RFX_CONTEXT::ctxId = 0
    0x40, 0x00, // TS_RFX_CONTEXT::tileSize = 64
    0x28,
    0x28, // TS_RFX_CONTEXT::properties
          // TS_RFX_CONTEXT::properties::flags = VIDEO_MODE (0)
          // TS_RFX_CONTEXT::properties::cct = COL_CONV_ICT (1)
          // TS_RFX_CONTEXT::properties::xft = CLW_XFORM_DWT_53_A (1)
          // TS_RFX_CONTEXT::properties::et = CLW_ENTROPY_RLGR3 (4)
          // TS_RFX_CONTEXT::properties::qt = SCALAR_QUANTIZATION (1)
          // TS_RFX_CONTEXT::properties::r = RESERVED
];
const CONTEXT_PDU_BUFFER_WITH_ZERO_DATA_LENGTH: [u8; 13] = [
    0xc3, 0xcc, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockType = WBT_CONTEXT
    0x01, 0x00, 0x00, 0x00, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockLen = 1
    0x01, // TS_RFX_CONTEXT::CodecChannelT::codecId = 1
    0xff, // TS_RFX_CONTEXT::CodecChannelT::channelId = 255
    0x00, // TS_RFX_CONTEXT::ctxId = 0
    0x40, 0x00, // TS_RFX_CONTEXT::tileSize = 64
    0x28, 0x28, // TS_RFX_CONTEXT::properties
];
const CONTEXT_PDU_BUFFER_WITH_BIG_DATA_LENGTH: [u8; 13] = [
    0xc3, 0xcc, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockType = WBT_CONTEXT
    0xff, 0x00, 0x00, 0x00, // TS_RFX_CONTEXT::CodecChannelT::BlockT::blockLen = 0xff
    0x01, // TS_RFX_CONTEXT::CodecChannelT::codecId = 1
    0xff, // TS_RFX_CONTEXT::CodecChannelT::channelId = 255
    0x00, // TS_RFX_CONTEXT::ctxId = 0
    0x40, 0x00, // TS_RFX_CONTEXT::tileSize = 64
    0x28, 0x28, // TS_RFX_CONTEXT::properties
];
const FRAME_BEGIN_PDU_BUFFER: [u8; 14] = [
    0xc4, 0xcc, // TS_RFX_FRAME_BEGIN::CodecChannelT::blockType = WBT_FRAME_BEGIN
    0x0e, 0x00, 0x00, 0x00, // TS_RFX_FRAME_BEGIN::CodecChannelT::blockLen = 14
    0x01, // TS_RFX_FRAME_BEGIN::CodecChannelT::codecId = 1
    0x00, // TS_RFX_FRAME_BEGIN::CodecChannelT::channelId = 0
    0x00, 0x00, 0x00, 0x00, // TS_RFX_FRAME_BEGIN::frameIdx = 0
    0x01, 0x00, // TS_RFX_FRAME_BEGIN::numRegions  = 1
];
const FRAME_END_PDU_BUFFER: [u8; 8] = [
    0xc5, 0xcc, // TS_RFX_FRAME_END::CodecChannelT::blockType = WBT_FRAME_END
    0x08, 0x00, 0x00, 0x00, // TS_FRAME_END::CodecChannelT::blockLen = 14
    0x01, // TS_FRAME_END::CodecChannelT::codecId = 1
    0x00, // TS_FRAME_END::CodecChannelT::channelId = 0
];
const REGION_PDU_BUFFER: [u8; 31] = [
    0xc6, 0xcc, // TS_RFX_REGION::CodecChannelT::blockType = WBT_REGION
    0x1f, 0x00, 0x00, 0x00, // TS_RFX_REGION::CodecChannelT::blockLen = 31
    0x01, // TS_RFX_REGION::CodecChannelT::codecId = 1
    0x00, // TS_RFX_REGION::CodecChannelT::channelId = 0
    0x01, // TS_RFX_REGION::regionFlags
    //TS_RFX_REGION::regionFlags::lrf = 1
    0x02, 0x00, // TS_RFX_REGION::numRects = 2
    0x00, 0x00, // TS_RFX_REGION::TS_RFX_RECT::x = 0
    0x00, 0x00, // TS_RFX_REGION::TS_RFX_RECT::y = 0
    0x40, 0x00, // TS_RFX_REGION::TS_RFX_RECT::width = 64
    0x40, 0x00, // TS_RFX_REGION::TS_RFX_RECT::height = 64
    0x40, 0x00, // TS_RFX_REGION::TS_RFX_RECT::x = 64
    0x40, 0x00, // TS_RFX_REGION::TS_RFX_RECT::y = 64
    0xff, 0x00, // TS_RFX_REGION::TS_RFX_RECT::width = 0xff
    0xff, 0x00, // TS_RFX_REGION::TS_RFX_RECT::height = 0xff
    0xc1, 0xca, // TS_RFX_REGION::regionType = CBT_REGION
    0x01, 0x00, // TS_RFX_REGION::numTilesets = 1
];

const TILESET_PDU_BUFFER: [u8; 82] = [
    0xc7, 0xcc, // TS_RFX_TILESET::CodecChannelT::blockType = WBT_EXTENSION
    0x52, 0x00, 0x00, 0x00, // TS_RFX_TILESET::CodecChannelT::blockLen = 82
    0x01, // TS_RFX_TILESET::codecId = 1
    0x00, // TS_RFX_TILESET::channelId = 0
    0xc2, 0xca, // TS_RFX_TILESET::subtype = CBT_TILESET
    0x00, 0x00, // TS_RFX_TILESET::idx = 0x00
    0x51, 0x50, // TS_RFX_TILESET::properties
    //TS_RFX_TILESET::properties::lt = TRUE (1)
    //TS_RFX_TILESET::properties::flags =  VIDEO_MODE (0)
    //TS_RFX_TILESET::properties::cct = COL_CONV_ICT (1)
    //TS_RFX_TILESET::properties::xft = CLW_XFORM_DWT_53_A (1)
    //TS_RFX_TILESET::properties::et = CLW_ENTROPY_RLGR3 (4)
    //TS_RFX_TILESET::properties::qt = SCALAR_QUANTIZATION (1)
    0x02, // TS_RFX_TILESET::numQuant = 2
    0x40, // TS_RFX_TILESET::tileSize = 64
    0x02, 0x00, // TS_RFX_TILESET::numTiles = 2
    0x32, 0x00, 0x00, 0x00, // TS_RFX_TILESET::tilesDataSize = 50
    0x66, 0x66, 0x77, 0x88, 0x98, // TS_RFX_TILESET::quant #1
    0x66, 0x66, 0x77, 0x88, 0x98, // TS_RFX_TILESET::quant #2
    //TS_RFX_TILESET::quantVals::LL3 = 6
    //TS_RFX_TILESET::quantVals::LH3 = 6
    //TS_RFX_TILESET::quantVals::HL3 = 6
    //TS_RFX_TILESET::quantVals::HH3 = 6
    //TS_RFX_TILESET::quantVals::LH2 = 7
    //TS_RFX_TILESET::quantVals::HL2 = 7
    //TS_RFX_TILESET::quantVals::HH2 = 8
    //TS_RFX_TILESET::quantVals::LH1 = 8
    //TS_RFX_TILESET::quantVals::HL1 = 8
    //TS_RFX_TILESET::quantVals::HH1 = 9
    // TILE #1
    0xc3, 0xca, // TS_RFX_TILE::BlockT::blockType = CBT_TILE
    0x19, 0x00, 0x00, 0x00, // TS_RFX_TILE::BlockT::blockLen = 25
    0x00, // TS_RFX_TILE::quantIdxY = 0
    0x00, // TS_RFX_TILE::quantIdxCb = 0
    0x00, // TS_RFX_TILE::quantIdxCr = 0
    0x00, 0x00, // TS_RFX_TILE::xIdx = 0
    0x00, 0x00, // TS_RFX_TILE::yIdx = 0
    0x01, 0x00, // TS_RFX_TILE::YLen = 1
    0x02, 0x00, // TS_RFX_TILE::CbLen = 2
    0x03, 0x00, // TS_RFX_TILE::CrLen = 3
    0xf0, // TS_RFX_TILE::YData
    0xf1, 0xf2, // TS_RFX_TILE::CbData
    0xf3, 0xf4, 0xf5, // TS_RFX_TILE::CrData
    // TILE #2
    0xc3, 0xca, // TS_RFX_TILE::BlockT::blockType = CBT_TILE
    0x19, 0x00, 0x00, 0x00, // TS_RFX_TILE::BlockT::blockLen = 25
    0xff, // TS_RFX_TILE::quantIdxY = 0
    0xff, // TS_RFX_TILE::quantIdxCb = 0
    0xff, // TS_RFX_TILE::quantIdxCr = 0
    0xff, 0xff, // TS_RFX_TILE::xIdx = 0
    0xff, 0xff, // TS_RFX_TILE::yIdx = 0
    0x01, 0x00, // TS_RFX_TILE::YLen = 1
    0x02, 0x00, // TS_RFX_TILE::CbLen = 2
    0x03, 0x00, // TS_RFX_TILE::CrLen = 3
    0xf6, // TS_RFX_TILE::YData
    0xf7, 0xf8, // TS_RFX_TILE::CbData
    0xf9, 0xfa, 0xfb, // TS_RFX_TILE::CrData
];
const TILE1_Y_DATA: [u8; 1] = [0xf0];
const TILE1_CB_DATA: [u8; 2] = [0xf1, 0xf2];
const TILE1_CR_DATA: [u8; 3] = [0xf3, 0xf4, 0xf5];
const TILE2_Y_DATA: [u8; 1] = [0xf6];
const TILE2_CB_DATA: [u8; 2] = [0xf7, 0xf8];
const TILE2_CR_DATA: [u8; 3] = [0xf9, 0xfa, 0xfb];

const TILESET_PDU_BUFFER_WITH_INVALID_NUMBER_OF_QUANTS: [u8; 27] = [
    0xc7, 0xcc, // TS_RFX_TILESET::CodecChannelT::blockType = WBT_EXTENSION
    0xd9, 0x03, 0x00, 0x00, // TS_RFX_TILESET::CodecChannelT::blockLen = 985
    0x01, // TS_RFX_TILESET::codecId = 1
    0x00, // TS_RFX_TILESET::channelId = 0
    0xc2, 0xca, // TS_RFX_TILESET::subtype = CBT_TILESET
    0x00, 0x00, // TS_RFX_TILESET::idx = 0x00
    0x51, 0x50, // TS_RFX_TILESET::properties
    0x0f, // TS_RFX_TILESET::numQuant = 0x0f
    0x40, // TS_RFX_TILESET::tileSize = 64
    0x01, 0x00, // TS_RFX_TILESET::numTiles = 1
    0xdf, 0x03, 0x00, 0x00, // TS_RFX_TILESET::tilesDataSize = 991
    0x66, 0x66, 0x77, 0x88, 0x98, // TS_RFX_TILESET::quantVals
];

const TILESET_PDU_BUFFER_WITH_INVALID_TILES_DATA_SIZE: [u8; 27] = [
    0xc7, 0xcc, // TS_RFX_TILESET::CodecChannelT::blockType = WBT_EXTENSION
    0xd9, 0x03, 0x00, 0x00, // TS_RFX_TILESET::CodecChannelT::blockLen = 985
    0x01, // TS_RFX_TILESET::codecId = 1
    0x00, // TS_RFX_TILESET::channelId = 0
    0xc2, 0xca, // TS_RFX_TILESET::subtype = CBT_TILESET
    0x00, 0x00, // TS_RFX_TILESET::idx = 0x00
    0x51, 0x50, // TS_RFX_TILESET::properties
    0x0f, // TS_RFX_TILESET::numQuant = 0x0f
    0x40, // TS_RFX_TILESET::tileSize = 64
    0x01, 0x00, // TS_RFX_TILESET::numTiles = 1
    0xff, 0xff, 0xff, 0xff, // TS_RFX_TILESET::tilesDataSize = 0xffff_ffff
    0x66, 0x66, 0x77, 0x88, 0x98, // TS_RFX_TILESET::quantVals
];

const SYNC_PDU: SyncPdu = SyncPdu;
const CODEC_VERSIONS_PDU: CodecVersionsPdu = CodecVersionsPdu;
const CONTEXT_PDU: ContextPdu = ContextPdu {
    flags: OperatingMode::empty(),
    entropy_algorithm: EntropyAlgorithm::Rlgr3,
};
const FRAME_BEGIN_PDU: FrameBeginPdu = FrameBeginPdu {
    index: 0,
    number_of_regions: 1,
};
const FRAME_END_PDU: FrameEndPdu = FrameEndPdu;

lazy_static! {
    static ref CHANNELS_PDU: ChannelsPdu = ChannelsPdu(vec![
        Channel { width: 64, height: 64 },
        Channel { width: 32, height: 32 }
    ]);
    static ref REGION_PDU: RegionPdu = RegionPdu {
        rectangles: vec![
            RfxRectangle {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            RfxRectangle {
                x: 64,
                y: 64,
                width: 0xff,
                height: 0xff,
            },
        ]
    };
    static ref TILESET_PDU: TileSetPdu<'static> = TileSetPdu {
        entropy_algorithm: EntropyAlgorithm::Rlgr3,
        quants: vec![
            Quant {
                ll3: 6,
                lh3: 6,
                hl3: 6,
                hh3: 6,
                lh2: 7,
                hl2: 7,
                hh2: 8,
                lh1: 8,
                hl1: 8,
                hh1: 9,
            };
            2
        ],
        tiles: vec![
            Tile {
                y_quant_index: 0,
                cb_quant_index: 0,
                cr_quant_index: 0,

                x: 0,
                y: 0,

                y_data: &TILE1_Y_DATA,
                cb_data: &TILE1_CB_DATA,
                cr_data: &TILE1_CR_DATA,
            },
            Tile {
                y_quant_index: 0xff,
                cb_quant_index: 0xff,
                cr_quant_index: 0xff,

                x: 0xffff,
                y: 0xffff,

                y_data: &TILE2_Y_DATA,
                cb_data: &TILE2_CB_DATA,
                cr_data: &TILE2_CR_DATA,
            },
        ],
    };
}

#[test]
fn from_buffer_for_block_header_returns_error_on_zero_data_length() {
    assert!(SyncPdu::from_buffer(SYNC_PDU_BUFFER_WITH_ZERO_DATA_LENGTH.as_ref()).is_err(),);
}

#[test]
fn from_buffer_for_block_header_returns_error_on_data_length_greater_then_available_data() {
    assert!(SyncPdu::from_buffer(SYNC_PDU_BUFFER_WITH_BIG_DATA_LENGTH.as_ref()).is_err(),);
}

#[test]
fn from_buffer_for_pdu_with_codec_channel_header_returns_error_on_small_buffer() {
    assert!(RegionPdu::from_buffer(SYNC_PDU_BUFFER_WITH_SMALL_BUFFER.as_ref()).is_err());
}

#[test]
fn from_buffer_correctly_parses_sync_pdu() {
    assert_eq!(SYNC_PDU, SyncPdu::from_buffer(SYNC_PDU_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_sync_pdu() {
    let expected = SYNC_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    SYNC_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_sync_pdu() {
    assert_eq!(SYNC_PDU_BUFFER.len(), SYNC_PDU.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_codec_versions_pdu() {
    assert_eq!(
        CODEC_VERSIONS_PDU,
        CodecVersionsPdu::from_buffer(CODEC_VERSIONS_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_codec_versions_pdu() {
    let expected = CODEC_VERSIONS_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    CODEC_VERSIONS_PDU
        .to_buffer_consume(&mut buffer.as_mut_slice())
        .unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_codec_versions_pdu() {
    assert_eq!(CODEC_VERSIONS_PDU_BUFFER.len(), CODEC_VERSIONS_PDU.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_channels_pdu() {
    assert_eq!(
        *CHANNELS_PDU,
        ChannelsPdu::from_buffer(CHANNELS_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_channels_pdu() {
    let expected = CHANNELS_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    CHANNELS_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn from_buffer_returns_error_on_invalid_data_length_for_channels_pdu() {
    assert!(ChannelsPdu::from_buffer(CHANNELS_PDU_BUFFER_WITH_INVALID_DATA_LENGTH.as_ref()).is_err());
}

#[test]
fn buffer_length_is_correct_for_channels_pdu() {
    assert_eq!(CHANNELS_PDU_BUFFER.len(), CHANNELS_PDU.buffer_length());
}

#[test]
fn from_buffer_for_codec_channel_header_returns_error_on_zero_data_length() {
    assert!(ContextPdu::from_buffer(CONTEXT_PDU_BUFFER_WITH_ZERO_DATA_LENGTH.as_ref()).is_err());
}

#[test]
fn from_buffer_for_codec_channel_header_returns_error_on_data_length_greater_then_available_data() {
    assert!(ContextPdu::from_buffer(CONTEXT_PDU_BUFFER_WITH_BIG_DATA_LENGTH.as_ref()).is_err());
}

#[test]
fn from_buffer_correctly_parses_context_pdu() {
    assert_eq!(
        CONTEXT_PDU,
        ContextPdu::from_buffer(CONTEXT_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_context_pdu() {
    let expected = CONTEXT_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    CONTEXT_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_context_pdu() {
    assert_eq!(CONTEXT_PDU_BUFFER.len(), CONTEXT_PDU.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_frame_begin_pdu() {
    assert_eq!(
        FRAME_BEGIN_PDU,
        FrameBeginPdu::from_buffer(FRAME_BEGIN_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_frame_begin_pdu() {
    let expected = FRAME_BEGIN_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    FRAME_BEGIN_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_frame_begin_pdu() {
    assert_eq!(FRAME_BEGIN_PDU_BUFFER.len(), FRAME_BEGIN_PDU.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_frame_end_pdu() {
    assert_eq!(
        FRAME_END_PDU,
        FrameEndPdu::from_buffer(FRAME_END_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_frame_end_pdu() {
    let expected = FRAME_END_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    FRAME_END_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_frame_end_pdu() {
    assert_eq!(FRAME_END_PDU_BUFFER.len(), FRAME_END_PDU.buffer_length());
}

#[test]
fn from_buffer_correctly_parses_region_pdu() {
    assert_eq!(*REGION_PDU, RegionPdu::from_buffer(REGION_PDU_BUFFER.as_ref()).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_region_pdu() {
    let expected = REGION_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    REGION_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_region_pdu() {
    assert_eq!(REGION_PDU_BUFFER.len(), REGION_PDU.buffer_length());
}

#[test]
fn from_buffer_returns_error_on_invalid_number_of_quants_for_tile_set_pdu() {
    assert!(TileSetPdu::from_buffer_consume(&mut TILESET_PDU_BUFFER_WITH_INVALID_NUMBER_OF_QUANTS.as_ref()).is_err());
}

#[test]
fn from_buffer_returns_error_on_invalid_tiles_data_size_for_tile_set_pdu() {
    assert!(TileSetPdu::from_buffer_consume(&mut TILESET_PDU_BUFFER_WITH_INVALID_TILES_DATA_SIZE.as_ref()).is_err());
}

#[test]
fn from_buffer_correctly_parses_tile_set_pdu() {
    assert_eq!(
        *TILESET_PDU,
        TileSetPdu::from_buffer_consume(&mut TILESET_PDU_BUFFER.as_ref()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_tile_set_pdu() {
    let expected = TILESET_PDU_BUFFER.as_ref();
    let mut buffer = vec![0; expected.len()];

    TILESET_PDU.to_buffer_consume(&mut buffer.as_mut_slice()).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

#[test]
fn buffer_length_is_correct_for_tile_set_pdu() {
    assert_eq!(TILESET_PDU_BUFFER.len(), TILESET_PDU.buffer_length());
}

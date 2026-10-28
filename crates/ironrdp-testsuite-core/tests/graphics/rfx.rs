use ironrdp_core::{Encode as _, WriteCursor};
use ironrdp_graphics::rfx::{MessageDecoder, RfxDecodeError, TILE_RGBA_LEN, TileDecoder};
use ironrdp_graphics::rfx_encode_component;
use ironrdp_pdu::codecs::rfx::{
    Block, ChannelsPdu, CodecChannel, ContextPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu, OperatingMode, Quant,
    RegionPdu, RfxChannel, RfxRectangle, Tile, TileSetPdu,
};

fn encode_plane(plane: &mut [i16; 4096], quant: &Quant, entropy: EntropyAlgorithm) -> Vec<u8> {
    let mut out = vec![0; 4096];
    let len = rfx_encode_component(plane, &mut out, quant, entropy).unwrap();
    out.truncate(len);
    out
}

/// Encode a constant component: every coefficient set to `value` before the forward transform.
fn flat_component(value: i16, quant: &Quant, entropy: EntropyAlgorithm) -> Vec<u8> {
    let mut input = [value; 4096];
    encode_plane(&mut input, quant, entropy)
}

fn tile(x: u16, y: u16, data: &[u8]) -> Tile<'_> {
    Tile {
        y_quant_index: 0,
        cb_quant_index: 0,
        cr_quant_index: 0,
        x,
        y,
        y_data: data,
        cb_data: data,
        cr_data: data,
    }
}

fn encode_blocks(blocks: &[Block<'_>]) -> Vec<u8> {
    let mut buffer = vec![0; blocks.iter().map(|block| block.size()).sum()];
    let mut cursor = WriteCursor::new(&mut buffer);
    for block in blocks {
        block.encode(&mut cursor).unwrap();
    }
    buffer
}

fn frame_blocks<'a>(tile_set: TileSetPdu<'a>, rectangles: Vec<RfxRectangle>) -> [Block<'a>; 4] {
    [
        Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
            index: 7,
            number_of_regions: 1,
        })),
        Block::CodecChannel(CodecChannel::Region(RegionPdu { rectangles })),
        Block::CodecChannel(CodecChannel::TileSet(tile_set)),
        Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu)),
    ]
}

fn frame(tile_set: TileSetPdu<'_>, rectangles: Vec<RfxRectangle>) -> Vec<u8> {
    encode_blocks(&frame_blocks(tile_set, rectangles))
}

fn begin() -> Block<'static> {
    Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
        index: 0,
        number_of_regions: 1,
    }))
}

fn region() -> Block<'static> {
    Block::CodecChannel(CodecChannel::Region(RegionPdu { rectangles: vec![] }))
}

fn end() -> Block<'static> {
    Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu))
}

fn empty_tile_set() -> Block<'static> {
    Block::CodecChannel(CodecChannel::TileSet(TileSetPdu {
        entropy_algorithm: EntropyAlgorithm::Rlgr3,
        quants: vec![Quant::default()],
        tiles: vec![],
    }))
}

#[test]
fn a_zero_tile_decodes_to_mid_gray() {
    // oracle: spec MS-RDPRFX §3.1.8.2.5 (Y is level-shifted up by 128; zero chroma is gray)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr3);

    let mut decoder = TileDecoder::new();
    let pixels = decoder
        .decode(&tile(0, 0, &zero), &[quant], EntropyAlgorithm::Rlgr3)
        .unwrap();

    assert_eq!(pixels.len(), TILE_RGBA_LEN);
    for pixel in pixels.chunks_exact(4) {
        assert_eq!(pixel, [128, 128, 128, 255]);
    }
}

#[test]
fn a_tile_naming_a_missing_quant_is_rejected() {
    // oracle: spec MS-RDPRFX §2.2.2.3.4.1 (quantIdx fields index the tileset's quantVals array)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr3);
    let mut bad = tile(0, 0, &zero);
    bad.cb_quant_index = 2;

    let error = TileDecoder::new()
        .decode(&bad, &[quant], EntropyAlgorithm::Rlgr3)
        .unwrap_err();

    assert!(
        matches!(error, RfxDecodeError::InvalidQuantIndex { index: 2, table_len: 1 }),
        "{error:?}"
    );
}

#[test]
fn a_message_without_headers_parses_its_rectangles_and_tiles() {
    // oracle: spec MS-RDPRFX §2.2.2.3.3 and §2.2.2.3.4 (the region and tileset the test wrote)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr3);
    let rectangles = vec![
        RfxRectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        },
        RfxRectangle {
            x: 64,
            y: 64,
            width: 10,
            height: 10,
        },
    ];
    let stream = frame(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quants: vec![quant],
            tiles: vec![tile(0, 0, &zero), tile(1, 1, &zero)],
        },
        rectangles.clone(),
    );

    let message = MessageDecoder::new().parse(&stream).unwrap();

    assert_eq!(message.frames.len(), 1);
    assert_eq!(message.frames[0].rectangles, rectangles);
    assert_eq!(message.tile_count(), 2);
    let positions: Vec<_> = message.frames[0]
        .tile_set
        .tiles
        .iter()
        .map(|tile| (tile.x, tile.y))
        .collect();
    assert_eq!(positions, [(0, 0), (1, 1)]);
}

#[test]
fn every_frame_of_a_message_is_parsed_with_its_own_region() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (a stream carries frames, each bracketed by FRAME_BEGIN and
    // FRAME_END with one region and one tileset)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr3);
    let first = vec![RfxRectangle {
        x: 0,
        y: 0,
        width: 8,
        height: 8,
    }];
    let second = vec![RfxRectangle {
        x: 64,
        y: 0,
        width: 4,
        height: 4,
    }];
    let mut blocks = frame_blocks(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quants: vec![quant.clone()],
            tiles: vec![tile(0, 0, &zero)],
        },
        first.clone(),
    )
    .to_vec();
    blocks.extend(frame_blocks(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quants: vec![quant],
            tiles: vec![tile(1, 0, &zero)],
        },
        second.clone(),
    ));

    let stream = encode_blocks(&blocks);
    let message = MessageDecoder::new().parse(&stream).unwrap();

    let rectangles: Vec<_> = message.frames.iter().map(|frame| frame.rectangles.clone()).collect();
    assert_eq!(rectangles, [first, second]);
    let tiles: Vec<_> = message
        .frames
        .iter()
        .map(|frame| (frame.tile_set.tiles[0].x, frame.tile_set.tiles[0].y))
        .collect();
    assert_eq!(tiles, [(0, 0), (1, 0)]);
}

#[test]
fn a_tileset_disagreeing_with_the_context_entropy_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.1 and §3.1.5 (every data message is encoded with the
    // EntropyAlgorithm the TS_RFX_CONTEXT stored)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr1);
    let mut decoder = MessageDecoder::new();
    let context = encode_blocks(&[
        Block::CodecChannel(CodecChannel::Context(ContextPdu {
            flags: OperatingMode::IMAGE_MODE,
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
        })),
        begin(),
        region(),
        empty_tile_set(),
        end(),
    ]);
    decoder.parse(&context).unwrap();

    // A later message without headers still follows the context of the stream.
    let stream = frame(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quants: vec![quant],
            tiles: vec![tile(0, 0, &zero)],
        },
        vec![],
    );
    let error = decoder.parse(&stream).unwrap_err();

    assert!(
        matches!(
            error,
            RfxDecodeError::EntropyMismatch {
                context: EntropyAlgorithm::Rlgr3,
                tileset: EntropyAlgorithm::Rlgr1
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_message_with_more_tiles_than_any_display_needs_is_rejected_before_decoding() {
    // oracle: mutant (a message past the tile limit decoded instead of rejected)
    let quant = Quant::default();
    let zero = flat_component(0, &quant, EntropyAlgorithm::Rlgr3);
    // numTiles is 16-bit, so the 65,537 tiles go in two frames.
    let tiles = |count: u32| -> Vec<_> {
        (0..count)
            .map(|i| tile(u16::try_from(i % 256).unwrap(), u16::try_from(i / 256).unwrap(), &zero))
            .collect()
    };
    let mut blocks = frame_blocks(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quants: vec![quant.clone()],
            tiles: tiles(40_000),
        },
        vec![],
    )
    .to_vec();
    blocks.extend(frame_blocks(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
            quants: vec![quant],
            tiles: tiles(25_537),
        },
        vec![],
    ));
    let stream = encode_blocks(&blocks);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::TooManyTiles(65_537)), "{error:?}");
}

#[test]
fn a_tileset_before_its_region_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (each frame is a region followed by its tileset)
    let stream = encode_blocks(&[begin(), empty_tile_set(), region(), end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::MissingBlock("REGION")), "{error:?}");
}

#[test]
fn a_header_block_inside_a_frame_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (header messages precede the data messages)
    let context = Block::CodecChannel(CodecChannel::Context(ContextPdu {
        flags: OperatingMode::IMAGE_MODE,
        entropy_algorithm: EntropyAlgorithm::Rlgr3,
    }));
    let stream = encode_blocks(&[begin(), context, region(), empty_tile_set(), end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::UnexpectedBlock("header")), "{error:?}");
}

#[test]
fn channels_without_exactly_one_non_empty_channel_are_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (TS_RFX_CHANNELS MUST contain a single channel) and
    // §2.2.2.1.3 (frame sizes start at 1)
    for channels in [
        vec![],
        vec![
            RfxChannel { width: 64, height: 64 },
            RfxChannel { width: 64, height: 64 },
        ],
        vec![RfxChannel { width: 0, height: 64 }],
        vec![RfxChannel { width: 64, height: -1 }],
    ] {
        let stream = encode_blocks(&[
            Block::Channels(ChannelsPdu(channels.clone())),
            begin(),
            region(),
            empty_tile_set(),
            end(),
        ]);

        let error = MessageDecoder::new().parse(&stream).unwrap_err();

        assert!(
            matches!(error, RfxDecodeError::InvalidChannels),
            "{channels:?}: {error:?}"
        );
    }
}

#[test]
fn a_rejected_message_does_not_change_the_context_for_the_next_one() {
    // oracle: mutant (a context block from a rejected message applied to later messages)
    let quant = Quant::default();
    let rlgr1 = flat_component(0, &quant, EntropyAlgorithm::Rlgr1);
    let mut decoder = MessageDecoder::new();
    // A context switching to RLGR3, in a message rejected for its unterminated frame.
    let rejected = encode_blocks(&[
        Block::CodecChannel(CodecChannel::Context(ContextPdu {
            flags: OperatingMode::IMAGE_MODE,
            entropy_algorithm: EntropyAlgorithm::Rlgr3,
        })),
        begin(),
    ]);
    decoder.parse(&rejected).unwrap_err();

    let stream = frame(
        TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quants: vec![quant],
            tiles: vec![tile(0, 0, &rlgr1)],
        },
        vec![],
    );

    assert!(decoder.parse(&stream).is_ok());
}

#[test]
fn a_frame_without_a_tileset_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (one TS_RFX_TILESET per TS_RFX_REGION)
    let stream = encode_blocks(&[begin(), region(), end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::MissingBlock("TILESET")), "{error:?}");
}

#[test]
fn a_frame_without_a_region_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (one TS_RFX_REGION per frame)
    let stream = encode_blocks(&[begin(), empty_tile_set(), end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::MissingBlock("REGION")), "{error:?}");
}

#[test]
fn a_frame_with_two_regions_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (there MUST only be one TS_RFX_REGION per frame)
    let stream = encode_blocks(&[begin(), region(), region(), empty_tile_set(), end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::UnexpectedBlock("REGION")), "{error:?}");
}

#[test]
fn a_frame_without_a_frame_end_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (frame data is bracketed by FRAME_BEGIN and FRAME_END)
    let stream = encode_blocks(&[begin(), region(), empty_tile_set()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(matches!(error, RfxDecodeError::MissingBlock("FRAME_END")), "{error:?}");
}

#[test]
fn a_message_without_a_frame_begin_is_rejected() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (data messages start with TS_RFX_FRAME_BEGIN)
    let stream = encode_blocks(&[end()]);

    let error = MessageDecoder::new().parse(&stream).unwrap_err();

    assert!(
        matches!(error, RfxDecodeError::MissingBlock("FRAME_BEGIN")),
        "{error:?}"
    );
}

#[test]
fn quantization_rounds_at_half_step_boundaries_for_both_signs() {
    // oracle: spec MS-RDPRFX §3.1.8.1.5 and §4.2.4.3 (11.5 fixed-point quantization shifts by
    // quant - 1 = 5 for quant = 6, so scale = 32, half = 16, and (value + half) >> 5 rounds
    // half-way ties -48, -16, 16, 48 toward +infinity to -1, 0, 1, 2).
    let quant = Quant {
        ll3: 6,
        lh3: 6,
        hl3: 6,
        hh3: 6,
        lh2: 6,
        hl2: 6,
        hh2: 6,
        lh1: 6,
        hl1: 6,
        hh1: 6,
    };
    let samples = [
        (i16::MIN, -1024),
        (-49, -2),
        (-48, -1), // Half-way tie: (-48 + 16) >> 5 == -1.
        (-32, -1),
        (-31, -1),
        (-17, -1),
        (-16, 0), // Half-way tie: (-16 + 16) >> 5 == 0.
        (-1, 0),
        (0, 0),
        (15, 0),
        (16, 1), // Half-way tie: (16 + 16) >> 5 == 1.
        (31, 1),
        (32, 1),
        (47, 1),
        (48, 2), // Half-way tie: (48 + 16) >> 5 == 2.
        (i16::MAX, 1024),
    ];

    let mut buffer = [0i16; 4096];
    for (idx, (input, _)) in samples.iter().copied().enumerate() {
        buffer[idx] = input;
    }

    ironrdp_graphics::quantization::encode(&mut buffer, &quant);

    for (idx, (input, expected)) in samples.iter().copied().enumerate() {
        assert_eq!(buffer[idx], expected, "input={input}");
    }
}

#[test]
fn forward_dwt_uses_ceiling_average_in_low_pass_subbands() {
    // oracle: spec MS-RDPRFX §3.1.8.2.4 (Figure 14 inverse DWT reconstructs even samples via
    // X[2n] = L[n] - floor((H[n-1] + H[n] + 1) / 2), so forward DWT L, LL, and LH sub-bands must
    // add floor((H[n-1] + H[n] + 1) / 2) to cancel odd H[n-1] + H[n] sums and round-trip exactly).
    const WAVELET_BASIS_POS: [i16; 4] = [-1, 1, -1, -1];
    const WAVELET_BASIS_NEG: [i16; 4] = [1, -2, 0, 0];

    // Exercises dwt_vertical L: each column is [-1, 1, -1, -1, 0, ...], which has H[0] = 1,
    // H[1] = 0, and L = 0 only when L[1] adds (1 + 0 + 1) >> 1 == 1.
    let mut vertical = [0i16; 4096];
    for (y, sample) in WAVELET_BASIS_POS.into_iter().enumerate() {
        vertical[y * 64..(y + 1) * 64].fill(sample);
    }

    // Exercises dwt_horizontal LL: each row is [-1, 1, -1, -1, 0, ...], which has HL1[0] = 1,
    // HL1[1] = 0, and LL1 = 0 only when LL1[1] adds (1 + 0 + 1) >> 1 == 1.
    let mut horizontal = [0i16; 4096];
    for y in 0..64 {
        horizontal[y * 64..y * 64 + 4].copy_from_slice(&WAVELET_BASIS_POS);
    }

    // Exercises dwt_horizontal LH: dwt_vertical produces L = 0 and H row 0 = [-1, 1, -1, -1, 0, ...],
    // which has HH1[0] = 1, HH1[1] = 0, and LH1 = 0 only when LH1[1] adds (1 + 0 + 1) >> 1 == 1.
    let mut diagonal = [0i16; 4096];
    for x in 0..4 {
        let basis = if x == 1 { WAVELET_BASIS_POS } else { WAVELET_BASIS_NEG };
        for (y, sample) in basis.into_iter().enumerate() {
            diagonal[y * 64 + x] = sample;
        }
    }

    for (name, expected, nonzero_subband_index) in [
        ("vertical L", vertical, 1024),
        ("horizontal LL", horizontal, 0),
        ("horizontal LH", diagonal, 2048),
    ] {
        let mut buffer = expected;
        let mut temp = [0i16; 4096];
        ironrdp_graphics::dwt::encode(&mut buffer, &mut temp);
        assert_eq!(buffer[nonzero_subband_index], 1, "{name} impulse coefficient");
        assert!(
            buffer[3072..].iter().all(|&coeff| coeff == 0),
            "{name} low-pass sub-bands must cancel to zero"
        );
        ironrdp_graphics::dwt::decode(&mut buffer, &mut temp);
        assert_eq!(buffer, expected, "{name} round-trip");
    }
}

#[test]
fn a_smooth_gradient_tile_round_trips_without_negative_quantization_spikes() {
    // oracle: spec MS-RDPRFX §3.1.8.1.5 and §4.2.4.3 (11.5 fixed-point quantization rounds via
    // (value + half) >> (quant - 1); small negative detail coefficients on a decreasing gradient
    // round to zero instead of floor-shifting to -1, which previously spiked decoded channels by up
    // to 13 units).
    let mut input = [0u8; TILE_RGBA_LEN];
    for y in 0..64usize {
        for x in 0..64usize {
            let offset = (y * 64 + x) * 4;
            let step = u8::try_from((x + y) / 4).unwrap();
            input[offset] = 180 - step;
            input[offset + 1] = 210 - step;
            input[offset + 2] = 235 - step;
            input[offset + 3] = 255;
        }
    }

    let (mut luma, mut cb, mut cr) = ([0i16; 4096], [0i16; 4096], [0i16; 4096]);
    ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile(
        &input,
        64,
        64,
        64 * 4,
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        &mut luma,
        &mut cb,
        &mut cr,
    )
    .unwrap();

    let quant = Quant::default();
    let y_data = encode_plane(&mut luma, &quant, EntropyAlgorithm::Rlgr3);
    let blue_chroma = encode_plane(&mut cb, &quant, EntropyAlgorithm::Rlgr3);
    let red_chroma = encode_plane(&mut cr, &quant, EntropyAlgorithm::Rlgr3);

    let encoded = Tile {
        y_quant_index: 0,
        cb_quant_index: 0,
        cr_quant_index: 0,
        x: 0,
        y: 0,
        y_data: &y_data,
        cb_data: &blue_chroma,
        cr_data: &red_chroma,
    };

    let mut decoder = TileDecoder::new();
    let pixels = decoder.decode(&encoded, &[quant], EntropyAlgorithm::Rlgr3).unwrap();

    for (actual, expected) in pixels.chunks_exact(4).zip(input.chunks_exact(4)) {
        for channel in 0..3 {
            assert!(
                actual[channel].abs_diff(expected[channel]) <= 4,
                "actual={actual:?} expected={expected:?}"
            );
        }
        assert_eq!(actual[3], 255);
    }
}

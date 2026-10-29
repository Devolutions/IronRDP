use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_core::decode;
use ironrdp_pdu::bitmap::BitmapUpdateData;
use ironrdp_pdu::rdp::capability_sets::{
    CapabilitySet, CmdFlags, EntropyBits, LargePointerSupportFlags, MultifragmentUpdate,
};
use ironrdp_server::{
    BitmapUpdate, ColorPointer, DesktopSize, DisplayUpdate, MAX_FASTPATH_UPDATE_SIZE, PixelFormat, RemoteFxCodec,
    UpdateEncoder, UpdateEncoderCodecs, client_max_request_size,
};
use proptest::prelude::*;
use rstest::rstest;

fn multifragment_update(max_request_size: u32) -> CapabilitySet {
    CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate { max_request_size })
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (MaxRequestSize is the size of the client's reassembly buffer)
#[rstest]
#[case::client_smaller_than_server(8 * 1024 * 1024, 65_536, 65_536)]
#[case::client_larger_than_server(65_536, 8 * 1024 * 1024, 65_536)]
fn negotiated_max_request_size_is_the_smaller_of_both(#[case] server: u32, #[case] client: u32, #[case] expected: u32) {
    assert_eq!(
        client_max_request_size(server, &[multifragment_update(client)]),
        expected
    );
}

// oracle: mutant a zero or missing client size taken as the limit instead of the server's own
#[rstest]
#[case::client_sends_zero(Some(0))]
#[case::client_sends_no_capability_set(None)]
fn without_a_client_size_the_server_keeps_its_own(#[case] client: Option<u32>) {
    let capabilities: Vec<_> = client.into_iter().map(multifragment_update).collect();
    assert_eq!(client_max_request_size(65_536, &capabilities), 65_536);
}

fn one_fragment() -> u32 {
    u32::try_from(MAX_FASTPATH_UPDATE_SIZE).unwrap()
}

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1 (an update sent as a single FASTPATH_FRAGMENT_SINGLE is never reassembled)
#[rstest]
#[case::tiny_client(65_536, Some(1))]
#[case::zero_server(0, Some(65_536))]
#[case::zero_server_without_client_capability(0, None)]
fn the_negotiated_size_does_not_go_below_one_fragment(#[case] server: u32, #[case] client: Option<u32>) {
    let capabilities: Vec<_> = client.into_iter().map(multifragment_update).collect();
    assert_eq!(client_max_request_size(server, &capabilities), one_fragment());
}

#[derive(Clone, Copy, Debug)]
enum Codec {
    Bitmap,
    Raw,
    Qoi,
    Qoiz,
    RemoteFx,
}

fn encoder(codec: Codec, width: u16, height: u16, max_request_size: u32) -> UpdateEncoder {
    let mut codecs = UpdateEncoderCodecs::new();
    let surface_flags = match codec {
        Codec::Bitmap => CmdFlags::empty(),
        Codec::Raw => CmdFlags::SET_SURFACE_BITS,
        Codec::Qoi => {
            codecs.set_qoi(Some(1));
            CmdFlags::SET_SURFACE_BITS
        }
        Codec::Qoiz => {
            codecs.set_qoiz(Some(1));
            CmdFlags::SET_SURFACE_BITS
        }
        Codec::RemoteFx => {
            codecs.set_remotefx_codec(Some((EntropyBits::Rlgr3, 1, RemoteFxCodec::RemoteFx)));
            CmdFlags::SET_SURFACE_BITS
        }
    };
    UpdateEncoder::new(
        DesktopSize { width, height },
        surface_flags,
        codecs,
        max_request_size,
        0,
        0,
        LargePointerSupportFlags::empty(),
    )
    .unwrap()
}

/// A full frame of pixels with no runs, repeats or small deltas for a codec to exploit.
fn incompressible_frame(width: u16, height: u16) -> BitmapUpdate {
    let len = usize::from(width) * usize::from(height) * 4;
    // xorshift64
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let data: Vec<u8> = core::iter::repeat_with(|| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state.to_le_bytes()[0]
    })
    .take(len)
    .collect();
    BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(height).unwrap(),
        format: PixelFormat::BgrA32,
        data: data.into(),
        stride: NonZeroUsize::new(usize::from(width) * 4).unwrap(),
    }
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (a fragmented update larger than MaxRequestSize overflows the client's reassembly buffer)
#[rstest]
#[case::raw_surface_rows_of_2048_pixels(Codec::Raw, 2048, 64, Some(65_536))]
#[case::qoi_surface_rows_of_2048_pixels(Codec::Qoi, 2048, 64, Some(65_536))]
#[case::qoiz_surface_rows_of_2048_pixels(Codec::Qoiz, 2048, 64, Some(65_536))]
#[case::remotefx_surface_rows_of_2048_pixels(Codec::RemoteFx, 2048, 64, Some(65_536))]
#[case::legacy_bitmap(Codec::Bitmap, 1440, 900, Some(65_536))]
#[case::legacy_bitmap_at_one_fragment(Codec::Bitmap, 1440, 900, None)]
// Wider than one fragment's worth of pixels: each row is split by width.
#[case::legacy_bitmap_one_row_strip_at_one_fragment(Codec::Bitmap, 4096, 1, None)]
#[case::legacy_bitmap_rows_split_by_width(Codec::Bitmap, 8188, 64, Some(20_000))]
// RemoteFX encodes whole 64x64 tiles, so a short strip costs as much as 64 rows.
#[case::remotefx_one_row_strip_at_one_fragment(Codec::RemoteFx, 4096, 1, None)]
#[case::remotefx_one_row_strip(Codec::RemoteFx, 8188, 1, Some(65_536))]
#[case::remotefx_1080p_at_one_fragment(Codec::RemoteFx, 1920, 1080, None)]
#[case::remotefx_1440p(Codec::RemoteFx, 2560, 1440, Some(32_768))]
#[tokio::test]
async fn every_encoded_update_fits_the_negotiated_request_size(
    #[case] codec: Codec,
    #[case] width: u16,
    #[case] height: u16,
    #[case] max_request_size: Option<u32>,
) {
    let max_request_size = max_request_size.unwrap_or_else(one_fragment);
    let largest = largest_update(codec, width, height, max_request_size).await;
    assert!(
        largest <= usize::try_from(max_request_size).unwrap(),
        "{largest}-byte update exceeds the {max_request_size}-byte request size"
    );
}

/// The size of the largest update the encoder emits for one incompressible frame.
async fn largest_update(codec: Codec, width: u16, height: u16, max_request_size: u32) -> usize {
    let mut encoder = encoder(codec, width, height, max_request_size);
    let mut updates = encoder.update(DisplayUpdate::Bitmap(incompressible_frame(width, height)));
    let mut largest = 0;
    while let Some(update) = updates.next().await {
        largest = largest.max(update.unwrap().data.len());
    }
    largest
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (TS_BITMAP_DATA's destination rectangle and height describe the same rows)
#[tokio::test]
async fn a_legacy_bitmap_tile_narrower_than_the_frame_keeps_all_its_rows() {
    let (width, height) = (256, 64);
    let mut encoder = encoder(Codec::Bitmap, width, height, 65_536);
    let frame = incompressible_frame(width, height);
    let mut updates = encoder.update(DisplayUpdate::Bitmap(frame.clone()));
    while let Some(update) = updates.next().await {
        update.unwrap();
    }

    // Change the second 64x64 tile only: its rows end short of the frame's stride.
    let stride = frame.stride.get();
    let mut data = frame.data.to_vec();
    for row in data.chunks_mut(stride) {
        for byte in &mut row[64 * 4..128 * 4] {
            *byte ^= 0xFF;
        }
    }
    let mut updates = encoder.update(DisplayUpdate::Bitmap(BitmapUpdate {
        data: data.into(),
        ..frame
    }));

    let mut rows = 0;
    while let Some(update) = updates.next().await {
        let update = update.unwrap();
        for bitmap in decode::<BitmapUpdateData<'_>>(&update.data).unwrap().rectangles {
            let rect = bitmap.rectangle;
            assert_eq!((rect.left, rect.right), (64, 127));
            assert_eq!(
                rect.bottom - rect.top + 1,
                bitmap.height,
                "{rect:?} is not {} rows",
                bitmap.height
            );
            rows += bitmap.height;
        }
    }
    assert_eq!(rows, 64);
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (a fragmented update larger than MaxRequestSize overflows the client's reassembly buffer)
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn no_encoded_update_exceeds_the_negotiated_request_size(
        codec in prop_oneof![
            Just(Codec::Bitmap),
            Just(Codec::Raw),
            Just(Codec::Qoi),
            Just(Codec::Qoiz),
            Just(Codec::RemoteFx),
        ],
        // The legacy bitmap encoder takes widths in multiples of 4. Strips wider than one
        // fragment's worth of pixels are split by width, and short ones are padded by RemoteFX.
        (width, height) in prop_oneof![
            ((1u16..=256).prop_map(|width| width * 4), 1u16..=256),
            ((1024u16..=2047).prop_map(|width| width * 4), 1u16..=4),
        ],
        max_request_size in 16_374u32..=262_144,
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let largest = runtime.block_on(largest_update(codec, width, height, max_request_size));
        prop_assert!(
            largest <= usize::try_from(max_request_size).unwrap(),
            "{largest}-byte update exceeds the {max_request_size}-byte request size"
        );
    }
}

/// A 96x96 color pointer: 36,864 bytes of color data, more than one fragment.
async fn color_pointer_update(max_request_size: u32) -> Option<ironrdp_server::ServerResult<usize>> {
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width: 96, height: 96 },
        CmdFlags::empty(),
        UpdateEncoderCodecs::new(),
        max_request_size,
        1,
        1,
        LargePointerSupportFlags::UP_TO_96X96_PIXELS,
    )
    .unwrap();
    let pointer = ColorPointer {
        cache_index: 0,
        width: 96,
        height: 96,
        hot_x: 0,
        hot_y: 0,
        and_mask: vec![],
        xor_mask: vec![0xAB; 96 * 96 * 4],
    };

    let update = encoder.update(DisplayUpdate::ColorPointer(pointer)).next().await;
    update.map(|update| update.map(|update| update.data.len()))
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (a fragmented update larger than MaxRequestSize overflows the client's reassembly buffer)
#[tokio::test]
async fn an_oversized_color_pointer_is_dropped() {
    let max_request_size = one_fragment();
    let update = color_pointer_update(max_request_size).await;
    assert!(
        update.is_none(),
        "a {:?}-byte pointer update was sent with a {max_request_size}-byte request size",
        update.map(|update| update.unwrap())
    );
}

// oracle: mutant pointer updates dropped regardless of their size
#[tokio::test]
async fn a_color_pointer_within_the_request_size_is_sent() {
    let update = color_pointer_update(65_536).await;
    assert!(update.is_some_and(|update| update.is_ok()), "the pointer was dropped");
}

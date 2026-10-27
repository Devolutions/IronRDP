use core::num::{NonZeroU16, NonZeroUsize};

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
#[case::client_smaller_than_server(8 * 1024 * 1024, Some(65_536), 65_536)]
#[case::client_larger_than_server(65_536, Some(8 * 1024 * 1024), 65_536)]
#[case::client_sends_zero(65_536, Some(0), 65_536)]
#[case::client_sends_no_capability_set(65_536, None, 65_536)]
fn negotiated_max_request_size_is_the_smaller_of_both(
    #[case] server: u32,
    #[case] client: Option<u32>,
    #[case] expected: u32,
) {
    let capabilities: Vec<_> = client.into_iter().map(multifragment_update).collect();
    assert_eq!(client_max_request_size(server, &capabilities), expected);
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
        // The legacy bitmap encoder takes widths in multiples of 4.
        width in (1u16..=256).prop_map(|width| width * 4),
        height in 1u16..=256,
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

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (a fragmented update larger than MaxRequestSize overflows the client's reassembly buffer)
#[rstest]
#[case::larger_than_the_request_size(None, false)]
#[case::within_the_request_size(Some(65_536), true)]
#[tokio::test]
async fn a_color_pointer_is_sent_only_within_the_request_size(
    #[case] max_request_size: Option<u32>,
    #[case] sent: bool,
) {
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width: 96, height: 96 },
        CmdFlags::empty(),
        UpdateEncoderCodecs::new(),
        max_request_size.unwrap_or_else(one_fragment),
        1,
        LargePointerSupportFlags::UP_TO_96X96_PIXELS,
    )
    .unwrap();
    // 36,864 bytes of color data, more than one fragment.
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

    match update {
        Some(update) => assert!(sent, "the pointer was sent: {} bytes", update.unwrap().data.len()),
        None => assert!(!sent, "the pointer was dropped"),
    }
}

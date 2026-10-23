use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_pdu::rdp::capability_sets::{
    CapabilitySet, CmdFlags, EntropyBits, LargePointerSupportFlags, MultifragmentUpdate,
};
use ironrdp_server::{
    BitmapUpdate, ColorPointer, DesktopSize, DisplayUpdate, PixelFormat, UpdateEncoder, UpdateEncoderCodecs,
    client_max_request_size,
};
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

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1 (an update sent as a single FASTPATH_FRAGMENT_SINGLE is never reassembled; the server's fragments carry 16,374 bytes)
#[test]
fn a_tiny_client_max_request_size_does_not_go_below_one_fragment() {
    assert_eq!(client_max_request_size(65_536, &[multifragment_update(1)]), 16_374);
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
            codecs.set_remotefx(Some((EntropyBits::Rlgr3, 1)));
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
#[case::raw_surface_rows_of_2048_pixels(Codec::Raw, 2048, 64, 65_536)]
#[case::qoi_surface_rows_of_2048_pixels(Codec::Qoi, 2048, 64, 65_536)]
#[case::qoiz_surface_rows_of_2048_pixels(Codec::Qoiz, 2048, 64, 65_536)]
#[case::remotefx_surface_rows_of_2048_pixels(Codec::RemoteFx, 2048, 64, 65_536)]
#[case::legacy_bitmap(Codec::Bitmap, 1440, 900, 65_536)]
#[case::legacy_bitmap_at_one_fragment(Codec::Bitmap, 1440, 900, 16_374)]
#[tokio::test]
async fn every_encoded_update_fits_the_negotiated_request_size(
    #[case] codec: Codec,
    #[case] width: u16,
    #[case] height: u16,
    #[case] max_request_size: u32,
) {
    let mut encoder = encoder(codec, width, height, max_request_size);
    let mut updates = encoder.update(DisplayUpdate::Bitmap(incompressible_frame(width, height)));
    while let Some(update) = updates.next().await {
        let len = update.unwrap().data.len();
        assert!(
            len <= usize::try_from(max_request_size).unwrap(),
            "{len}-byte update exceeds the {max_request_size}-byte request size"
        );
    }
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6 (a fragmented update larger than MaxRequestSize overflows the client's reassembly buffer)
#[tokio::test]
async fn a_color_pointer_larger_than_the_request_size_is_not_sent() {
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width: 96, height: 96 },
        CmdFlags::empty(),
        UpdateEncoderCodecs::new(),
        16_374,
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

    let update = encoder
        .update(DisplayUpdate::ColorPointer(pointer))
        .next()
        .await
        .unwrap();

    assert!(
        update.is_err(),
        "a 36,864-byte pointer must not reach a 16,374-byte buffer"
    );
}

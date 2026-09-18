use core::num::{NonZeroU16, NonZeroUsize};

use bytes::Bytes;
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::bench::encoder::{UpdateEncoder, UpdateEncoderCodecs};
use ironrdp_server::{BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat};

const EIGHT_MIB: u32 = 8 * 1024 * 1024;

/// Framing `set_surface` adds around the pixels of one uncompressed surface-bits update:
/// `TS_SURFCMD_SET_SURF_BITS` (cmdType 2 + destRect 8) + `TS_BITMAP_DATA_EX` (12).
const SURFACE_BITS_FRAMING: usize = 22;

fn uncompressed_encoder(width: u16, height: u16, max_request_size: u32) -> UpdateEncoder {
    // Surface bits allowed and no codec: the `NoneHandler` path, one surface command per tile.
    UpdateEncoder::new(
        DesktopSize { width, height },
        CmdFlags::SET_SURFACE_BITS,
        UpdateEncoderCodecs::new(),
        max_request_size,
        0,
        LargePointerSupportFlags::empty(),
    )
    .unwrap()
}

fn frame(width: u16, height: u16) -> DisplayUpdate {
    let stride = usize::from(width) * 4;
    DisplayUpdate::Bitmap(BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(height).unwrap(),
        format: PixelFormat::XRgb32,
        data: Bytes::from(vec![0; stride * usize::from(height)]),
        stride: NonZeroUsize::new(stride).unwrap(),
    })
}

/// Sizes of the updates the encoder emits for one frame: exactly what the client reassembles and
/// checks against `MultifragMaxRequestSize`.
async fn update_sizes(encoder: &mut UpdateEncoder, update: DisplayUpdate) -> Vec<usize> {
    let mut updates = encoder.update(update);
    let mut sizes = Vec::new();
    while let Some(fragmenter) = updates.next().await {
        sizes.push(fragmenter.unwrap().data.len());
    }
    sizes
}

fn strip_bytes(width: u16, rows: u16) -> usize {
    usize::from(width) * usize::from(rows) * 4 + SURFACE_BITS_FRAMING
}

#[tokio::test]
async fn a_width_that_divides_the_buffer_leaves_room_for_the_framing() {
    // 2048 * 4 divides 8 MiB exactly: 1024 rows filled the buffer with pixels alone and the
    // framing pushed the update past what the server advertised. One row fewer fits.
    let mut encoder = uncompressed_encoder(2048, 1080, EIGHT_MIB);
    let sizes = update_sizes(&mut encoder, frame(2048, 1080)).await;

    assert_eq!(sizes, vec![strip_bytes(2048, 1023), strip_bytes(2048, 57)]);
    assert!(sizes.iter().all(|&size| size <= usize::try_from(EIGHT_MIB).unwrap()));
}

#[tokio::test]
async fn tall_frames_split_into_strips_that_fit() {
    let mut encoder = uncompressed_encoder(4096, 2160, EIGHT_MIB);
    let sizes = update_sizes(&mut encoder, frame(4096, 2160)).await;

    let mut expected = vec![strip_bytes(4096, 511); 4];
    expected.push(strip_bytes(4096, 2160 - 4 * 511));
    assert_eq!(sizes, expected);
}

#[tokio::test]
async fn a_frame_that_fits_with_its_framing_is_sent_whole() {
    // The budget is inclusive: pixels plus framing equal to the buffer size is one update.
    let (width, height) = (640u16, 480u16);
    let cap = u32::try_from(strip_bytes(width, height)).unwrap();
    let mut encoder = uncompressed_encoder(width, height, cap);
    assert_eq!(
        update_sizes(&mut encoder, frame(width, height)).await,
        vec![strip_bytes(width, height)]
    );

    // One byte less and the frame has to be split.
    let mut encoder = uncompressed_encoder(width, height, cap - 1);
    let sizes = update_sizes(&mut encoder, frame(width, height)).await;
    assert!(1 < sizes.len());
    assert!(sizes.iter().all(|&size| size < usize::try_from(cap).unwrap()));
}

#[tokio::test]
async fn a_budget_below_one_row_splits_by_width_too() {
    // 2048 * 4 bytes per row do not fit a 1000-byte budget, so the strips are one row high and
    // are split along the width as well; every piece still fits with its framing.
    let mut encoder = uncompressed_encoder(2048, 8, 1000);
    let sizes = update_sizes(&mut encoder, frame(2048, 8)).await;

    assert!(1 < sizes.len());
    assert!(sizes.iter().all(|&size| size <= 1000));
}

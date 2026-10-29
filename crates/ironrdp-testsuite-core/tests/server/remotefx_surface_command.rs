use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_core::{ReadCursor, decode, decode_cursor};
use ironrdp_pdu::codecs::rfx::{Block, CodecChannel, SyncPdu};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, EntropyBits, LargePointerSupportFlags};
use ironrdp_pdu::surface_commands::SurfaceCommand;
use ironrdp_server::{
    BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, RemoteFxCodec, UpdateEncoder, UpdateEncoderCodecs,
};

const SIZE: DesktopSize = DesktopSize { width: 64, height: 64 };

#[derive(Debug, PartialEq)]
enum Command {
    Set,
    Stream,
}

fn bitmap() -> BitmapUpdate {
    BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(SIZE.width).unwrap(),
        height: NonZeroU16::new(SIZE.height).unwrap(),
        format: PixelFormat::BgrX32,
        data: vec![0x80; usize::from(SIZE.width) * usize::from(SIZE.height) * 4].into(),
        stride: NonZeroUsize::new(usize::from(SIZE.width) * 4).unwrap(),
    }
}

/// Encodes one bitmap and returns the surface command that carries it and its codec id.
async fn first_surface_command(surface_flags: CmdFlags, codec: RemoteFxCodec, codec_id: u8) -> (Command, u8) {
    let mut codecs = UpdateEncoderCodecs::new();
    codecs.set_remotefx_codec(Some((EntropyBits::Rlgr3, codec_id, codec)));
    let mut encoder = UpdateEncoder::new(
        SIZE,
        surface_flags,
        codecs,
        16 * 1024 * 1024,
        0,
        0,
        LargePointerSupportFlags::empty(),
    )
    .unwrap();
    let fragmenter = encoder
        .update(DisplayUpdate::Bitmap(bitmap()))
        .next()
        .await
        .unwrap()
        .unwrap();
    match decode::<SurfaceCommand<'_>>(&fragmenter.data).unwrap() {
        SurfaceCommand::SetSurfaceBits(pdu) => (Command::Set, pdu.extended_bitmap_data.codec_id),
        SurfaceCommand::StreamSurfaceBits(pdu) => (Command::Stream, pdu.extended_bitmap_data.codec_id),
        SurfaceCommand::FrameMarker(_) => panic!("expected a surface bits command, got a frame marker"),
    }
}

// oracle: spec MS-RDPRFX §3.1.8.3.1
#[tokio::test]
async fn remotefx_uses_stream_surface_bits_when_the_client_supports_it() {
    let flags = CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS;
    assert_eq!(
        first_surface_command(flags, RemoteFxCodec::RemoteFx, 3).await,
        (Command::Stream, 3)
    );
}

// oracle: spec MS-RDPBCGR §2.2.7.2.9 (STREAM_SURFACE_BITS flag), spec MS-RDPRFX §3.1.8.3.1 (RemoteFX surface command)
#[tokio::test]
async fn remotefx_uses_set_surface_bits_when_the_client_lacks_stream_surface_bits() {
    assert_eq!(
        first_surface_command(CmdFlags::SET_SURFACE_BITS, RemoteFxCodec::RemoteFx, 3).await,
        (Command::Set, 3)
    );
}

// oracle: reference Windows 10 mstsc 10.0.19041.4717 (ImageRemoteFX in Stream Surface Bits: "An internal error has occurred")
#[tokio::test]
async fn image_remotefx_uses_set_surface_bits() {
    let flags = CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS;
    assert_eq!(
        first_surface_command(flags, RemoteFxCodec::ImageRemoteFx, 5).await,
        (Command::Set, 5)
    );
}

// oracle: spec MS-RDPRFX §3.1.8.1.3 and §2.2.2.3.4
// A 1x1 first update still emits the initial Sync header block, and a later two-tile update grows the reused buffer.
#[tokio::test]
async fn remotefx_encodes_sub_tile_and_growing_updates() {
    let mut codecs = UpdateEncoderCodecs::new();
    codecs.set_remotefx_codec(Some((EntropyBits::Rlgr3, 3, RemoteFxCodec::RemoteFx)));
    let mut encoder = UpdateEncoder::new(
        DesktopSize { width: 128, height: 64 },
        CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS,
        codecs,
        16 * 1024 * 1024,
        0,
        0,
        LargePointerSupportFlags::empty(),
    )
    .unwrap();

    let small = BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(1).unwrap(),
        height: NonZeroU16::new(1).unwrap(),
        format: PixelFormat::BgrX32,
        data: vec![0x40; 4].into(),
        stride: NonZeroUsize::new(4).unwrap(),
    };
    let first = encoder
        .update(DisplayUpdate::Bitmap(small))
        .next()
        .await
        .unwrap()
        .unwrap();
    let SurfaceCommand::StreamSurfaceBits(first_pdu) = decode::<SurfaceCommand<'_>>(&first.data).unwrap() else {
        panic!("expected StreamSurfaceBits");
    };
    let mut cursor = ReadCursor::new(first_pdu.extended_bitmap_data.data);
    assert_eq!(decode_cursor::<Block<'_>>(&mut cursor).unwrap(), Block::Sync(SyncPdu));

    let large = BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(128).unwrap(),
        height: NonZeroU16::new(64).unwrap(),
        format: PixelFormat::BgrX32,
        data: vec![0x80; 128 * 64 * 4].into(),
        stride: NonZeroUsize::new(128 * 4).unwrap(),
    };
    let second = encoder
        .update(DisplayUpdate::Bitmap(large))
        .next()
        .await
        .unwrap()
        .unwrap();
    let SurfaceCommand::StreamSurfaceBits(second_pdu) = decode::<SurfaceCommand<'_>>(&second.data).unwrap() else {
        panic!("expected StreamSurfaceBits");
    };
    let mut cursor = ReadCursor::new(second_pdu.extended_bitmap_data.data);
    let mut tile_coords = Vec::new();
    while !cursor.is_empty() {
        let block = decode_cursor::<Block<'_>>(&mut cursor).unwrap();
        assert!(
            !matches!(
                block,
                Block::Sync(_)
                    | Block::Channels(_)
                    | Block::CodecVersions(_)
                    | Block::CodecChannel(CodecChannel::Context(_))
            ),
            "unexpected once-per-session header block in second update: {block:?}"
        );
        if let Block::CodecChannel(CodecChannel::TileSet(tile_set)) = block {
            tile_coords.extend(tile_set.tiles.iter().map(|tile| (tile.x, tile.y)));
        }
    }
    tile_coords.sort_unstable();
    assert_eq!(tile_coords, [(0, 0), (1, 0)]);
}

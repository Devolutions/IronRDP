use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_core::decode;
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

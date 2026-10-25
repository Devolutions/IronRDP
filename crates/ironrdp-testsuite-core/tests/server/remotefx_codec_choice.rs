use ironrdp_pdu::rdp::capability_sets::{
    CaptureFlags, Codec, CodecProperty, EntropyBits, RemoteFxContainer, RfxCaps, RfxCapset, RfxClientCapsContainer,
    RfxICap, RfxICapFlags,
};
use ironrdp_server::{RemoteFxCodec, pick_remotefx_codec};

fn client_codec(id: u8, codec: RemoteFxCodec) -> Codec {
    let caps = RfxClientCapsContainer {
        capture_flags: CaptureFlags::empty(),
        caps_data: RfxCaps(RfxCapset(vec![RfxICap {
            flags: RfxICapFlags::empty(),
            entropy_bits: EntropyBits::Rlgr3,
        }])),
    };
    let property = match codec {
        RemoteFxCodec::RemoteFx => CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(caps)),
        RemoteFxCodec::ImageRemoteFx => CodecProperty::ImageRemoteFx(RemoteFxContainer::ClientContainer(caps)),
    };
    Codec { id, property }
}

// oracle: reference Windows 10 mstsc 10.0.19041.4717 (ImageRemoteFX in Stream Surface Bits: "An internal error has occurred")
#[test]
fn image_remotefx_alone_is_picked_as_image_remotefx() {
    let codecs = [client_codec(5, RemoteFxCodec::ImageRemoteFx)];
    assert_eq!(
        pick_remotefx_codec(&codecs, true, true, None),
        Some((EntropyBits::Rlgr3, 5, RemoteFxCodec::ImageRemoteFx))
    );
}

// oracle: mutant the picked codec, and so the surface command, follows the client's codec order
#[test]
fn remotefx_wins_over_image_remotefx_in_either_order() {
    let image = client_codec(5, RemoteFxCodec::ImageRemoteFx);
    let remotefx = client_codec(3, RemoteFxCodec::RemoteFx);
    let expected = Some((EntropyBits::Rlgr3, 3, RemoteFxCodec::RemoteFx));
    assert_eq!(
        pick_remotefx_codec(&[image.clone(), remotefx.clone()], true, true, None),
        expected
    );
    assert_eq!(pick_remotefx_codec(&[remotefx, image], true, true, None), expected);
}

// oracle: mutant a codec the server does not enable is picked
#[test]
fn a_codec_the_server_disables_is_not_picked() {
    let codecs = [
        client_codec(5, RemoteFxCodec::ImageRemoteFx),
        client_codec(3, RemoteFxCodec::RemoteFx),
    ];
    assert_eq!(
        pick_remotefx_codec(&codecs, false, true, None),
        Some((EntropyBits::Rlgr3, 5, RemoteFxCodec::ImageRemoteFx))
    );
    assert_eq!(pick_remotefx_codec(&codecs, false, false, None), None);
}

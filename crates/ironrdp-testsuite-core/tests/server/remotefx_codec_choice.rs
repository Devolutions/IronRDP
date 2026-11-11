use ironrdp_pdu::rdp::capability_sets::{
    CaptureFlags, Codec, CodecProperty, EntropyBits, RemoteFxContainer, RfxCaps, RfxCapset, RfxClientCapsContainer,
    RfxICap, RfxICapFlags,
};
use ironrdp_server::{RemoteFxCodec, pick_remotefx_codec};
use proptest::collection::vec;
use proptest::prelude::*;

/// A codec of the client's list: a RemoteFX codec, or `None` for a codec of another kind.
fn listed_codec(id: u8, codec: Option<RemoteFxCodec>) -> Codec {
    let caps = RfxClientCapsContainer {
        capture_flags: CaptureFlags::empty(),
        caps_data: RfxCaps(RfxCapset(vec![RfxICap {
            flags: RfxICapFlags::empty(),
            entropy_bits: EntropyBits::Rlgr3,
        }])),
    };
    let property = match codec {
        Some(RemoteFxCodec::RemoteFx) => CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(caps)),
        Some(RemoteFxCodec::ImageRemoteFx) => CodecProperty::ImageRemoteFx(RemoteFxContainer::ClientContainer(caps)),
        None => CodecProperty::Ignore,
    };
    Codec { id, property }
}

/// Client codec lists with distinct codec ids, each with a shuffled copy.
fn codec_lists() -> impl Strategy<Value = (Vec<Codec>, Vec<Codec>)> {
    let kind = prop_oneof![
        Just(Some(RemoteFxCodec::RemoteFx)),
        Just(Some(RemoteFxCodec::ImageRemoteFx)),
        Just(None),
    ];
    vec(kind, 0..6).prop_flat_map(|kinds| {
        let codecs: Vec<Codec> = (1u8..).zip(kinds).map(|(id, codec)| listed_codec(id, codec)).collect();
        (Just(codecs.clone()), Just(codecs).prop_shuffle())
    })
}

fn lists(codecs: &[Codec], codec: RemoteFxCodec) -> bool {
    codecs.iter().any(|c| match &c.property {
        CodecProperty::RemoteFx(_) => codec == RemoteFxCodec::RemoteFx,
        CodecProperty::ImageRemoteFx(_) => codec == RemoteFxCodec::ImageRemoteFx,
        _ => false,
    })
}

fn picked_codec(codecs: &[Codec], remote_fx: bool, image_remote_fx: bool) -> Option<RemoteFxCodec> {
    pick_remotefx_codec(None, codecs, remote_fx, image_remote_fx).map(|(_, _, codec)| codec)
}

// oracle: mutant the picked codec, and so the surface command, follows the client's codec order, or a codec the server does not enable is picked
#[test]
fn remotefx_wins_over_image_remotefx_in_any_order() {
    proptest!(|(
        (codecs, shuffled) in codec_lists(),
        remote_fx in any::<bool>(),
        image_remote_fx in any::<bool>(),
    )| {
        let expected = if remote_fx && lists(&codecs, RemoteFxCodec::RemoteFx) {
            Some(RemoteFxCodec::RemoteFx)
        } else if image_remote_fx && lists(&codecs, RemoteFxCodec::ImageRemoteFx) {
            Some(RemoteFxCodec::ImageRemoteFx)
        } else {
            None
        };
        prop_assert_eq!(picked_codec(&codecs, remote_fx, image_remote_fx), expected);
        prop_assert_eq!(picked_codec(&shuffled, remote_fx, image_remote_fx), expected);
    });
}

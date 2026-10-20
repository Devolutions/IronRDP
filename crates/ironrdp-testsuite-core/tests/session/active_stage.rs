//! Regression tests for `composite_graphics_updates`, which applies EGFX compositor
//! deltas and returns both their exact regions and the union reported by `ActiveStage`.
//!
//! `ironrdp-session` builds with `[lib] test = false`, so inline `#[cfg(test)]`
//! modules there never run under `cargo test --workspace --locked`. These tests
//! live here instead so they actually execute in CI.

use std::sync::Arc;

use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_session::composite_graphics_updates;
use ironrdp_session::image::DecodedImage;

fn update(left: u16, top: u16, right: u16, bottom: u16) -> (ExclusiveRectangle, Vec<u8>) {
    let w = usize::from(right - left);
    let h = usize::from(bottom - top);
    (
        ExclusiveRectangle {
            left,
            top,
            right,
            bottom,
        },
        vec![0xFF; w * h * 4],
    )
}

/// Two disjoint deltas retain their exact regions alongside the union used by
/// full-frame fallback consumers.
#[test]
fn disjoint_deltas_collapse_to_their_union() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 200, 200);

    let (region, regions) =
        composite_graphics_updates(&mut image, [update(10, 10, 20, 20), update(100, 100, 150, 150)])
            .expect("both deltas are inside the image");
    let region = region.expect("two deltas produce a region");
    assert_eq!(regions.len(), 2);

    // Exclusive right/bottom of 20 and 150 become inclusive 19 and 149.
    assert_eq!(region.left, 10);
    assert_eq!(region.top, 10);
    assert_eq!(region.right, 149);
    assert_eq!(region.bottom, 149);
}

/// Every applied delta remains available to dirty-region consumers while fallback
/// consumers receive one union.
#[test]
fn many_deltas_yield_one_region() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 512, 512);
    let updates: Vec<_> = (0..64).map(|i| update(i, i, i + 8, i + 8)).collect();

    let (region, regions) = composite_graphics_updates(&mut image, updates).expect("all deltas are inside the image");
    let region = region.expect("64 deltas produce a region");
    assert_eq!(regions.len(), 64);

    assert_eq!(region.left, 0);
    assert_eq!(region.top, 0);
    assert_eq!(region.right, 70);
    assert_eq!(region.bottom, 70);
}

/// A drain that produced nothing must not surface an update at all, so a non-EGFX
/// session sees no change in behavior.
#[test]
fn no_deltas_yield_no_region() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);
    assert!(
        composite_graphics_updates(&mut image, [])
            .expect("an empty drain cannot fail")
            .0
            .is_none()
    );
}

/// One delta passes through as itself rather than being widened by the accumulator.
#[test]
fn a_single_delta_is_its_own_region() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);

    let (region, regions) =
        composite_graphics_updates(&mut image, [update(4, 8, 12, 16)]).expect("the delta is inside the image");
    let region = region.expect("one delta produces a region");
    assert_eq!(regions.as_slice(), core::slice::from_ref(&region));

    assert_eq!(region.left, 4);
    assert_eq!(region.top, 8);
    assert_eq!(region.right, 11);
    assert_eq!(region.bottom, 15);
}

/// A delta outside the image bounds must not be folded into either result.
/// This remains a defense against pre-reset deltas and future accounting mismatches.
#[test]
fn an_out_of_bounds_delta_is_dropped_not_unioned() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);

    let (region, regions) =
        composite_graphics_updates(&mut image, [update(20, 20, 30, 30), update(100, 100, 149, 149)])
            .expect("the in-bounds delta succeeds");
    let region = region.expect("the in-bounds delta produces a region");
    assert_eq!(regions.as_slice(), core::slice::from_ref(&region));

    assert_eq!(
        (region.left, region.top, region.right, region.bottom),
        (20, 20, 29, 29),
        "the out-of-bounds delta must not widen the region to include the origin"
    );
}

/// When every delta is out of bounds, the drain must report no region at all, not a
/// phantom 1x1 rectangle at the origin. Before this fix, `dirty` ended up
/// `Some((0, 0, 0, 0))` in this case, contradicting the invariant
/// `no_deltas_yield_no_region` asserts for an empty drain.
#[test]
fn every_delta_out_of_bounds_yields_no_region() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);

    let (region, regions) = composite_graphics_updates(&mut image, [update(100, 100, 149, 149)])
        .expect("an out-of-bounds delta does not error");

    assert!(
        region.is_none(),
        "a frame where nothing was painted must not report a region, got {region:?}"
    );
    assert!(regions.is_empty());
}

/// The bounds check is `>=`, not `>`: a delta whose exclusive right/bottom equals the
/// image width/height is exactly at the edge the exclusive-to-inclusive conversion
/// turns on (an exclusive bound of `width` becomes inclusive `width - 1`, which fits).
/// This pins that edge is accepted, not dropped as if it were one pixel out of bounds.
#[test]
fn a_delta_touching_the_image_edge_is_accepted() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);

    let (region, regions) = composite_graphics_updates(&mut image, [update(60, 60, 64, 64)])
        .expect("a delta flush with the image edge is inside the image");
    let region = region.expect("the delta produces a region");
    assert_eq!(regions.as_slice(), core::slice::from_ref(&region));

    assert_eq!(
        (region.left, region.top, region.right, region.bottom),
        (60, 60, 63, 63),
        "an exclusive bound equal to the image dimension must convert to the last valid pixel, not be dropped"
    );
}

#[test]
fn reset_graphics_preserves_software_pointer_state() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);
    image.move_pointer(1, 1).expect("set pointer position");
    image
        .update_pointer(Arc::new(DecodedPointer {
            width: 1,
            height: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            bitmap_data: vec![0xFF, 0, 0, 0xFF],
        }))
        .expect("show pointer");

    let allocation = image.data().as_ptr();
    image
        .reset_preserving_pointer(2, 2)
        .expect("reset same-size framebuffer with visible pointer");
    assert_eq!(image.data().as_ptr(), allocation);
    assert_eq!(&image.data()[12..15], &[0xFF, 0, 0]);

    image
        .reset_preserving_pointer(3, 3)
        .expect("resize with visible pointer");
    assert_eq!(&image.data()[16..19], &[0xFF, 0, 0]);

    image.hide_pointer().expect("hide pointer");
    image
        .reset_preserving_pointer(4, 4)
        .expect("resize with hidden pointer");
    assert_eq!(&image.data()[20..23], &[0, 0, 0]);
    image.show_pointer().expect("show retained pointer");
    assert_eq!(&image.data()[20..23], &[0xFF, 0, 0]);
}

#[test]
fn reset_graphics_clips_a_hotspot_cursor_to_one_pixel() {
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);
    image.move_pointer(0, 0).expect("set pointer position");
    image
        .update_pointer(Arc::new(DecodedPointer {
            width: 32,
            height: 32,
            hotspot_x: 16,
            hotspot_y: 16,
            bitmap_data: vec![0xFF; 32 * 32 * 4],
        }))
        .expect("show clipped pointer");

    image
        .reset_preserving_pointer(1, 1)
        .expect("clip pointer to one-pixel framebuffer");

    assert_eq!(image.data(), &[0xFF, 0xFF, 0xFF, 0]);
}

/// Same-size resets must reach the client resize queue even without a completed
/// graphics frame, otherwise scale-only Display Control requests time out.
#[test]
fn output_reset_is_reported_even_when_the_framebuffer_size_is_unchanged() {
    use core::any::TypeId;
    use std::borrow::Cow;

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DrdynvcClient;
    use ironrdp_dvc::pdu::{
        CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu, SoftSyncChannelList, SoftSyncRequestPdu,
        SoftSyncTunnelType,
    };
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::{GfxPdu, ResetGraphicsPdu};
    use ironrdp_graphics::zgfx::wrap_uncompressed;
    use ironrdp_pdu::Action;
    use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
    use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
    use ironrdp_pdu::x224::X224;
    use ironrdp_session::ActiveStageBuilder;
    use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};

    struct Handler;
    impl GraphicsPipelineHandler for Handler {}

    let mut drdynvc = DrdynvcClient::new().with_dynamic_channel(GraphicsPipelineClient::new(Box::new(Handler), None));
    drdynvc
        .process(
            &encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
                1,
                ironrdp_egfx::CHANNEL_NAME.to_owned(),
            )))
            .unwrap(),
        )
        .unwrap();
    let mut static_channels = StaticChannelSet::new();
    assert!(static_channels.insert(drdynvc).is_none());
    assert!(
        static_channels
            .attach_channel_id(TypeId::of::<DrdynvcClient>(), 1004)
            .is_none()
    );
    let mut stage = ActiveStageBuilder {
        static_channels,
        user_channel_id: 1001,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 800, 600);
    assert_eq!(stage.take_graphics_output_reset(), None);

    for tunneled in [false, true] {
        if tunneled {
            stage.enable_reliable_udp_dvc_tunnel().unwrap();
            let soft_sync = encode_vec(&DrdynvcServerPdu::SoftSyncRequest(SoftSyncRequestPdu::new(vec![
                SoftSyncChannelList::new(SoftSyncTunnelType::RELIABLE_UDP, vec![1]),
            ])))
            .unwrap();
            stage
                .get_svc_processor_mut::<DrdynvcClient>()
                .unwrap()
                .process(&soft_sync)
                .unwrap();
        }

        for (width, height) in [(800, 600), (1024, 768), (1024, 768)] {
            let reset = encode_vec(&GfxPdu::ResetGraphics(ResetGraphicsPdu {
                width: u32::from(width),
                height: u32::from(height),
                monitors: Vec::new(),
            }))
            .unwrap();
            let data = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
                1,
                wrap_uncompressed(&reset),
            ))))
            .unwrap();
            if tunneled {
                stage
                    .process_dvc_tunnel(&mut image, SoftSyncTunnelType::RELIABLE_UDP, &data)
                    .unwrap();
            } else {
                let mut user_data = encode_vec(&ChannelPduHeader {
                    length: u32::try_from(data.len()).unwrap(),
                    flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
                })
                .unwrap();
                user_data.extend(data);
                let frame = encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
                    initiator_id: 1001,
                    channel_id: 1004,
                    user_data: Cow::Owned(user_data),
                })))
                .unwrap();
                stage.process(&mut image, Action::X224, &frame).unwrap();
            }
            assert_eq!((image.width(), image.height()), (width, height));
            assert_eq!(stage.take_graphics_output_reset(), Some((width, height)));
            assert_eq!(stage.take_graphics_output_reset(), None);
        }
    }
}

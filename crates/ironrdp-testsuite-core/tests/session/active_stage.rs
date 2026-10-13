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

/// A pipeline run outside `ActiveStage` (say, on a thread of its own) is
/// composited by `composite_graphics_pipeline` the way `ActiveStage` composites
/// its own: the reset resizes the image, and the frame's fill lands in it.
#[test]
fn composite_graphics_pipeline_follows_a_reset_then_paints_the_frame() {
    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::{
        Color, CreateSurfacePdu, EndFramePdu, GfxPdu, MapSurfaceToOutputPdu, PixelFormat as GfxPixelFormat,
        ResetGraphicsPdu, SolidFillPdu, StartFramePdu, Timestamp,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;
    use ironrdp_session::composite_graphics_pipeline;

    struct Handler;
    impl GraphicsPipelineHandler for Handler {}

    let mut gfx = GraphicsPipelineClient::new(Box::new(Handler), None);
    for pdu in [
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: 64,
            height: 32,
            monitors: Vec::new(),
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: 64,
            height: 32,
            pixel_format: GfxPixelFormat::XRgb,
        }),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
        GfxPdu::StartFrame(StartFramePdu {
            timestamp: Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id: 1,
        }),
        GfxPdu::SolidFill(SolidFillPdu {
            surface_id: 1,
            fill_pixel: Color {
                b: 0x30,
                g: 0x20,
                r: 0x10,
                xa: 0xFF,
            },
            rectangles: vec![ExclusiveRectangle {
                left: 8,
                top: 4,
                right: 24,
                bottom: 12,
            }],
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 1 }),
    ] {
        gfx.process(0, &wrap_uncompressed(&encode_vec(&pdu).expect("encode PDU")))
            .expect("process PDU");
    }

    let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);
    let composited = composite_graphics_pipeline(&mut gfx, &mut image).expect("composite");
    assert!(composited.reset);
    assert_eq!((image.width(), image.height()), (64, 32));
    let region = composited.region.expect("the fill changed a region");
    assert!(
        region.left <= 8 && region.top <= 4 && region.right >= 23 && region.bottom >= 11,
        "{region:?} doesn't cover the fill"
    );
    let pixel = (5 * 64 + 10) * 4;
    assert_eq!(&image.data()[pixel..pixel + 3], &[0x10, 0x20, 0x30]);

    let again = composite_graphics_pipeline(&mut gfx, &mut image).expect("composite again");
    assert!(!again.reset && again.region.is_none(), "drained output came back");
}

/// `ActiveStage` composites a registered pipeline through
/// `composite_graphics_pipeline`, then widens the first update after a reset to
/// the whole image and records each region it painted as damage.
#[test]
fn active_stage_widens_after_a_reset_and_records_damage() {
    use std::borrow::Cow;

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DrdynvcClient;
    use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::{
        Color, CreateSurfacePdu, EndFramePdu, GfxPdu, MapSurfaceToOutputPdu, PixelFormat as GfxPixelFormat,
        ResetGraphicsPdu, SolidFillPdu, StartFramePdu, Timestamp,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;
    use ironrdp_pdu::Action;
    use ironrdp_pdu::mcs::SendDataIndication;
    use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
    use ironrdp_pdu::x224::X224;
    use ironrdp_session::{ActiveStageBuilder, ActiveStageOutput};
    use ironrdp_svc::StaticChannelSet;

    struct Handler;
    impl GraphicsPipelineHandler for Handler {}

    const DRDYNVC: u16 = 1004;
    const EGFX: u32 = 7;

    let mut channels = StaticChannelSet::new();
    channels.insert(DrdynvcClient::new().with_dynamic_channel(GraphicsPipelineClient::new(Box::new(Handler), None)));
    channels.attach_channel_id(core::any::TypeId::of::<DrdynvcClient>(), DRDYNVC);
    let mut active_stage = ActiveStageBuilder {
        static_channels: channels,
        user_channel_id: 1002,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();

    // A DRDYNVC PDU as the server sends it: one static channel chunk.
    let frame = |pdu: &DrdynvcServerPdu| {
        let pdu = encode_vec(pdu).expect("encode DRDYNVC PDU");
        let mut data = encode_vec(&ChannelPduHeader {
            length: u32::try_from(pdu.len()).expect("fits"),
            flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
        })
        .expect("encode channel header");
        data.extend_from_slice(&pdu);
        encode_vec(&X224(SendDataIndication {
            initiator_id: 1002,
            channel_id: DRDYNVC,
            user_data: Cow::Owned(data),
        }))
        .expect("encode X224")
    };
    let gfx = |pdus: &[GfxPdu]| {
        let mut bytes = Vec::new();
        for pdu in pdus {
            bytes.extend(encode_vec(pdu).expect("encode EGFX PDU"));
        }
        frame(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            EGFX,
            wrap_uncompressed(&bytes),
        ))))
    };
    let fill = |frame_id, rect: ExclusiveRectangle| {
        [
            GfxPdu::StartFrame(StartFramePdu {
                timestamp: Timestamp {
                    milliseconds: 0,
                    seconds: 0,
                    minutes: 0,
                    hours: 0,
                },
                frame_id,
            }),
            GfxPdu::SolidFill(SolidFillPdu {
                surface_id: 1,
                fill_pixel: Color {
                    b: 0x30,
                    g: 0x20,
                    r: 0x10,
                    xa: 0xFF,
                },
                rectangles: vec![rect],
            }),
            GfxPdu::EndFrame(EndFramePdu { frame_id }),
        ]
    };
    let graphics_updates = |outputs: Vec<ActiveStageOutput>| -> Vec<_> {
        outputs
            .into_iter()
            .filter_map(|output| match output {
                ActiveStageOutput::GraphicsUpdate(region) => Some(region),
                _ => None,
            })
            .collect()
    };

    let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);
    active_stage
        .process(
            &mut image,
            Action::X224,
            &frame(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
                EGFX,
                ironrdp_egfx::CHANNEL_NAME.to_owned(),
            ))),
            None,
        )
        .expect("open the EGFX channel");

    let mut first = vec![
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: 64,
            height: 32,
            monitors: Vec::new(),
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: 64,
            height: 32,
            pixel_format: GfxPixelFormat::XRgb,
        }),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
    ];
    first.extend(fill(
        1,
        ExclusiveRectangle {
            left: 8,
            top: 4,
            right: 24,
            bottom: 12,
        },
    ));
    let updates = graphics_updates(
        active_stage
            .process(&mut image, Action::X224, &gfx(&first), None)
            .expect("first frame"),
    );
    assert_eq!((image.width(), image.height()), (64, 32));
    let [update] = updates.as_slice() else {
        panic!("one graphics update, got {updates:?}");
    };
    assert_eq!(
        (update.left, update.top, update.right, update.bottom),
        (0, 0, 63, 31),
        "the update after a reset covers the whole image"
    );

    let second = fill(
        2,
        ExclusiveRectangle {
            left: 40,
            top: 20,
            right: 48,
            bottom: 28,
        },
    );
    let updates = graphics_updates(
        active_stage
            .process(&mut image, Action::X224, &gfx(&second), None)
            .expect("second frame"),
    );
    assert_eq!(updates.len(), 1, "one graphics update, got {updates:?}");
    let damage = active_stage.take_damage_regions();
    assert!(
        damage
            .iter()
            .any(|region| (region.left, region.top, region.right, region.bottom) == (40, 20, 47, 27)),
        "the fill is recorded as damage, got {damage:?}"
    );
}

/// A surface the server maps to a RemoteApp window is not part of the image:
/// `composite_graphics_pipeline` hands its pixels back as a window update, once,
/// so they don't pile up in the pipeline's memory budget.
#[test]
fn composite_graphics_pipeline_takes_the_window_updates() {
    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::{
        Color, CreateSurfacePdu, EndFramePdu, GfxPdu, MapSurfaceToWindowPdu, PixelFormat as GfxPixelFormat,
        ResetGraphicsPdu, SolidFillPdu, StartFramePdu, Timestamp,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;
    use ironrdp_session::composite_graphics_pipeline;

    struct Handler;
    impl GraphicsPipelineHandler for Handler {}

    let mut gfx = GraphicsPipelineClient::new(Box::new(Handler), None);
    for pdu in [
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: 64,
            height: 32,
            monitors: Vec::new(),
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: 8,
            height: 4,
            pixel_format: GfxPixelFormat::XRgb,
        }),
        GfxPdu::MapSurfaceToWindow(MapSurfaceToWindowPdu {
            surface_id: 1,
            window_id: 0x42,
            mapped_width: 4,
            mapped_height: 2,
        }),
        GfxPdu::StartFrame(StartFramePdu {
            timestamp: Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id: 1,
        }),
        GfxPdu::SolidFill(SolidFillPdu {
            surface_id: 1,
            fill_pixel: Color {
                b: 0x30,
                g: 0x20,
                r: 0x10,
                xa: 0xFF,
            },
            rectangles: vec![ExclusiveRectangle {
                left: 0,
                top: 0,
                right: 8,
                bottom: 4,
            }],
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 1 }),
    ] {
        gfx.process(0, &wrap_uncompressed(&encode_vec(&pdu).expect("encode PDU")))
            .expect("process PDU");
    }

    let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);
    let composited = composite_graphics_pipeline(&mut gfx, &mut image).expect("composite");
    assert!(composited.region.is_none(), "a window surface painted the image");
    // The mapping exposes its area too, so the fill may arrive as more than one update.
    assert!(!composited.window_updates.is_empty(), "no window update");
    for update in &composited.window_updates {
        assert_eq!(update.window_id, 0x42);
        // Only the mapped top-left 4x2 area of the surface reaches the window.
        assert_eq!((update.width, update.height), (4, 2));
        assert_eq!(update.data, [0x10, 0x20, 0x30, 0xFF].repeat(4 * 2));
    }

    let again = composite_graphics_pipeline(&mut gfx, &mut image).expect("composite again");
    assert!(again.window_updates.is_empty(), "drained window updates came back");
}

//! Regression tests for `composite_graphics_updates`, which applies EGFX compositor
//! deltas and returns both their exact regions and the union reported by `ActiveStage`,
//! and for the output reset that must precede compositing in `ActiveStage::process`.
//!
//! `ironrdp-session` builds with `[lib] test = false`, so inline `#[cfg(test)]`
//! modules there never run under `cargo test --workspace --locked`. These tests
//! live here instead so they actually execute in CI.

use core::any::TypeId;
use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_core::encode_vec;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::pdu::{
    CapabilitiesConfirmPdu, CapabilitiesV8Flags, CapabilitySet, Color, CreateSurfacePdu, EndFramePdu, GfxPdu,
    MapSurfaceToOutputPdu, PixelFormat as GfxPixelFormat, ResetGraphicsPdu, SolidFillPdu, StartFramePdu, Timestamp,
};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_graphics::zgfx::wrap_uncompressed;
use ironrdp_pdu::Action;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_session::composite_graphics_updates;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};

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

// ── EGFX ResetGraphics ───────────────────────────────────────────────────────

const USER_CHANNEL_ID: u16 = 1001;
const IO_CHANNEL_ID: u16 = 1003;
const DRDYNVC_CHANNEL_ID: u16 = 1004;
/// Any non-zero id works; the server picks this when it opens the EGFX channel.
const EGFX_DVC_ID: u32 = 7;

const OLD_WIDTH: u16 = 640;
const OLD_HEIGHT: u16 = 480;
const NEW_WIDTH: u16 = 1024;
const NEW_HEIGHT: u16 = 768;

/// The handler is only notified; the compositing under test is driven by the PDUs.
struct NoopEgfxHandler;

impl GraphicsPipelineHandler for NoopEgfxHandler {}

/// One complete static-channel chunk: the SVC layer dechunkifies before dispatching, so
/// a payload without FIRST|LAST is rejected as a fragment with no opening.
fn channel_chunk(data: &[u8]) -> Vec<u8> {
    let header = ChannelPduHeader {
        length: u32::try_from(data.len()).expect("chunk length fits u32"),
        flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
    };
    let mut chunk = encode_vec(&header).expect("encode channel header");
    chunk.extend_from_slice(data);
    chunk
}

/// Wrap EGFX PDUs the way a server does: concatenated, ZGFX-segmented, carried on a
/// DVC data PDU inside an MCS Send Data Indication for the drdynvc channel.
fn egfx_frame(pdus: &[GfxPdu]) -> Vec<u8> {
    let mut raw = Vec::new();
    for pdu in pdus {
        raw.extend_from_slice(&encode_vec(pdu).expect("encode EGFX PDU"));
    }

    let dvc = DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(EGFX_DVC_ID, wrap_uncompressed(&raw))));

    let indication = McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: DRDYNVC_CHANNEL_ID,
        user_data: Cow::Owned(channel_chunk(&encode_vec(&dvc).expect("encode DVC data"))),
    });

    encode_vec(&X224(indication)).expect("encode MCS indication")
}

/// An `ActiveStage` whose EGFX channel is open and past capability negotiation.
fn active_stage_with_active_egfx() -> ActiveStage {
    // Register by listener so the channel is reachable by type, which is how
    // `ActiveStage` looks the EGFX processor up.
    let mut drdynvc =
        DrdynvcClient::new().with_dynamic_channel(GraphicsPipelineClient::new(Box::new(NoopEgfxHandler), None));

    // Open the channel and get past capability negotiation before the stage exists, so
    // the frame under test carries nothing but the reset and the drawing after it.
    let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(
        EGFX_DVC_ID,
        ironrdp_egfx::CHANNEL_NAME.to_owned(),
    ));
    drdynvc
        .process(&encode_vec(&create).expect("encode create request"))
        .expect("open EGFX channel");

    let confirm = GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V8 {
        flags: CapabilitiesV8Flags::empty(),
    }));
    let raw = wrap_uncompressed(&encode_vec(&confirm).expect("encode caps confirm"));
    let dvc = DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(EGFX_DVC_ID, raw)));
    drdynvc
        .process(&encode_vec(&dvc).expect("encode DVC data"))
        .expect("confirm capabilities");

    let mut static_channels = StaticChannelSet::new();
    assert!(static_channels.insert(drdynvc).is_none());
    assert!(
        static_channels
            .attach_channel_id(TypeId::of::<DrdynvcClient>(), DRDYNVC_CHANNEL_ID)
            .is_none()
    );

    ActiveStageBuilder {
        static_channels,
        user_channel_id: USER_CHANNEL_ID,
        io_channel_id: IO_CHANNEL_ID,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
}

/// The image adopts the new output size before the deltas from the same payload are
/// composited into it.
///
/// A server changing resolution sends `ResetGraphics` and the drawing that repaints the
/// new desktop together, and it does not send those deltas again. The compositor already
/// holds them by the time the stage drains it, so an image still sized for the previous
/// output silently drops everything beyond the old bounds — see
/// `an_out_of_bounds_delta_is_dropped_not_unioned` for that half. The result on screen is
/// the previous desktop left behind under the new resolution: the tearing this guards.
///
/// The fill here lands entirely outside the old 640x480 image and inside the new
/// 1024x768 one, so it can only survive if the resize happened first.
#[test]
fn egfx_reset_resizes_the_image_before_compositing_the_same_payload() {
    let mut stage = active_stage_with_active_egfx();
    let mut image = DecodedImage::new(PixelFormat::RgbA32, OLD_WIDTH, OLD_HEIGHT);

    let fill = ExclusiveRectangle {
        left: 704,
        top: 512,
        right: 768,
        bottom: 576,
    };
    assert!(
        fill.left >= OLD_WIDTH && fill.top >= OLD_HEIGHT,
        "the fill has to start outside the old image for this test to mean anything"
    );

    let frame = egfx_frame(&[
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(NEW_WIDTH),
            height: u32::from(NEW_HEIGHT),
            monitors: Vec::new(),
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: NEW_WIDTH,
            height: NEW_HEIGHT,
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
                b: 0x10,
                g: 0x20,
                r: 0x30,
                xa: 0,
            },
            rectangles: vec![fill.clone()],
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 1 }),
    ]);

    let outputs = stage
        .process(&mut image, Action::X224, &frame)
        .expect("process EGFX reset frame");

    assert_eq!(
        (image.width(), image.height()),
        (NEW_WIDTH, NEW_HEIGHT),
        "the image must follow the output size the server just set"
    );

    let regions: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ActiveStageOutput::GraphicsUpdate(region) => Some(region),
            _ => None,
        })
        .collect();
    assert_eq!(
        regions.len(),
        1,
        "the fill from this payload must be reported, not dropped as out of bounds"
    );

    let region = regions[0];
    assert!(
        region.left <= fill.left
            && region.top <= fill.top
            && region.right >= fill.right - 1
            && region.bottom >= fill.bottom - 1,
        "reported region {region:?} must cover the fill {fill:?}"
    );

    // The pixels really landed, in the image's RGBA order.
    let stride = usize::from(image.width()) * 4;
    let offset = usize::from(fill.top) * stride + usize::from(fill.left) * 4;
    assert_eq!(
        &image.data()[offset..offset + 4],
        &[0x30, 0x20, 0x10, 0xFF],
        "the filled pixel must be present at its new-output coordinates"
    );
}

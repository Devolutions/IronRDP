//! Check fixture framing and reconstruction quality.
//! RemoteFX is lossy, so decoded pixels need not match the source exactly.

use ironrdp::core::{Decode as _, ReadCursor};
use ironrdp::pdu::codecs::rfx::{Block, CodecChannel, EntropyAlgorithm, OperatingMode};

use rfx_decode_fixtures::{Driver, Pattern, fixture, render};
use rstest::rstest;

/// Mean absolute error per channel between the decoded image and the source.
fn mean_abs_error(pattern: Pattern, width: u16, height: u16, algorithm: EntropyAlgorithm) -> f64 {
    let fx = fixture(pattern, width, height, algorithm);
    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);

    let source = render(pattern, usize::from(width), usize::from(height));
    let decoded = driver.image().data();
    mae_between(&source, decoded)
}

fn mae_between(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());

    let mut total = 0u64;
    let mut count = 0u64;

    for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 {
            total += u64::from(pa[c].abs_diff(pb[c]));
            count += 1;
        }
    }

    #[expect(clippy::as_conversions, reason = "casting u64 to f64")]
    {
        total as f64 / count as f64
    }
}

// oracle: mutant guard against blank or partially decoded fixture output.
// Baseline c4c27fdd, 640x384 RGB MAE at Quant::default() for both entropy modes:
// Desktop=2.073459, text=2.338346, gradient=9.708313, photo=10.887949, noise=8.870715.
// Ceilings round each measured MAE up to the next channel-value unit; these are regression gates, not codec accuracy guarantees.
#[rstest]
#[case::desktop(Pattern::Desktop, 3.0)]
#[case::text(Pattern::Text, 3.0)]
#[case::gradient(Pattern::Gradient, 10.0)]
#[case::photo(Pattern::Photo, 11.0)]
#[case::noise(Pattern::Noise, 9.0)]
fn every_pattern_round_trips(
    #[case] pattern: Pattern,
    #[case] max_mae: f64,
    #[values(EntropyAlgorithm::Rlgr1, EntropyAlgorithm::Rlgr3)] algorithm: EntropyAlgorithm,
) {
    let mae = mean_abs_error(pattern, 640, 384, algorithm);
    assert!(
        mae < max_mae,
        "{pattern:?}/{algorithm:?}: mean abs error {mae:.6} exceeds {max_mae}"
    );
}

// oracle: mutant guard against substituting another pattern's decoded image.
#[rstest]
#[case::desktop_text(Pattern::Desktop, Pattern::Text)]
#[case::desktop_gradient(Pattern::Desktop, Pattern::Gradient)]
#[case::desktop_photo(Pattern::Desktop, Pattern::Photo)]
#[case::desktop_noise(Pattern::Desktop, Pattern::Noise)]
#[case::text_desktop(Pattern::Text, Pattern::Desktop)]
#[case::text_gradient(Pattern::Text, Pattern::Gradient)]
#[case::text_photo(Pattern::Text, Pattern::Photo)]
#[case::text_noise(Pattern::Text, Pattern::Noise)]
#[case::gradient_desktop(Pattern::Gradient, Pattern::Desktop)]
#[case::gradient_text(Pattern::Gradient, Pattern::Text)]
#[case::gradient_photo(Pattern::Gradient, Pattern::Photo)]
#[case::gradient_noise(Pattern::Gradient, Pattern::Noise)]
#[case::photo_desktop(Pattern::Photo, Pattern::Desktop)]
#[case::photo_text(Pattern::Photo, Pattern::Text)]
#[case::photo_gradient(Pattern::Photo, Pattern::Gradient)]
#[case::photo_noise(Pattern::Photo, Pattern::Noise)]
#[case::noise_desktop(Pattern::Noise, Pattern::Desktop)]
#[case::noise_text(Pattern::Noise, Pattern::Text)]
#[case::noise_gradient(Pattern::Noise, Pattern::Gradient)]
#[case::noise_photo(Pattern::Noise, Pattern::Photo)]
fn decode_matches_its_own_source_better_than_an_unrelated_one(#[case] pattern: Pattern, #[case] other: Pattern) {
    let fx = fixture(pattern, 640, 384, EntropyAlgorithm::Rlgr3);
    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);
    let decoded = driver.image().data();

    let own = mae_between(decoded, &render(pattern, 640, 384));
    let cross = mae_between(decoded, &render(other, 640, 384));
    assert!(
        own < cross,
        "{pattern:?}: error against own source ({own:.2}) is not below error against {other:?} ({cross:.2})"
    );
}

// oracle: mutant guard against leaving partial edge tiles undecoded.
// Baseline c4c27fdd at Quant::default():
// Desktop RGB MAE is 4.982012 at 65x65 and 1.701910 at 1920x1080, rounded up as above.
#[rstest]
#[case::partial_row_and_column(65, 65, 4, 5.0)]
#[case::full_hd(1920, 1080, 30 * 17, 2.0)]
fn partial_tiles_are_covered(#[case] width: u16, #[case] height: u16, #[case] tiles: usize, #[case] max_mae: f64) {
    let fx = fixture(Pattern::Desktop, width, height, EntropyAlgorithm::Rlgr3);
    // oracle: 64x64 tiling geometry for the chosen dimensions.
    assert_eq!(fx.tiles, tiles);

    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);
    let source = render(Pattern::Desktop, usize::from(width), usize::from(height));
    let mae = mae_between(&source, driver.image().data());
    assert!(
        mae < max_mae,
        "{width}x{height}: mean abs error {mae:.6} exceeds {max_mae}"
    );
}

// oracle: spec MS-RDPRFX 2.2.2.2.4 requires video mode for headers sent once.
#[test]
fn steady_state_frames_use_video_mode() {
    let fx = fixture(Pattern::Text, 64, 64, EntropyAlgorithm::Rlgr3);
    let mut init = ReadCursor::new(&fx.init);
    assert!(matches!(Block::decode(&mut init).expect("sync"), Block::Sync(_)));
    let mut context_count = 0;
    for _ in 0..3 {
        if let Block::CodecChannel(CodecChannel::Context(context)) = Block::decode(&mut init).expect("header") {
            assert!(!context.flags.contains(OperatingMode::IMAGE_MODE));
            context_count += 1;
        }
    }
    assert_eq!(context_count, 1);
    assert!(matches!(
        Block::decode(&mut ReadCursor::new(&fx.frame)).expect("frame begin"),
        Block::CodecChannel(CodecChannel::FrameBegin(_))
    ));

    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);
    let first = driver.image().data().to_vec();
    driver.decode(&fx.frame);
    // oracle: property idempotent, replaying this fixed frame leaves the decoded pixels unchanged.
    assert_eq!(driver.image().data(), first);
}

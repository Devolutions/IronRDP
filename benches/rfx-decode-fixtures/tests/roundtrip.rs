//! Check fixture framing and reconstruction quality.
//! RemoteFX is lossy, so decoded pixels need not match the source exactly.

use ironrdp::core::{Decode as _, ReadCursor};
use ironrdp::pdu::codecs::rfx::{Block, CodecChannel, EntropyAlgorithm, OperatingMode};

use rfx_decode_fixtures::{Driver, Pattern, fixture, render};

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

#[test]
fn every_pattern_round_trips() {
    for pattern in Pattern::ALL {
        for algorithm in [EntropyAlgorithm::Rlgr1, EntropyAlgorithm::Rlgr3] {
            let mae = mean_abs_error(pattern, 640, 384, algorithm);

            // Allow quantization loss, but reject incomplete or incorrectly parsed frames.
            assert!(
                mae < 24.0,
                "{pattern:?}/{algorithm:?}: mean abs error {mae:.2} is too high, \
                 the round trip is broken"
            );
        }
    }
}

#[test]
fn decode_matches_its_own_source_better_than_an_unrelated_one() {
    // This checks the decoder is actually reconstructing `this` image:
    // the error against its own source must be far smaller than against
    // a different pattern of the same size.
    for pattern in Pattern::ALL {
        let fx = fixture(pattern, 640, 384, EntropyAlgorithm::Rlgr3);
        let mut driver = Driver::new(&fx);
        driver.decode(&fx.frame);
        let decoded = driver.image().data().to_vec();

        let own = mae_between(&decoded, &render(pattern, 640, 384));
        for other in Pattern::ALL {
            if other == pattern {
                continue;
            }
            let cross = mae_between(&decoded, &render(other, 640, 384));
            assert!(
                own * 3.0 < cross,
                "{pattern:?}: error against own source ({own:.2}) is not clearly \
                 below error against {other:?} ({cross:.2})"
            );
        }
    }
}

#[test]
fn partial_tiles_are_covered() {
    for (width, height, tiles) in [(65, 65, 4), (1920, 1080, 30 * 17)] {
        let fx = fixture(Pattern::Desktop, width, height, EntropyAlgorithm::Rlgr3);
        assert_eq!(fx.tiles, tiles);

        let mut driver = Driver::new(&fx);
        driver.decode(&fx.frame);
        let source = render(Pattern::Desktop, usize::from(width), usize::from(height));
        assert!(mae_between(&source, driver.image().data()) < 24.0);
    }
}

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
    assert_eq!(driver.image().data(), first);
}

//! This guards the fixtures by running them through a round trip encode/decode
//! and making sure that the decoded result is very similar to the original.
//! (RemoteFX is lossy, so we don't expect to get the original input back verbatim.)

#![allow(unused_crate_dependencies)] // Not every dependency is used by every target.

use ironrdp_pdu::codecs::rfx::EntropyAlgorithm;
use ironrdp_rfxbench::{Driver, Pattern, fixture, render};

/// Mean absolute error per channel between the decoded image and the source.
#[expect(clippy::as_conversions, reason = "u64 counters averaged as f64")]
fn mean_abs_error(pattern: Pattern, width: u16, height: u16, algorithm: EntropyAlgorithm) -> f64 {
    let fx = fixture(pattern, width, height, algorithm);
    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);

    let source = render(pattern, usize::from(width), usize::from(height));
    let decoded = driver.image().data();
    assert_eq!(source.len(), decoded.len());

    let mut total = 0u64;
    let mut count = 0u64;
    for (px_src, px_dec) in source.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        for c in 0..3 {
            total += u64::from(px_src[c].abs_diff(px_dec[c]));
            count += 1;
        }
    }

    total as f64 / count as f64
}

#[test]
fn every_pattern_round_trips() {
    for pattern in Pattern::ALL {
        for algorithm in [EntropyAlgorithm::Rlgr1, EntropyAlgorithm::Rlgr3] {
            let mae = mean_abs_error(pattern, 640, 384, algorithm);
            // RFX at the default quantization table is visibly lossy on
            // high-frequency content, but a decoder that stopped early or
            // mis-parsed would fail this check.
            assert!(
                mae < 24.0,
                "{pattern:?}/{algorithm:?}: mean abs error {mae:.2} is too high, \
                 the round trip is broken"
            );
            // And it must not be suspiciously perfect either: a fixture that
            // decoded into an all-zero image would also score well on a
            // near-black pattern.
            assert!(mae > 0.0, "{pattern:?}/{algorithm:?}: suspiciously exact round trip");
        }
    }
}

#[test]
fn decode_matches_its_own_source_far_better_than_an_unrelated_one() {
    // This checks the decode is actually reconstructing *this* image:
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

#[expect(clippy::as_conversions, reason = "u64 counters averaged as f64")]
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
    total as f64 / count as f64
}

#[test]
fn partial_tiles_are_covered() {
    // 1080 is not a multiple of 64: the bottom row of tiles is partial, which is
    // the case a real 1920x1080 session hits on every frame.
    let fx = fixture(Pattern::Desktop, 1920, 1080, EntropyAlgorithm::Rlgr3);
    assert_eq!(fx.tiles, 30 * 17);
    let mut driver = Driver::new(&fx);
    driver.decode(&fx.frame);
}

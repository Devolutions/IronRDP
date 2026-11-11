//! WebAssembly build of the RemoteFX decode workload, measured by `run.mjs`.
//!
//! Raw C-ABI exports let Node use the `WebAssembly` API without `wasm-bindgen` glue.
//! Stage inputs are restored in batches outside the timed section.

#![expect(clippy::missing_panics_doc, reason = "these are C ABI exports")]

use core::cell::RefCell;
use core::hint::black_box;
use ironrdp::pdu::codecs::rfx::EntropyAlgorithm;
use rfx_decode_fixtures::{Driver, Fixture, LL3_OFFSET, Pattern, StageFixture, TILE, fixture, stage_fixture};

/// Amortize the JS call and timer overhead without unbounded input allocation.
const STAGE_BATCH_SIZE: usize = 64;

/// Byte sampled by the YCbCr stage checksum, in addition to the first one.
/// It is the red channel of the pixel at a quarter of the tile.
const RGBA_SAMPLE: usize = TILE * TILE;

/// Maps the `rlgr3` flag of the C ABI exports to an entropy algorithm.
fn entropy_algorithm(rlgr3: u32) -> EntropyAlgorithm {
    if rlgr3 == 0 {
        EntropyAlgorithm::Rlgr1
    } else {
        EntropyAlgorithm::Rlgr3
    }
}

/// Decode stages, for [`bench_stage_setup`] and [`bench_stage_run`].
const STAGE_RLGR: u32 = 0;
const STAGE_SUBBAND: u32 = 1;
const STAGE_QUANTIZATION: u32 = 2;
const STAGE_DWT: u32 = 3;
const STAGE_YCBCR: u32 = 4;

thread_local! {
    static STAGES: RefCell<Option<StageState>> = const { RefCell::new(None) };
}

struct StageState {
    fixture: StageFixture,
    /// Input each stage starts from, restored per iteration.
    input: Vec<i16>,
    work: Vec<i16>,
    temp: Vec<i16>,
    planes: [Vec<i16>; 3],
    rgba: Vec<u8>,
    stage: u32,
}

struct State {
    fixture: Fixture,
    driver: Driver,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Builds the fixture and primes the decoder outside the timed section.
///
/// Returns the encoded frame size, or 0 for invalid arguments.
/// Invalid arguments clear the previous frame workload.
#[unsafe(no_mangle)]
pub extern "C" fn bench_setup(pattern: u32, width: u32, height: u32, rlgr3: u32) -> u32 {
    STATE.with_borrow_mut(|state| *state = None);
    let Some(pattern) = Pattern::from_u32(pattern) else {
        return 0;
    };
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return 0;
    };
    if width == 0 || height == 0 || i16::try_from(width).is_err() || i16::try_from(height).is_err() {
        return 0;
    }

    let algorithm = entropy_algorithm(rlgr3);

    let fx = fixture(pattern, width, height, algorithm);
    let driver = Driver::new(&fx);
    let wire_bytes = u32::try_from(fx.wire_bytes).expect("frame size fits u32");

    STATE.with_borrow_mut(|state| {
        *state = Some(State { fixture: fx, driver });
    });

    wire_bytes
}

/// Decodes one full frame and returns a sampled checksum.
#[unsafe(no_mangle)]
pub extern "C" fn bench_decode_frame() -> u32 {
    STATE.with_borrow_mut(|state| {
        let state = state.as_mut().expect("frame benchmark is not initialized");

        let State { fixture, driver } = state;
        let sum = driver.decode(&fixture.frame);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::as_conversions,
            reason = "intended truncation"
        )]
        {
            sum as u32
        }
    })
}

/// Whether this module was compiled with the wasm SIMD proposal enabled.
#[unsafe(no_mangle)]
pub extern "C" fn bench_has_simd128() -> u32 {
    u32::from(cfg!(target_feature = "simd128"))
}

/// Prepares `stage` for `pattern` with the input that stage sees in production.
///
/// Returns the number of stage executions per batch, or 0 for invalid arguments.
/// Invalid arguments clear the previous stage workload.
#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_setup(pattern: u32, stage: u32, rlgr3: u32) -> u32 {
    STAGES.with_borrow_mut(|state| *state = None);
    let Some(pattern) = Pattern::from_u32(pattern) else {
        return 0;
    };
    if STAGE_YCBCR < stage {
        return 0;
    }
    let algorithm = entropy_algorithm(rlgr3);

    let fixture = stage_fixture(pattern, algorithm);
    let planes = fixture.planes();

    // Replay the pipeline up to `stage` so its input is the real thing.
    let mut input = vec![0i16; TILE * TILE];
    let temp = vec![0i16; TILE * TILE];
    if STAGE_SUBBAND <= stage {
        ironrdp::graphics::rlgr::decode(algorithm, &fixture.encoded[0], &mut input).expect("rlgr");
    }
    if STAGE_QUANTIZATION <= stage {
        ironrdp::graphics::subband_reconstruction::decode(&mut input[LL3_OFFSET..]);
    }
    if STAGE_DWT <= stage {
        ironrdp::graphics::quantization::decode(&mut input, &fixture.quant);
    }

    STAGES.with_borrow_mut(|state| {
        *state = Some(StageState {
            fixture,
            work: input.repeat(STAGE_BATCH_SIZE),
            input,
            temp,
            planes,
            rgba: vec![0u8; TILE * TILE * 4 /* RGBA bytes per pixel */],
            stage,
        });
    });

    u32::try_from(STAGE_BATCH_SIZE).expect("stage batch size fits u32")
}

/// Restores a batch of stage inputs outside the timed section.
#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_prepare() {
    STAGES.with_borrow_mut(|state| {
        let state = state.as_mut().expect("stage benchmark is not initialized");
        if matches!(state.stage, STAGE_SUBBAND | STAGE_QUANTIZATION | STAGE_DWT) {
            for work in state.work.chunks_exact_mut(TILE * TILE) {
                work.copy_from_slice(&state.input);
            }
        }
    });
}

/// Runs a prepared batch without restoring inputs and returns a sampled checksum.
#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_run() -> u32 {
    STAGES.with_borrow_mut(|s| {
        let s = s.as_mut().expect("stage benchmark is not initialized");
        let mut sum = 0u32;

        for work in s.work.chunks_exact_mut(TILE * TILE) {
            match s.stage {
                STAGE_RLGR => {
                    ironrdp::graphics::rlgr::decode(s.fixture.algorithm, &s.fixture.encoded[0], work).expect("rlgr");
                }
                STAGE_SUBBAND => {
                    ironrdp::graphics::subband_reconstruction::decode(&mut work[LL3_OFFSET..]);
                }
                STAGE_QUANTIZATION => {
                    ironrdp::graphics::quantization::decode(work, &s.fixture.quant);
                }
                STAGE_DWT => {
                    ironrdp::graphics::dwt::decode(work, &mut s.temp);
                }
                STAGE_YCBCR => {
                    ironrdp::graphics::color_conversion::ycbcr_to_rgba(
                        ironrdp::graphics::color_conversion::YCbCrBuffer {
                            y: &s.planes[0],
                            cb: &s.planes[1],
                            cr: &s.planes[2],
                        },
                        &mut s.rgba,
                    )
                    .expect("ycbcr");
                    let rgba = black_box(&s.rgba);
                    sum = sum.wrapping_add(u32::from(rgba[0]) | (u32::from(rgba[RGBA_SAMPLE]) << 8));
                    continue;
                }
                _ => unreachable!("stage was validated during setup"),
            }

            let output = black_box(work);
            // Include the final LL3 coefficient, which subband reconstruction modifies.
            sum = sum.wrapping_add(
                u32::from(output[0].cast_unsigned()) ^ (u32::from(output[TILE * TILE - 1].cast_unsigned()) << 16),
            );
        }

        sum
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    // oracle: mutant guard against wrong frame selection, stale pixels, or incorrect checksum truncation in the C-ABI wrapper.
    #[rstest]
    fn frame_decode_matches_reference_and_repeats(#[values(0, 1)] rlgr3: u32) {
        let fx = fixture(Pattern::Text, 65, 65, entropy_algorithm(rlgr3));
        let mut reference = Driver::new(&fx);
        let initial = reference.image().data().to_vec();
        let checksum = reference.decode(&fx.frame);
        let expected_checksum = u32::try_from(checksum & u64::from(u32::MAX)).expect("masked checksum fits");
        assert_ne!(
            reference.image().data(),
            initial,
            "the full frame must differ from the one-tile init"
        );

        assert_eq!(
            bench_setup(1, 65, 65, rlgr3),
            u32::try_from(fx.wire_bytes).expect("frame size fits")
        );
        for _ in 0..3 {
            assert_eq!(bench_decode_frame(), expected_checksum);
            STATE.with_borrow(|state| {
                assert_eq!(
                    state.as_ref().expect("initialized").driver.image().data(),
                    reference.image().data()
                );
            });
        }
    }

    // oracle: mutant guard against wrong stage dispatch or failed input restoration in the C-ABI wrapper.
    #[rstest]
    #[case::rlgr(STAGE_RLGR)]
    #[case::subband(STAGE_SUBBAND)]
    #[case::quantization(STAGE_QUANTIZATION)]
    #[case::dwt(STAGE_DWT)]
    fn stage_batches_match_reference_and_repeat_after_prepare(#[case] stage: u32, #[values(0, 1)] rlgr3: u32) {
        let fixture = stage_fixture(Pattern::Text, entropy_algorithm(rlgr3));
        let mut expected = vec![0i16; TILE * TILE];
        let mut temp = vec![0i16; TILE * TILE];
        for current in STAGE_RLGR..=stage {
            match current {
                STAGE_RLGR => {
                    ironrdp::graphics::rlgr::decode(fixture.algorithm, &fixture.encoded[0], &mut expected)
                        .expect("rlgr");
                }
                STAGE_SUBBAND => ironrdp::graphics::subband_reconstruction::decode(&mut expected[LL3_OFFSET..]),
                STAGE_QUANTIZATION => ironrdp::graphics::quantization::decode(&mut expected, &fixture.quant),
                STAGE_DWT => ironrdp::graphics::dwt::decode(&mut expected, &mut temp),
                _ => unreachable!("only component stages are tested here"),
            }
        }

        assert_eq!(
            bench_stage_setup(1, stage, rlgr3),
            u32::try_from(STAGE_BATCH_SIZE).expect("batch size fits")
        );
        bench_stage_prepare();
        let checksum = bench_stage_run();
        for _ in 0..2 {
            STAGES.with_borrow(|state| {
                let state = state.as_ref().expect("initialized");
                for work in state.work.chunks_exact(TILE * TILE) {
                    assert_eq!(work, expected);
                }
            });
            bench_stage_prepare();
            assert_eq!(bench_stage_run(), checksum);
        }
    }

    // oracle: mutant guard against wrong color-plane selection or stale RGBA output in the C-ABI wrapper.
    #[rstest]
    fn ycbcr_batches_match_reference_and_repeat_after_prepare(#[values(0, 1)] rlgr3: u32) {
        let planes = stage_fixture(Pattern::Text, entropy_algorithm(rlgr3)).planes();
        let mut expected = vec![0u8; TILE * TILE * 4 /* RGBA bytes per pixel */];
        ironrdp::graphics::color_conversion::ycbcr_to_rgba(
            ironrdp::graphics::color_conversion::YCbCrBuffer {
                y: &planes[0],
                cb: &planes[1],
                cr: &planes[2],
            },
            &mut expected,
        )
        .expect("ycbcr");

        assert_eq!(
            bench_stage_setup(1, STAGE_YCBCR, rlgr3),
            u32::try_from(STAGE_BATCH_SIZE).expect("batch size fits")
        );
        bench_stage_prepare();
        let checksum = bench_stage_run();
        for _ in 0..2 {
            STAGES.with_borrow(|state| assert_eq!(state.as_ref().expect("initialized").rgba, expected));
            bench_stage_prepare();
            assert_eq!(bench_stage_run(), checksum);
        }
    }

    // oracle: mutant guard against retaining a stale frame workload after invalid setup.
    #[rstest]
    #[case::invalid_pattern(5, 64, 64)]
    #[case::empty_width(0, 0, 64)]
    #[case::empty_height(0, 64, 0)]
    #[case::width_exceeds_channel_limit(0, 32768, 64)]
    #[case::height_exceeds_u16(0, 64, 65536)]
    fn invalid_frame_setup_clears_previous_workload(#[case] pattern: u32, #[case] width: u32, #[case] height: u32) {
        assert_ne!(bench_setup(0, 64, 64, 1), 0);
        STATE.with_borrow(|state| assert!(state.is_some()));
        assert_eq!(bench_setup(pattern, width, height, 1), 0);
        STATE.with_borrow(|state| assert!(state.is_none()));
    }

    // oracle: mutant guard against retaining a stale stage workload after invalid setup.
    #[rstest]
    #[case::invalid_pattern(5, STAGE_RLGR)]
    #[case::invalid_stage(0, STAGE_YCBCR + 1)]
    fn invalid_stage_setup_clears_previous_workload(#[case] pattern: u32, #[case] stage: u32) {
        assert_ne!(bench_stage_setup(1, STAGE_RLGR, 1), 0);
        STAGES.with_borrow(|state| assert!(state.is_some()));
        assert_eq!(bench_stage_setup(pattern, stage, 1), 0);
        STAGES.with_borrow(|state| assert!(state.is_none()));
    }
}

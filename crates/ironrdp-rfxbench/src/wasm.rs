//! Raw C-ABI exports so the wasm build can be driven from Node with the plain
//! `WebAssembly` API.
//!
//! We're not using `wasm-bindgen` because we want to measure the codec, and a
//! bindgen boundary would add JS glue, copies and type conversions to every
//! call.

// This module is private, so `unreachable_pub` fires on every export -- but
// `no_mangle` puts them in the cdylib's symbol table regardless, which is the
// only way `wasm/run.mjs` can reach them.
#![expect(unreachable_pub, reason = "no_mangle exports are reachable from the cdylib")]

use core::cell::RefCell;

use ironrdp_pdu::codecs::rfx::EntropyAlgorithm;

use crate::{Driver, Fixture, Pattern, fixture};

struct State {
    fixture: Fixture,
    driver: Driver,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Builds the fixture and primes the decoder. Returns the encoded size of one
/// steady-state frame, or 0 if the arguments were out of range.
///
/// Encoding happens here rather than in the timed section, so `decode_frame`
/// measures decoding only.
#[unsafe(no_mangle)]
pub extern "C" fn bench_setup(pattern: u32, width: u32, height: u32, rlgr3: u32) -> u32 {
    let Some(pattern) = Pattern::from_u32(pattern) else {
        return 0;
    };
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return 0;
    };
    let algorithm = if rlgr3 == 0 {
        EntropyAlgorithm::Rlgr1
    } else {
        EntropyAlgorithm::Rlgr3
    };

    let fx = fixture(pattern, width, height, algorithm);
    let driver = Driver::new(&fx);
    let wire_bytes = u32::try_from(fx.wire_bytes).unwrap_or(u32::MAX);

    STATE.with_borrow_mut(|state| {
        *state = Some(State { fixture: fx, driver });
    });

    wire_bytes
}

/// Decodes one full frame. Returns a checksum to prevent it from being
/// optimized out.
#[unsafe(no_mangle)]
pub extern "C" fn bench_decode_frame() -> u32 {
    STATE.with_borrow_mut(|state| {
        let Some(state) = state.as_mut() else {
            return 0;
        };
        let frame = core::mem::take(&mut state.fixture.frame);
        let sum = state.driver.decode(&frame);
        state.fixture.frame = frame;
        #[expect(clippy::cast_possible_truncation, reason = "checksum, any 32 bits will do")]
        let sum = sum as u32;
        sum
    })
}

/// Number of tiles in the benchmarked frame.
#[unsafe(no_mangle)]
pub extern "C" fn bench_tiles() -> u32 {
    STATE.with_borrow(|state| {
        state
            .as_ref()
            .and_then(|s| u32::try_from(s.fixture.tiles).ok())
            .unwrap_or(0)
    })
}

/// Whether this module was compiled with the wasm SIMD proposal enabled.
#[unsafe(no_mangle)]
pub extern "C" fn bench_has_simd128() -> u32 {
    u32::from(cfg!(target_feature = "simd128"))
}

/// Decode stages, for [`bench_stage`].
const STAGE_RLGR: u32 = 0;
const STAGE_SUBBAND: u32 = 1;
const STAGE_QUANTIZATION: u32 = 2;
const STAGE_DWT: u32 = 3;
const STAGE_YCBCR: u32 = 4;

struct StageState {
    fixture: crate::StageFixture,
    /// Input each stage starts from, restored per iteration.
    input: Vec<i16>,
    work: Vec<i16>,
    temp: Vec<i16>,
    planes: [Vec<i16>; 3],
    rgba: Vec<u8>,
    stage: u32,
}

thread_local! {
    static STAGES: RefCell<Option<StageState>> = const { RefCell::new(None) };
}

/// Prepares `stage` for `pattern`, seeded with the buffer that stage sees in
/// production. Returns 1 on success, 0 if the arguments were out of range.
#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_setup(pattern: u32, stage: u32, rlgr3: u32) -> u32 {
    let Some(pattern) = Pattern::from_u32(pattern) else {
        return 0;
    };
    if stage > STAGE_YCBCR {
        return 0;
    }
    let algorithm = if rlgr3 == 0 {
        EntropyAlgorithm::Rlgr1
    } else {
        EntropyAlgorithm::Rlgr3
    };

    let fixture = crate::stage_fixture(pattern, algorithm);
    let planes = fixture.planes();

    // Replay the pipeline up to `stage` so its input is the real thing.
    let mut input = vec![0i16; crate::TILE * crate::TILE];
    let mut temp = vec![0i16; crate::TILE * crate::TILE];
    if stage >= STAGE_SUBBAND {
        ironrdp_graphics::rlgr::decode(algorithm, &fixture.encoded[0], &mut input).expect("rlgr");
    }
    if stage >= STAGE_QUANTIZATION {
        ironrdp_graphics::subband_reconstruction::decode(&mut input[4032..]);
    }
    if stage >= STAGE_DWT {
        ironrdp_graphics::quantization::decode(&mut input, &fixture.quant);
    }

    STAGES.with_borrow_mut(|state| {
        *state = Some(StageState {
            fixture,
            work: input.clone(),
            input,
            temp: core::mem::take(&mut temp),
            planes,
            rgba: vec![0u8; crate::TILE * crate::TILE * 4],
            stage,
        });
    });

    1
}

/// Runs the prepared stage once. Returns a checksum so the call cannot be
/// optimized away.
#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_run() -> u32 {
    STAGES.with_borrow_mut(|s| {
        let Some(s) = s.as_mut() else {
            return 0;
        };

        match s.stage {
            STAGE_RLGR => {
                ironrdp_graphics::rlgr::decode(s.fixture.algorithm, &s.fixture.encoded[0], &mut s.work).expect("rlgr");
            }
            STAGE_SUBBAND => {
                s.work.copy_from_slice(&s.input);
                ironrdp_graphics::subband_reconstruction::decode(&mut s.work[4032..]);
            }
            STAGE_QUANTIZATION => {
                s.work.copy_from_slice(&s.input);
                ironrdp_graphics::quantization::decode(&mut s.work, &s.fixture.quant);
            }
            STAGE_DWT => {
                s.work.copy_from_slice(&s.input);
                ironrdp_graphics::dwt::decode(&mut s.work, &mut s.temp);
            }
            STAGE_YCBCR => {
                ironrdp_graphics::color_conversion::ycbcr_to_rgba(
                    ironrdp_graphics::color_conversion::YCbCrBuffer {
                        y: &s.planes[0],
                        cb: &s.planes[1],
                        cr: &s.planes[2],
                    },
                    &mut s.rgba,
                )
                .expect("ycbcr");
                return u32::from(s.rgba[0]) | (u32::from(s.rgba[4096]) << 8);
            }
            _ => return 0,
        }

        #[expect(clippy::cast_sign_loss, reason = "checksum, any bit pattern will do")]
        let sum = s.work[0] as u32 ^ ((s.work[2048] as u32) << 16);
        sum
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn bench_stage_restore_only() -> u32 {
    STAGES.with_borrow_mut(|s| {
        let Some(s) = s.as_mut() else {
            return 0;
        };
        s.work.copy_from_slice(&s.input);
        #[expect(clippy::cast_sign_loss, reason = "checksum, any bit pattern will do")]
        let sum = s.work[0] as u32;
        sum
    })
}

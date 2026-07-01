//! Regression lock for the progressive colour descale: proves the RFX
//! progressive DWT is unity-gain, so the `>> 5` descale in `reconstruct_to_rgba`
//! compensates the fixed-point headroom retained by the base dequant
//! (`<< (quant - 1)`), not any scale introduced by the transform.
//! Run: cargo test -p ironrdp-graphics --test dwt_gain -- --nocapture
//!
//! Reasoning: the wire carries forward-DWT coefficients. If a flat pixel tile of
//! value P forward-transforms to an LL/DC coefficient of ~P, wire coeffs are at
//! pixel scale and the inverse DWT must return pixel scale (no descale needed).
//! If it transformed to ~32*P, the transform would be 2^5 non-normalized. It
//! does not: the ~32x seen on real streams comes from dequant headroom.

// This integration-test binary only exercises `ironrdp_graphics`; the crate's
// other dev-dependencies are for its unit tests.
#![allow(unused_crate_dependencies)]

use ironrdp_graphics::dwt;

const N: usize = 4096; // 64x64

#[test]
fn forward_dc_gain_and_roundtrip() {
    let p: i16 = 100;

    // Flat tile, all pixels = P.
    let mut buf = [p; N];
    let mut tmp = [0i16; N];
    dwt::encode(&mut buf, &mut tmp);

    let (min, max) = buf
        .iter()
        .fold((i16::MAX, i16::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let peak = buf.iter().map(|c| c.unsigned_abs()).max().unwrap_or(0);
    eprintln!("flat P={p}: full coeff range after forward DWT = [{min}, {max}]  peak|coeff|={peak}");

    // Round-trip must be identity.
    let mut rt = [p; N];
    let mut tmp2 = [0i16; N];
    dwt::encode(&mut rt, &mut tmp2);
    dwt::decode(&mut rt, &mut tmp2);
    let (rmin, rmax) = rt
        .iter()
        .fold((i16::MAX, i16::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    eprintln!("round-trip flat P={p}: recovered range = [{rmin}, {rmax}] (expect ~{p})");

    // Inverse-only gain: feed a pure DC coefficient and see the spatial amplitude.
    let mut dc_only = [0i16; N];
    dc_only[0] = 100;
    let mut tmp3 = [0i16; N];
    dwt::decode(&mut dc_only, &mut tmp3);
    let (imin, imax) = dc_only
        .iter()
        .fold((i16::MAX, i16::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    eprintln!("inverse-only DC=100: spatial output range = [{imin}, {imax}]");

    // Guard: unity-gain. No forward coefficient exceeds the input magnitude P
    // (a 2^5 transform would push the peak to ~32*P = ~3200), and the round-trip
    // is identity within integer-lifting rounding.
    assert!(
        i32::from(peak) <= i32::from(p) + 2,
        "DWT is not unity-gain: peak|coeff|={peak} for P={p}"
    );
    assert!((i32::from(rmin) - i32::from(p)).abs() <= 2, "round-trip low drift");
    assert!((i32::from(rmax) - i32::from(p)).abs() <= 2, "round-trip high drift");
}

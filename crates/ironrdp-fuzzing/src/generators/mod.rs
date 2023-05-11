//! Test case generators.
//!
//! Test case generators take raw, unstructured input from a fuzzer
//! (e.g. libFuzzer) and translate that into a structured test case (e.g. a
//! valid RDP PDU).
//!
//! These are generally implementations of the `Arbitrary` trait, or some
//! wrapper over an external tool, such that the wrapper implements the
//! `Arbitrary` trait for the wrapped external tool.

#[derive(Arbitrary, Debug)]
pub struct BitmapInput<'a> {
    pub src: &'a [u8],
    pub width: u8,
    pub height: u8,
}

#![allow(clippy::arithmetic_side_effects)] // FIXME: remove
#![allow(clippy::cast_lossless)] // FIXME: remove
#![allow(clippy::cast_possible_truncation)] // FIXME: remove
#![allow(clippy::cast_possible_wrap)] // FIXME: remove
#![allow(clippy::cast_sign_loss)] // FIXME: remove

pub mod color_conversion;
pub mod dwt;
pub mod image_processing;
pub mod pointer;
pub mod quantization;
pub mod rdp6;
pub mod rectangle_processing;
pub mod rle;
pub mod rlgr;
pub mod subband_reconstruction;
pub mod zgfx;

mod utils;

pub fn rfx_encode_component(
    input: &mut [i16],
    output: &mut [u8],
    quant: &ironrdp_pdu::codecs::rfx::Quant,
    mode: ironrdp_pdu::codecs::rfx::EntropyAlgorithm,
) -> Result<usize, rlgr::RlgrError> {
    assert_eq!(input.len(), 64 * 64);

    let mut temp = [0; 64 * 64]; // size = 8k, too big?

    dwt::encode(input, temp.as_mut_slice());
    quantization::encode(input, quant);
    subband_reconstruction::encode(&mut input[4032..]);
    rlgr::encode(mode, input, output)
}

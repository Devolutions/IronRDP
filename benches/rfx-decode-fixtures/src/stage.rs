use ironrdp::pdu::codecs::rfx::{EntropyAlgorithm, Quant};

use super::fixture::encode_tile;
use super::pattern::{Pattern, render};
use super::{Image, LL3_OFFSET, TILE, TILE_SIZE_BYTES};

/// Entropy-coded components of one [2.2.2.3.4.1] TS_RFX_TILE for isolated stage measurements.
///
/// [2.2.2.3.4.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/89e669ed-b6dd-4591-a267-73a72bc6d84e
pub struct StageFixture {
    pub quant: Quant,
    pub algorithm: EntropyAlgorithm,
    /// Y, Cb and Cr entropy-coded planes, exactly as they arrive on the wire.
    pub encoded: [Vec<u8>; 3],
}

impl StageFixture {
    /// Runs the entropy + subband + dequant + DWT chain, returning the three
    /// spatial-domain planes that `ycbcr_to_rgba` consumes.
    pub fn planes(&self) -> [Vec<i16>; 3] {
        let mut temp = vec![0i16; TILE * TILE];
        core::array::from_fn(|i| {
            let mut out = vec![0i16; TILE * TILE];
            ironrdp::graphics::rlgr::decode(self.algorithm, &self.encoded[i], &mut out).expect("RLGR decode");
            ironrdp::graphics::subband_reconstruction::decode(&mut out[LL3_OFFSET..]);
            ironrdp::graphics::quantization::decode(&mut out, &self.quant);
            ironrdp::graphics::dwt::decode(&mut out, &mut temp);
            out
        })
    }
}

/// Encodes one representative tile of `pattern` and returns its three planes.
pub fn stage_fixture(pattern: Pattern, algorithm: EntropyAlgorithm) -> StageFixture {
    // A 1920x1080 screen with the tile taken from the middle of the image, where
    // every pattern has content.
    const W: usize = 1920;
    const H: usize = 1080;
    let image = render(pattern, W, H);
    let quant = Quant::default();

    let mut buf = vec![0u8; TILE_SIZE_BYTES];

    let image = Image {
        width: W,
        height: H,
        data: &image,
    };
    let tile = encode_tile(&image, 8, 6, &quant, algorithm, &mut buf);
    let encoded = [tile.y_data.to_vec(), tile.cb_data.to_vec(), tile.cr_data.to_vec()];

    StageFixture {
        quant,
        algorithm,
        encoded,
    }
}

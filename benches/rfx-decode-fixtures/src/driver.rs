use core::hint::black_box;

use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::rfx::DecodingContext;

use super::BPP;
use super::fixture::{FORMAT, Fixture};

/// 64-bit FNV-1a prime, used to mix the sampled bytes into the checksum.
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// Distance in rows between sampled pixels.
const SAMPLE_ROW_STEP: usize = 16;

/// Holds the decoder state across frames, as a live session does.
pub struct Driver {
    context: DecodingContext,
    image: DecodedImage,
    destination: InclusiveRectangle,
}

impl Driver {
    /// Builds a driver and primes it with the `init` stream, so it is ready to decode the steady-state frame.
    pub fn new(fixture: &Fixture) -> Self {
        let mut this = Self {
            context: DecodingContext::new(),
            image: DecodedImage::new(FORMAT, fixture.width, fixture.height),
            destination: InclusiveRectangle {
                left: 0,
                top: 0,
                right: fixture.width - 1,
                bottom: fixture.height - 1,
            },
        };
        this.decode(&fixture.init);
        this
    }

    /// Decodes one frame and returns a checksum of the touched region.
    pub fn decode(&mut self, frame: &[u8]) -> u64 {
        let mut cursor = ironrdp::core::ReadCursor::new(frame);
        let (_frame_id, rect) = self
            .context
            .decode(&mut self.image, &self.destination, &mut cursor)
            .expect("decode frame");

        // Keep all pixel writes observable without hashing the entire image.
        let data = black_box(self.image.data());
        let stride = self.image.stride();
        let x_off = usize::from(rect.left) * BPP;

        let mut sum = u64::from(rect.left) ^ (u64::from(rect.bottom) << 16);
        for row in (usize::from(rect.top)..=usize::from(rect.bottom)).step_by(SAMPLE_ROW_STEP) {
            // INVARIANT: rect is within the image bounds.
            sum = sum.wrapping_mul(FNV_PRIME) ^ u64::from(data[row * stride + x_off]);
        }
        sum
    }

    /// Returns the driver's decoded image.
    pub fn image(&self) -> &DecodedImage {
        &self.image
    }
}

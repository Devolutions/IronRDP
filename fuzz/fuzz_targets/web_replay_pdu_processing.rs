#![no_main]

use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use ironrdp_web_replay::{PduSource, ReplayProcessor, ReplayProcessorConfig};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    for source in [PduSource::Client, PduSource::Server] {
        let mut processor = ReplayProcessor::new(&ReplayProcessorConfig::default());
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);
        let _ = processor.process_pdu(&mut image, source, data);
    }
});

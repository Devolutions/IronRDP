mod codecs;
mod fast_path;
mod x224;

use std::{
    io,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use ironrdp::{
    codecs::rfx::image_processing::PixelFormat, fast_path::FastPathError, PduParsing, RdpPdu,
};
use log::warn;

use crate::{
    connection_sequence::DesktopSizes,
    transport::{Decoder, RdpTransport},
    utils, Config, RdpError, RdpResult, StaticChannels,
};

const DESTINATION_PIXEL_FORMAT: PixelFormat = PixelFormat::RgbA32;

pub fn process_active_stage(
    mut stream: impl io::BufRead + io::Write,
    config: &Config,
    static_channels: StaticChannels,
    global_channel_id: u16,
    initiator_id: u16,
    desktop_sizes: DesktopSizes,
) -> RdpResult<()> {
    let decoded_image = Arc::new(Mutex::new(DecodedImage::new(
        u32::from(desktop_sizes.width),
        u32::from(desktop_sizes.height),
        DESTINATION_PIXEL_FORMAT,
        Some(config.images_path.clone()),
    )));
    let mut x224_processor = x224::Processor::new(utils::swap_hashmap_kv(static_channels));
    let mut fast_path_processor = fast_path::ProcessorBuilder {
        decoded_image,
        global_channel_id,
        initiator_id,
    }
    .build();
    let mut rdp_transport = RdpTransport::default();

    loop {
        let mut input_buffer = stream.fill_buf()?;
        let input_buffer_length = input_buffer.len();
        match rdp_transport.decode(&mut input_buffer) {
            Ok(RdpPdu::X224(data)) if input_buffer.len() >= data.data_length => {
                stream.consume(data.buffer_length());

                if let Err(error) = x224_processor.process(&mut stream, data) {
                    match error {
                        RdpError::UnexpectedDisconnection(message) => {
                            warn!("User-Initiated disconnection on Server: {}", message);
                            break;
                        }
                        RdpError::UnexpectedChannel(channel_id) => {
                            warn!("Got message on a channel with {} ID", channel_id);
                            break;
                        }
                        err => {
                            return Err(err);
                        }
                    }
                }
            }
            Ok(RdpPdu::FastPath(header)) if input_buffer.len() >= header.data_length => {
                // skip header bytes in such way because here is possible
                // that data length was written in the not right way,
                // so we should skip only what has been actually read
                let bytes_read = input_buffer_length - input_buffer.len();
                stream.consume(bytes_read);

                fast_path_processor.process(&header, &mut stream)?;
            }
            Err(RdpError::FastPathError(FastPathError::NullLength { bytes_read })) => {
                warn!("Received null-length Fast-Path packet, dropping it");
                stream.consume(bytes_read);
            }
            Ok(_) => {
                warn!("Received not complete packet, waiting for remaining data");
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

pub struct DecodedImage {
    width: u32,
    height: u32,
    data: Vec<u8>,
    images_path: Option<String>,
    frame_counter: u32,
}

impl DecodedImage {
    fn new(
        width: u32,
        height: u32,
        pixel_format: PixelFormat,
        images_path: Option<String>,
    ) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width * height * u32::from(pixel_format.bytes_per_pixel())) as usize],
            images_path,
            frame_counter: 0,
        }
    }

    fn get_mut(&mut self) -> &mut [u8] {
        self.data.as_mut_slice()
    }

    fn save(&mut self) -> image::ImageResult<()> {
        if let Some(images_path) = self.images_path.as_ref() {
            let image_buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                self.width,
                self.height,
                self.data.as_slice(),
            )
            .expect("Container must be large enough");

            image_buffer.save_with_format(
                format!("{}/update#{:010}.png", images_path, self.frame_counter),
                image::ImageFormat::Png,
            )?;
            self.frame_counter += 1;
        }

        Ok(())
    }
}

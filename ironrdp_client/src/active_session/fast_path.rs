use std::io;
use std::sync::{Arc, Mutex};

use ironrdp::codecs::rfx::FrameAcknowledgePdu;
use ironrdp::fast_path::{FastPathError, FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use ironrdp::{PduBufferParsing, ShareDataPdu};
use log::{debug, info, warn};
use num_traits::FromPrimitive;

use super::codecs::rfx;
use super::{DecodedImage, DESTINATION_PIXEL_FORMAT};
use crate::transport::{
    DataTransport, Encoder, McsTransport, SendDataContextTransport, ShareControlHeaderTransport,
    ShareDataHeaderTransport,
};
use crate::utils::CodecId;
use crate::RdpError;

pub struct Processor {
    complete_data: CompleteData,
    rfx_handler: rfx::DecodingContext,
    decoded_image: Arc<Mutex<DecodedImage>>,
    frame: Frame,
}

impl Processor {
    pub fn process(
        &mut self,
        header: &FastPathHeader,
        stream: &[u8],
        mut output: impl io::Write,
    ) -> Result<(), RdpError> {
        debug!("Got Fast-Path Header: {:?}", header);

        let update_pdu = FastPathUpdatePdu::from_buffer(stream)?;
        debug!("Fast-Path Update fragmentation: {:?}", update_pdu.fragmentation);

        let processed_complete_data = self
            .complete_data
            .process_data(update_pdu.data, update_pdu.fragmentation);
        let update_code = update_pdu.update_code;

        if let Some(data) = processed_complete_data {
            let update = FastPathUpdate::from_buffer_with_code(data.as_slice(), update_code);

            match update {
                Ok(FastPathUpdate::SurfaceCommands(surface_commands)) => {
                    info!("Received Surface Commands: {} pieces", surface_commands.len());

                    self.process_surface_commands(&mut output, surface_commands)?;
                }
                Ok(FastPathUpdate::Bitmap(bitmap)) => {
                    info!("Received Bitmap: {:?}", bitmap);
                }
                Err(FastPathError::UnsupportedFastPathUpdate(code))
                    if code == UpdateCode::Orders || code == UpdateCode::Palette =>
                {
                    return Err(RdpError::UnexpectedFastPathUpdate(code));
                }
                Err(FastPathError::UnsupportedFastPathUpdate(update_code)) => {
                    warn!("Received unsupported Fast-Path update: {:?}", update_code)
                }
                Err(FastPathError::BitmapError(error)) => {
                    warn!("Received invalid bitmap: {:?}", error)
                }
                Err(e) => {
                    return Err(RdpError::from(e));
                }
            }
        }

        Ok(())
    }

    fn process_surface_commands(
        &mut self,
        mut output: impl io::Write,
        surface_commands: Vec<SurfaceCommand<'_>>,
    ) -> Result<(), RdpError> {
        for command in surface_commands {
            match command {
                SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) => {
                    info!("Surface bits");
                    let codec_id = CodecId::from_u8(bits.extended_bitmap_data.codec_id)
                        .ok_or(RdpError::UnexpectedCodecId(bits.extended_bitmap_data.codec_id))?;
                    match codec_id {
                        CodecId::RemoteFx => {
                            let destination = bits.destination;
                            let mut data = bits.extended_bitmap_data.data;

                            while !data.is_empty() {
                                self.rfx_handler
                                    .decode(&destination, &mut data, self.decoded_image.clone())?;
                            }
                        }
                    }
                }
                SurfaceCommand::FrameMarker(marker) => {
                    info!(
                        "Frame marker: action {:?} with ID #{}",
                        marker.frame_action,
                        marker.frame_id.unwrap_or(0)
                    );
                    self.frame.process_marker(&marker, &mut output)?;
                }
            }
        }

        Ok(())
    }
}

pub struct ProcessorBuilder {
    pub decoded_image: Arc<Mutex<DecodedImage>>,
    pub global_channel_id: u16,
    pub initiator_id: u16,
}

impl ProcessorBuilder {
    pub fn build(self) -> Processor {
        Processor {
            complete_data: CompleteData::new(),
            rfx_handler: rfx::DecodingContext::new(DESTINATION_PIXEL_FORMAT),
            decoded_image: self.decoded_image.clone(),
            frame: Frame::new(self.initiator_id, self.global_channel_id),
        }
    }
}

#[derive(Debug, PartialEq)]
struct CompleteData {
    fragmented_data: Option<Vec<u8>>,
}

impl CompleteData {
    fn new() -> Self {
        Self { fragmented_data: None }
    }

    fn process_data(&mut self, data: &[u8], fragmentation: Fragmentation) -> Option<Vec<u8>> {
        match fragmentation {
            Fragmentation::Single => {
                self.check_data_is_empty();

                Some(data.to_vec())
            }
            Fragmentation::First => {
                self.check_data_is_empty();

                self.fragmented_data = Some(data.to_vec());

                None
            }
            Fragmentation::Next => {
                self.append_data(data);

                None
            }
            Fragmentation::Last => {
                self.append_data(data);

                let complete_data = self.fragmented_data.take().unwrap();

                Some(complete_data)
            }
        }
    }

    fn check_data_is_empty(&mut self) {
        if self.fragmented_data.is_some() {
            warn!("Skipping pending Fast-Path Update internal multiple elements data");
            self.fragmented_data = None;
        }
    }

    fn append_data(&mut self, data: &[u8]) {
        if self.fragmented_data.is_none() {
            warn!("Got unexpected Next fragmentation PDU without prior First fragmentation PDU");
            self.fragmented_data = None;
        } else {
            self.fragmented_data.as_mut().unwrap().extend_from_slice(data);
        }
    }
}

struct Frame {
    transport: ShareDataHeaderTransport,
}

impl Frame {
    fn new(initiator_id: u16, global_channel_id: u16) -> Self {
        Self {
            transport: ShareDataHeaderTransport::new(ShareControlHeaderTransport::new(
                SendDataContextTransport::new(
                    McsTransport::new(DataTransport::default()),
                    initiator_id,
                    global_channel_id,
                ),
                initiator_id,
                global_channel_id,
            )),
        }
    }

    fn process_marker(&mut self, marker: &FrameMarkerPdu, mut output: impl io::Write) -> Result<(), RdpError> {
        match marker.frame_action {
            FrameAction::Begin => Ok(()),
            FrameAction::End => self.transport.encode(
                ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu {
                    frame_id: marker.frame_id.unwrap_or(0),
                }),
                &mut output,
            ),
        }
    }
}

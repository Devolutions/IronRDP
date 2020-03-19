use std::{
    io,
    sync::{Arc, Mutex},
};

use ironrdp::{
    codecs::rfx::FrameAcknowledgePdu,
    fast_path::{
        FastPathError, FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode,
    },
    surface_commands::{FrameAction, SurfaceCommand},
    PduBufferParsing, ShareDataPdu,
};
use log::{debug, info, warn};
use num_traits::FromPrimitive;

use super::{codecs::rfx, DecodedImage, DESTINATION_PIXEL_FORMAT};
use crate::{
    transport::{
        DataTransport, Encoder, McsTransport, SendDataContextTransport,
        ShareControlHeaderTransport, ShareDataHeaderTransport,
    },
    utils::CodecId,
    RdpError, RdpResult,
};
use ironrdp::surface_commands::FrameMarkerPdu;

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
        mut stream: impl io::BufRead + io::Write,
    ) -> RdpResult<()> {
        debug!("Got Fast-Path Header: {:?}", header);

        let input_buffer = stream.fill_buf()?;

        let update_pdu = FastPathUpdatePdu::from_buffer(input_buffer)?;
        let update_pdu_length = update_pdu.buffer_length();

        debug!(
            "Fast-Path Update fragmentation: {:?}",
            update_pdu.fragmentation
        );

        let processed_complete_data = self.complete_data.process_pdu(update_pdu);
        stream.consume(update_pdu_length);

        if let Some((update_code, data)) = processed_complete_data {
            let update =
                FastPathUpdate::from_buffer_consume_with_code(&mut data.as_slice(), update_code);
            info!("Got Fast-Path Update: {:?}", update_code);

            match update {
                Ok(FastPathUpdate::SurfaceCommands(surface_commands)) => {
                    self.process_surface_commands(&mut stream, surface_commands)?;
                }
                Err(FastPathError::UnsupportedFastPathUpdate(update_code)) => {
                    warn!("Received unsupported Fast-Path update: {:?}", update_code)
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
    ) -> RdpResult<()> {
        for command in surface_commands {
            match command {
                SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) => {
                    info!("Surface bits");
                    let codec_id = CodecId::from_u8(bits.extended_bitmap_data.codec_id).ok_or(
                        RdpError::UnexpectedCodecId(bits.extended_bitmap_data.codec_id),
                    )?;
                    match codec_id {
                        CodecId::RemoteFx => {
                            let destination = bits.destination;
                            let mut data = bits.extended_bitmap_data.data;

                            while !data.is_empty() {
                                self.rfx_handler.decode(
                                    &destination,
                                    &mut data,
                                    self.decoded_image.clone(),
                                )?;
                            }
                        }
                    }
                }
                SurfaceCommand::FrameMarker(marker) => {
                    info!(
                        "Frame marker: action {:?} with ID #{}",
                        marker.frame_action, marker.frame_id
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
    fast_path_pdu: Option<UpdateCode>,
    data: Option<Vec<u8>>,
}

impl CompleteData {
    fn new() -> Self {
        Self {
            fast_path_pdu: None,
            data: None,
        }
    }

    fn process_pdu(
        &mut self,
        fast_path_pdu: FastPathUpdatePdu<'_>,
    ) -> Option<(UpdateCode, Vec<u8>)> {
        match fast_path_pdu.fragmentation {
            Fragmentation::Single => {
                self.check_data_is_empty();

                Some((fast_path_pdu.update_code, fast_path_pdu.data.to_vec()))
            }
            Fragmentation::First => {
                self.check_data_is_empty();

                self.data = Some(fast_path_pdu.data.to_vec());
                self.fast_path_pdu = Some(fast_path_pdu.update_code);

                None
            }
            Fragmentation::Next => {
                self.append_data(fast_path_pdu.data);

                None
            }
            Fragmentation::Last => {
                self.append_data(fast_path_pdu.data);

                let update_code = self.fast_path_pdu.take().unwrap();
                let data = self.data.take().unwrap();

                Some((update_code, data))
            }
        }
    }

    fn check_data_is_empty(&mut self) {
        if self.data.is_some() && self.fast_path_pdu.is_some() {
            warn!("Skipping pending Fast-Path Update internal multiple elements data");
            self.data = None;
            self.fast_path_pdu = None;
        }
    }

    fn append_data(&mut self, data: &[u8]) {
        if self.data.is_none() && self.fast_path_pdu.is_none() {
            warn!("Got unexpected Next fragmentation PDU without prior First fragmentation PDU");
            self.data = None;
            self.fast_path_pdu = None;
        } else {
            self.data.as_mut().unwrap().extend_from_slice(data);
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

    fn process_marker(
        &mut self,
        marker: &FrameMarkerPdu,
        mut output: impl io::Write,
    ) -> RdpResult<()> {
        match marker.frame_action {
            FrameAction::Begin => Ok(()),
            FrameAction::End => self.transport.encode(
                ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu {
                    frame_id: marker.frame_id,
                }),
                &mut output,
            ),
        }
    }
}

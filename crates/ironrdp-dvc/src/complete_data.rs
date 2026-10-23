use alloc::vec::Vec;
use core::cmp;

use ironrdp_bulk::rdp8::Rdp8Decompressor;
use ironrdp_core::{DecodeResult, cast_length, invalid_field_err, other_err};
use tracing::error;

use crate::pdu::DrdynvcDataPdu;

#[derive(Debug)]
pub(crate) struct CompleteData {
    total_size: usize,
    data: Vec<u8>,
    /// MS-RDPEDYC 3.1.5.2.5: each channel has its own decompression context, created when
    /// the channel first receives compressed data.
    decompressor: Option<Rdp8Decompressor>,
    /// Set once decompression fails. The history may have advanced partway through the bad
    /// block, so it no longer matches the sender's and cannot decode anything after it.
    decompression_failed: bool,
}

impl CompleteData {
    pub(crate) fn new() -> Self {
        Self {
            total_size: 0,
            data: Vec::new(),
            decompressor: None,
            decompression_failed: false,
        }
    }

    pub(crate) fn process_data(&mut self, pdu: DrdynvcDataPdu) -> DecodeResult<Option<Vec<u8>>> {
        match pdu {
            DrdynvcDataPdu::DataFirst(data_first) => {
                let total_size = data_first.length();
                self.process_data_first(total_size, data_first.into_data())
            }
            DrdynvcDataPdu::Data(data) => self.process_data_block(data.into_data()),
            DrdynvcDataPdu::DataFirstCompressed(data_first) => {
                let data = self.decompress(data_first.data())?;
                self.process_data_first(data_first.length(), data)
            }
            DrdynvcDataPdu::DataCompressed(data) => {
                let data = self.decompress(data.data())?;
                self.process_data_block(data)
            }
        }
    }

    fn decompress(&mut self, compressed: &[u8]) -> DecodeResult<Vec<u8>> {
        if self.decompression_failed {
            return Err(invalid_field_err!(
                "DVC compressed data",
                "data",
                "the channel's decompression history is out of sync after an earlier error"
            ));
        }

        let decompressor = self.decompressor.get_or_insert_with(Rdp8Decompressor::new_lite);
        let mut data = Vec::new();
        if let Err(e) = decompressor.decompress(compressed, &mut data) {
            self.decompression_failed = true;
            return Err(other_err!("DVC compressed data", source: e));
        }
        Ok(data)
    }

    fn process_data_first(&mut self, total_size: u32, data: Vec<u8>) -> DecodeResult<Option<Vec<u8>>> {
        let total_data_size: DecodeResult<_> = cast_length!("DataFirstPdu::length", total_size);
        let total_data_size = total_data_size?;
        if self.total_size != 0 || !self.data.is_empty() {
            error!("Incomplete DVC message, it will be skipped");

            self.data.clear();
        }

        if total_data_size == data.len() {
            Ok(Some(data))
        } else if data.len() > total_data_size {
            self.total_size = 0;
            self.data.clear();

            Err(invalid_field_err!(
                "DVC message",
                "data",
                "first block is longer than the message length"
            ))
        } else {
            self.total_size = total_data_size;
            self.data = data;

            Ok(None)
        }
    }

    fn process_data_block(&mut self, mut data: Vec<u8>) -> DecodeResult<Option<Vec<u8>>> {
        if self.total_size == 0 && self.data.is_empty() {
            // message is not fragmented
            return Ok(Some(data));
        }

        // The message is fragmented and needs to be reassembled.
        match self.data.len().checked_add(data.len()) {
            Some(actual_data_length) => {
                match actual_data_length.cmp(&(self.total_size)) {
                    cmp::Ordering::Less => {
                        // this is one of the fragmented messages, just append it
                        self.data.append(&mut data);
                        Ok(None)
                    }
                    cmp::Ordering::Equal => {
                        // this is the last fragmented message, need to return the whole reassembled message
                        self.total_size = 0;
                        self.data.append(&mut data);
                        Ok(Some(self.data.drain(..).collect()))
                    }
                    cmp::Ordering::Greater => {
                        self.total_size = 0;
                        self.data.clear();
                        Err(invalid_field_err!(
                            "DVC message",
                            "data",
                            "blocks are longer than the message length"
                        ))
                    }
                }
            }
            _ => Err(invalid_field_err!("DVC message", "data", "overflow occurred")),
        }
    }
}

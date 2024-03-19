use alloc::vec::Vec;
use core::cmp;
use ironrdp_pdu::{cast_length, dvc, invalid_message_err, PduResult};

use crate::pdu::{DataFirstPdu, DataPdu, DrdynvcDataPdu};

#[derive(Debug, PartialEq)]
pub(crate) struct CompleteData {
    total_size: usize,
    data: Vec<u8>,
}

impl CompleteData {
    pub(crate) fn new() -> Self {
        Self {
            total_size: 0,
            data: Vec::new(),
        }
    }

    pub(crate) fn process_data(&mut self, pdu: DrdynvcDataPdu) -> PduResult<Option<Vec<u8>>> {
        match pdu {
            DrdynvcDataPdu::DataFirst(data_first) => self.process_data_first_pdu(data_first),
            DrdynvcDataPdu::Data(data) => self.process_data_pdu(data),
        }
    }

    fn process_data_first_pdu(&mut self, data_first: DataFirstPdu) -> PduResult<Option<Vec<u8>>> {
        let total_data_size = cast_length!("DataFirstPdu::length", data_first.length)?;
        if self.total_size != 0 || !self.data.is_empty() {
            error!("Incomplete DVC message, it will be skipped");

            self.data.clear();
        }

        if total_data_size == data_first.data.len() {
            Ok(Some(data_first.data))
        } else {
            self.total_size = total_data_size;
            self.data = data_first.data;

            Ok(None)
        }
    }

    fn process_data_pdu(&mut self, mut data: DataPdu) -> PduResult<Option<Vec<u8>>> {
        if self.total_size == 0 && self.data.is_empty() {
            // message is not fragmented
            Ok(Some(data.data))
        } else {
            // message is fragmented so need to reassemble it
            match self.data.len().checked_add(data.data.len()) {
                Some(actual_data_length) => {
                    match actual_data_length.cmp(&(self.total_size)) {
                        cmp::Ordering::Less => {
                            // this is one of the fragmented messages, just append it
                            self.data.append(&mut data.data);
                            Ok(None)
                        }
                        cmp::Ordering::Equal => {
                            // this is the last fragmented message, need to return the whole reassembled message
                            self.total_size = 0;
                            self.data.append(&mut data.data);
                            Ok(Some(self.data.drain(..).collect()))
                        }
                        cmp::Ordering::Greater => {
                            error!("Actual DVC message size is grater than expected total DVC message size");
                            self.total_size = 0;
                            self.data.clear();
                            Ok(None)
                        }
                    }
                }
                _ => Err(invalid_message_err!("DVC message", "data", "overflow occurred")),
            }
        }
    }
}

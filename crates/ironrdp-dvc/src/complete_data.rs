use alloc::vec::Vec;
use core::cmp;

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

    pub(crate) fn process_data_first_pdu(&mut self, total_data_size: usize, data: Vec<u8>) -> Option<Vec<u8>> {
        if self.total_size != 0 || !self.data.is_empty() {
            error!("Incomplete DVC message, it will be skipped");

            self.data.clear();
        }

        if total_data_size == data.len() {
            Some(data)
        } else {
            self.total_size = total_data_size;
            self.data = data;

            None
        }
    }

    pub(crate) fn process_data_pdu(&mut self, mut data: Vec<u8>) -> Option<Vec<u8>> {
        if self.total_size == 0 && self.data.is_empty() {
            // message is not fragmented
            Some(data)
        } else {
            // message is fragmented so need to reassemble it
            match self.data.len().checked_add(data.len()) {
                Some(actual_data_length) => {
                    match actual_data_length.cmp(&(self.total_size)) {
                        cmp::Ordering::Less => {
                            // this is one of the fragmented messages, just append it
                            self.data.append(&mut data);
                            None
                        }
                        cmp::Ordering::Equal => {
                            // this is the last fragmented message, need to return the whole reassembled message
                            self.total_size = 0;
                            self.data.append(&mut data);
                            Some(self.data.drain(..).collect())
                        }
                        cmp::Ordering::Greater => {
                            error!("Actual DVC message size is grater than expected total DVC message size");
                            self.total_size = 0;
                            self.data.clear();

                            None
                        }
                    }
                }
                _ => {
                    error!("DVC message size overflow occurred");
                    None
                }
            }
        }
    }
}

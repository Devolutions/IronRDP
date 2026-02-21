use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Read, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PacError {
    #[error("Version must be 0 but got {0}")]
    Version(u32),
    #[error("Invalid pac buffer type: {0}")]
    PacBufferType(u32),
    #[error("Unable to read bytes: {0:?}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid version: expected 0 but got {0}")]
    InvalidVersion(u32),
    #[error("Invalid buffer data range")]
    InvalidRange,
}

/// [MS-PAC](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-PAC/%5bMS-PAC%5d.pdf)
/// Section 2.4 PAC_INFO_BUFFER ulType
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PacBufferType {
    KerbValidationInfo,
    Credentials,
    ServerChecksum,
    KdcChecksum,
    ClientInfo,
    ConstrainedDelegationInformation,
    UpnDnsInfo,
    ClientClaimsInfo,
    DeviceInfo,
    DeviceClaimsInfo,
    TicketChecksum,
}

impl TryFrom<u32> for PacBufferType {
    type Error = PacError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(PacBufferType::KerbValidationInfo),
            2 => Ok(PacBufferType::Credentials),
            6 => Ok(PacBufferType::ServerChecksum),
            7 => Ok(PacBufferType::KdcChecksum),
            10 => Ok(PacBufferType::ClientInfo),
            11 => Ok(PacBufferType::ConstrainedDelegationInformation),
            12 => Ok(PacBufferType::UpnDnsInfo),
            13 => Ok(PacBufferType::ClientClaimsInfo),
            14 => Ok(PacBufferType::DeviceInfo),
            15 => Ok(PacBufferType::DeviceClaimsInfo),
            16 => Ok(PacBufferType::TicketChecksum),
            n => Err(PacError::PacBufferType(n)),
        }
    }
}

impl From<PacBufferType> for u32 {
    fn from(buffer_type: PacBufferType) -> Self {
        match buffer_type {
            PacBufferType::KerbValidationInfo => 1,
            PacBufferType::Credentials => 2,
            PacBufferType::ServerChecksum => 6,
            PacBufferType::KdcChecksum => 7,
            PacBufferType::ClientInfo => 10,
            PacBufferType::ConstrainedDelegationInformation => 11,
            PacBufferType::UpnDnsInfo => 12,
            PacBufferType::ClientClaimsInfo => 13,
            PacBufferType::DeviceInfo => 14,
            PacBufferType::DeviceClaimsInfo => 15,
            PacBufferType::TicketChecksum => 16,
        }
    }
}

/// [MS-PAC](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-PAC/%5bMS-PAC%5d.pdf)
/// Section 2.4 PAC_INFO_BUFFER
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PacInfoBuffer {
    ul_type: PacBufferType,
    cb_buffer_size: u32,
    offset: u64,
}

impl PacInfoBuffer {
    pub fn deserialize(data: &mut impl Read) -> Result<Self, PacError> {
        Ok(Self {
            ul_type: data.read_u32::<LittleEndian>()?.try_into()?,
            cb_buffer_size: data.read_u32::<LittleEndian>()?,
            offset: data.read_u64::<LittleEndian>()?,
        })
    }

    pub fn serialize(&self, data: &mut impl Write) -> Result<(), PacError> {
        data.write_u32::<LittleEndian>(self.ul_type.clone().into())?;
        data.write_u32::<LittleEndian>(self.cb_buffer_size)?;
        data.write_u64::<LittleEndian>(self.offset)?;

        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PacBuffer {
    info_buffer: PacInfoBuffer,
    data: Vec<u8>,
}

impl PacBuffer {
    pub fn deserialize(data: &mut Cursor<&[u8]>) -> Result<Self, PacError> {
        let info_buffer = PacInfoBuffer::deserialize(data)?;

        let from = info_buffer.offset as usize;
        let to = from + info_buffer.cb_buffer_size as usize;

        if to >= data.get_ref().len() {
            return Err(PacError::InvalidRange);
        }
        let data = data.get_ref()[from..to].to_vec();

        Ok(Self { info_buffer, data })
    }

    pub fn serialize(&self, data: &mut Cursor<Vec<u8>>) -> Result<(), PacError> {
        self.info_buffer.serialize(data)?;

        let from = self.info_buffer.offset as usize;
        let to = from + self.info_buffer.cb_buffer_size as usize;

        if to - from != self.data.len() || to >= data.get_ref().len() {
            return Err(PacError::InvalidRange);
        }

        data.get_mut()[from..to].copy_from_slice(&self.data);

        Ok(())
    }
}

/// [MS-PAC](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-PAC/%5bMS-PAC%5d.pdf)
/// Section 2.3 PACTYPE
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Pac {
    c_buffers: u32,
    buffers: Vec<PacBuffer>,
}

impl Pac {
    pub fn deserialize(mut reader: impl Read) -> Result<Self, PacError> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;
        let mut data = Cursor::new(data.as_ref());

        let c_buffers = data.read_u32::<LittleEndian>()?;

        let version = data.read_u32::<LittleEndian>()?;
        if version != 0 {
            return Err(PacError::InvalidVersion(version));
        }

        let mut buffers = Vec::with_capacity(c_buffers as usize);
        for _ in 0..c_buffers {
            buffers.push(PacBuffer::deserialize(&mut data)?);
        }

        Ok(Self { c_buffers, buffers })
    }

    pub fn serialize(&self, mut data: impl Write) -> Result<(), PacError> {
        let data_size = 4 /* c_buffers: buffers amount */
            + 4 /* version */
            + self.c_buffers as usize * 16 /* ul_type (4) + cb_buffer_size (4) + offset (8) = 16 */
            + self.buffers.iter().fold(0, |s, b| {
                let size = b.info_buffer.cb_buffer_size;
                // [MS-PAC](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-PAC/%5bMS-PAC%5d.pdf)
                // Section 2.3 PACTYPE: All PAC elements MUST be placed on an 8-byte boundary
                s + b.info_buffer.cb_buffer_size as usize + if size % 8 == 0 {
                    0
                } else {
                    (8 - size % 8) as usize
                }
            });
        let mut c = Cursor::new(vec![0; data_size]);

        c.write_u32::<LittleEndian>(self.c_buffers)?;
        // [MS-PAC](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-PAC/%5bMS-PAC%5d.pdf)
        // Section 2.3 PACTYPE: MUST be 0x00000000
        c.write_u32::<LittleEndian>(0)?;

        for buffer in self.buffers.iter() {
            buffer.serialize(&mut c)?;
        }

        data.write_all(c.get_ref())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Pac, PacBuffer, PacBufferType, PacInfoBuffer};

    #[test]
    fn pac() {
        let expected_pac = Pac {
            c_buffers: 4,
            buffers: vec![
                PacBuffer {
                    info_buffer: PacInfoBuffer {
                        ul_type: PacBufferType::KerbValidationInfo,
                        cb_buffer_size: 1200,
                        offset: 72,
                    },
                    data: vec![
                        1, 16, 8, 0, 204, 204, 204, 204, 160, 4, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 209, 134, 102, 15, 101,
                        106, 198, 1, 255, 255, 255, 255, 255, 255, 255, 127, 255, 255, 255, 255, 255, 255, 255, 127,
                        23, 212, 57, 254, 120, 74, 198, 1, 23, 148, 163, 40, 66, 75, 198, 1, 23, 84, 36, 151, 122, 129,
                        198, 1, 8, 0, 8, 0, 4, 0, 2, 0, 36, 0, 36, 0, 8, 0, 2, 0, 18, 0, 18, 0, 12, 0, 2, 0, 0, 0, 0,
                        0, 16, 0, 2, 0, 0, 0, 0, 0, 20, 0, 2, 0, 0, 0, 0, 0, 24, 0, 2, 0, 84, 16, 0, 0, 151, 121, 44,
                        0, 1, 2, 0, 0, 26, 0, 0, 0, 28, 0, 2, 0, 32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 22, 0, 24, 0, 32, 0, 2, 0, 10, 0, 12, 0, 36, 0, 2, 0, 40, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 13, 0, 0, 0, 44, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 4,
                        0, 0, 0, 108, 0, 122, 0, 104, 0, 117, 0, 18, 0, 0, 0, 0, 0, 0, 0, 18, 0, 0, 0, 76, 0, 105, 0,
                        113, 0, 105, 0, 97, 0, 110, 0, 103, 0, 40, 0, 76, 0, 97, 0, 114, 0, 114, 0, 121, 0, 41, 0, 32,
                        0, 90, 0, 104, 0, 117, 0, 9, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 110, 0, 116, 0, 100, 0, 115, 0,
                        50, 0, 46, 0, 98, 0, 97, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 26, 0, 0, 0, 97, 196, 51, 0, 7, 0, 0, 0,
                        9, 195, 45, 0, 7, 0, 0, 0, 94, 180, 50, 0, 7, 0, 0, 0, 1, 2, 0, 0, 7, 0, 0, 0, 151, 185, 44, 0,
                        7, 0, 0, 0, 43, 241, 50, 0, 7, 0, 0, 0, 206, 48, 51, 0, 7, 0, 0, 0, 167, 46, 46, 0, 7, 0, 0, 0,
                        42, 241, 50, 0, 7, 0, 0, 0, 152, 185, 44, 0, 7, 0, 0, 0, 98, 196, 51, 0, 7, 0, 0, 0, 148, 1,
                        51, 0, 7, 0, 0, 0, 118, 196, 51, 0, 7, 0, 0, 0, 174, 254, 45, 0, 7, 0, 0, 0, 50, 210, 44, 0, 7,
                        0, 0, 0, 22, 8, 50, 0, 7, 0, 0, 0, 66, 91, 46, 0, 7, 0, 0, 0, 95, 180, 50, 0, 7, 0, 0, 0, 202,
                        156, 53, 0, 7, 0, 0, 0, 133, 68, 45, 0, 7, 0, 0, 0, 194, 240, 50, 0, 7, 0, 0, 0, 233, 234, 49,
                        0, 7, 0, 0, 0, 237, 142, 46, 0, 7, 0, 0, 0, 182, 235, 49, 0, 7, 0, 0, 0, 171, 46, 46, 0, 7, 0,
                        0, 0, 114, 14, 46, 0, 7, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 11, 0, 0, 0, 78, 0, 84, 0, 68, 0,
                        69, 0, 86, 0, 45, 0, 68, 0, 67, 0, 45, 0, 48, 0, 53, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0,
                        0, 78, 0, 84, 0, 68, 0, 69, 0, 86, 0, 0, 0, 4, 0, 0, 0, 1, 4, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0,
                        89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 13, 0, 0, 0, 48, 0, 2, 0, 7, 0, 0, 0, 52,
                        0, 2, 0, 7, 0, 0, 32, 56, 0, 2, 0, 7, 0, 0, 32, 60, 0, 2, 0, 7, 0, 0, 32, 64, 0, 2, 0, 7, 0, 0,
                        32, 68, 0, 2, 0, 7, 0, 0, 32, 72, 0, 2, 0, 7, 0, 0, 32, 76, 0, 2, 0, 7, 0, 0, 32, 80, 0, 2, 0,
                        7, 0, 0, 32, 84, 0, 2, 0, 7, 0, 0, 32, 88, 0, 2, 0, 7, 0, 0, 32, 92, 0, 2, 0, 7, 0, 0, 32, 96,
                        0, 2, 0, 7, 0, 0, 32, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 185, 48, 27, 46, 183,
                        65, 76, 108, 140, 59, 53, 21, 1, 2, 0, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89,
                        81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 116, 84, 47, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0,
                        0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 232, 56, 50, 0, 5, 0, 0,
                        0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11,
                        205, 56, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93,
                        37, 100, 99, 59, 11, 93, 180, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81,
                        184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 65, 22, 53, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5,
                        21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 232, 234, 49, 0, 5, 0, 0, 0,
                        1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 193,
                        25, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37,
                        100, 99, 59, 11, 41, 241, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184,
                        23, 102, 114, 93, 37, 100, 99, 59, 11, 15, 95, 46, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21,
                        0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 47, 91, 46, 0, 5, 0, 0, 0, 1, 5,
                        0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 239, 143,
                        49, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100,
                        99, 59, 11, 7, 95, 46, 0, 0, 0, 0, 0,
                    ],
                },
                PacBuffer {
                    info_buffer: PacInfoBuffer {
                        ul_type: PacBufferType::ClientInfo,
                        cb_buffer_size: 18,
                        offset: 1272,
                    },
                    data: vec![0, 73, 217, 14, 101, 106, 198, 1, 8, 0, 108, 0, 122, 0, 104, 0, 117, 0],
                },
                PacBuffer {
                    info_buffer: PacInfoBuffer {
                        ul_type: PacBufferType::ServerChecksum,
                        cb_buffer_size: 20,
                        offset: 1296,
                    },
                    data: vec![
                        118, 255, 255, 255, 65, 237, 206, 154, 52, 129, 93, 58, 239, 123, 201, 136, 116, 128, 93, 37,
                    ],
                },
                PacBuffer {
                    info_buffer: PacInfoBuffer {
                        ul_type: PacBufferType::KdcChecksum,
                        cb_buffer_size: 20,
                        offset: 1320,
                    },
                    data: vec![
                        118, 255, 255, 255, 247, 165, 52, 218, 178, 192, 41, 134, 239, 224, 251, 229, 17, 10, 79, 50,
                    ],
                },
            ],
        };
        let expected_raw = vec![
            4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 176, 4, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 10, 0, 0, 0, 18, 0, 0, 0, 248,
            4, 0, 0, 0, 0, 0, 0, 6, 0, 0, 0, 20, 0, 0, 0, 16, 5, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 20, 0, 0, 0, 40, 5, 0,
            0, 0, 0, 0, 0, 1, 16, 8, 0, 204, 204, 204, 204, 160, 4, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 209, 134, 102, 15,
            101, 106, 198, 1, 255, 255, 255, 255, 255, 255, 255, 127, 255, 255, 255, 255, 255, 255, 255, 127, 23, 212,
            57, 254, 120, 74, 198, 1, 23, 148, 163, 40, 66, 75, 198, 1, 23, 84, 36, 151, 122, 129, 198, 1, 8, 0, 8, 0,
            4, 0, 2, 0, 36, 0, 36, 0, 8, 0, 2, 0, 18, 0, 18, 0, 12, 0, 2, 0, 0, 0, 0, 0, 16, 0, 2, 0, 0, 0, 0, 0, 20,
            0, 2, 0, 0, 0, 0, 0, 24, 0, 2, 0, 84, 16, 0, 0, 151, 121, 44, 0, 1, 2, 0, 0, 26, 0, 0, 0, 28, 0, 2, 0, 32,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 22, 0, 24, 0, 32, 0, 2, 0, 10, 0, 12, 0, 36, 0, 2,
            0, 40, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 13, 0, 0, 0, 44, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0,
            0, 0, 0, 4, 0, 0, 0, 108, 0, 122, 0, 104, 0, 117, 0, 18, 0, 0, 0, 0, 0, 0, 0, 18, 0, 0, 0, 76, 0, 105, 0,
            113, 0, 105, 0, 97, 0, 110, 0, 103, 0, 40, 0, 76, 0, 97, 0, 114, 0, 114, 0, 121, 0, 41, 0, 32, 0, 90, 0,
            104, 0, 117, 0, 9, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 0, 110, 0, 116, 0, 100, 0, 115, 0, 50, 0, 46, 0, 98, 0,
            97, 0, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 26, 0, 0, 0, 97, 196, 51, 0, 7, 0, 0, 0, 9, 195, 45, 0, 7, 0, 0, 0, 94, 180, 50, 0, 7,
            0, 0, 0, 1, 2, 0, 0, 7, 0, 0, 0, 151, 185, 44, 0, 7, 0, 0, 0, 43, 241, 50, 0, 7, 0, 0, 0, 206, 48, 51, 0,
            7, 0, 0, 0, 167, 46, 46, 0, 7, 0, 0, 0, 42, 241, 50, 0, 7, 0, 0, 0, 152, 185, 44, 0, 7, 0, 0, 0, 98, 196,
            51, 0, 7, 0, 0, 0, 148, 1, 51, 0, 7, 0, 0, 0, 118, 196, 51, 0, 7, 0, 0, 0, 174, 254, 45, 0, 7, 0, 0, 0, 50,
            210, 44, 0, 7, 0, 0, 0, 22, 8, 50, 0, 7, 0, 0, 0, 66, 91, 46, 0, 7, 0, 0, 0, 95, 180, 50, 0, 7, 0, 0, 0,
            202, 156, 53, 0, 7, 0, 0, 0, 133, 68, 45, 0, 7, 0, 0, 0, 194, 240, 50, 0, 7, 0, 0, 0, 233, 234, 49, 0, 7,
            0, 0, 0, 237, 142, 46, 0, 7, 0, 0, 0, 182, 235, 49, 0, 7, 0, 0, 0, 171, 46, 46, 0, 7, 0, 0, 0, 114, 14, 46,
            0, 7, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 11, 0, 0, 0, 78, 0, 84, 0, 68, 0, 69, 0, 86, 0, 45, 0, 68, 0, 67,
            0, 45, 0, 48, 0, 53, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 78, 0, 84, 0, 68, 0, 69, 0, 86, 0, 0, 0,
            4, 0, 0, 0, 1, 4, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 13, 0,
            0, 0, 48, 0, 2, 0, 7, 0, 0, 0, 52, 0, 2, 0, 7, 0, 0, 32, 56, 0, 2, 0, 7, 0, 0, 32, 60, 0, 2, 0, 7, 0, 0,
            32, 64, 0, 2, 0, 7, 0, 0, 32, 68, 0, 2, 0, 7, 0, 0, 32, 72, 0, 2, 0, 7, 0, 0, 32, 76, 0, 2, 0, 7, 0, 0, 32,
            80, 0, 2, 0, 7, 0, 0, 32, 84, 0, 2, 0, 7, 0, 0, 32, 88, 0, 2, 0, 7, 0, 0, 32, 92, 0, 2, 0, 7, 0, 0, 32, 96,
            0, 2, 0, 7, 0, 0, 32, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 185, 48, 27, 46, 183, 65, 76, 108,
            140, 59, 53, 21, 1, 2, 0, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114,
            93, 37, 100, 99, 59, 11, 116, 84, 47, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23,
            102, 114, 93, 37, 100, 99, 59, 11, 232, 56, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81,
            184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 205, 56, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0,
            0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 93, 180, 50, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5,
            21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 65, 22, 53, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0,
            0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 232, 234, 49, 0, 5, 0, 0, 0, 1,
            5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 193, 25, 50, 0, 5, 0,
            0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 41, 241, 50,
            0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59, 11, 15,
            95, 46, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100, 99, 59,
            11, 47, 91, 46, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114, 93, 37, 100,
            99, 59, 11, 239, 143, 49, 0, 5, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 89, 81, 184, 23, 102, 114,
            93, 37, 100, 99, 59, 11, 7, 95, 46, 0, 0, 0, 0, 0, 0, 73, 217, 14, 101, 106, 198, 1, 8, 0, 108, 0, 122, 0,
            104, 0, 117, 0, 0, 0, 0, 0, 0, 0, 118, 255, 255, 255, 65, 237, 206, 154, 52, 129, 93, 58, 239, 123, 201,
            136, 116, 128, 93, 37, 0, 0, 0, 0, 118, 255, 255, 255, 247, 165, 52, 218, 178, 192, 41, 134, 239, 224, 251,
            229, 17, 10, 79, 50, 0, 0, 0, 0,
        ];

        let pac = Pac::deserialize(expected_raw.as_slice()).unwrap();
        let mut pac_raw = Vec::new();
        pac.serialize(&mut pac_raw).unwrap();

        assert_eq!(expected_pac, pac);
        assert_eq!(expected_raw, pac_raw);
    }
}

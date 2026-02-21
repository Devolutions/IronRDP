use std::io::{self, Read, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

pub mod data_types;
pub mod messages;

/// [2.2.3 Constants](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// #define MESSAGE_SIGNATURE 0x535458454f47454ei64 // "NEGOEXTS"
/// ```
pub const NEGOEXTS_MESSAGE_SIGNATURE: u64 = 0x535458454f47454e;

/// [2.2.3 Constants](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// #define CHECKSUM_SCHEME_RFC3961 1
/// ```
pub const CHECKSUM_SCHEME_RFC3961: u32 = 0x1;

/// [2.2.6.3 NEGO_MESSAGE](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ProtocolVersion: A ULONG64 type that indicates the numbered version of this protocol. This field contains 0.
pub const PROTOCOL_VERSION: u64 = 0;

/// [2.2.4 Random array](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// UCHAR Random[32];
/// ```
pub const RANDOM_ARRAY_SIZE: usize = 32;

/// This trait provides interface for decoding/encoding NEGOEX messages like Nego, Exchange, Verify
/// * [NEGOEX](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-negoex/77c795cf-e522-4678-b0f1-2063c5c0561c)
pub trait NegoexMessage
where
    Self: Sized,
{
    type Error;

    //= Decodes NEGOEX message. `message` is the NEGOEX message buffer =//
    fn decode(message: &[u8]) -> Result<Self, Self::Error>;

    //= Encodes NEGOEX message into provided `to`. =//
    fn encode(&self, to: impl Write) -> Result<(), Self::Error>;
}

/// This trait provides interface for decoding/encoding NEGOEX data types like MessageHeader, Checksum, etc.
/// * [NEGOEX](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-negoex/77c795cf-e522-4678-b0f1-2063c5c0561c)
pub trait NegoexDataType
where
    Self: Sized,
{
    type Error;

    //= Returns the encoded size of the data type. =//
    fn size(&self) -> usize;

    /// Decodes Self from the provided sources.
    /// `message` - the initial message buffer.
    fn decode(from: impl Read, message: &[u8]) -> Result<Self, Self::Error>;

    //= Encodes Self into `to`. The data will be places right after the header =//
    fn encode(&self, to: impl Write) -> Result<(), Self::Error>;

    //= Encodes header into the `to` and data into the `data` =//
    fn encode_with_payload(&self, offset: usize, to: impl Write, data: impl Write) -> Result<usize, Self::Error>;
}

impl NegoexDataType for u8 {
    type Error = io::Error;

    fn size(&self) -> usize {
        1
    }

    fn decode(mut from: impl Read, _message: &[u8]) -> Result<Self, Self::Error> {
        from.read_u8()
    }

    fn encode_with_payload(&self, _offset: usize, mut to: impl Write, _data: impl Write) -> Result<usize, Self::Error> {
        to.write_u8(*self)?;

        Ok(0)
    }

    fn encode(&self, to: impl Write) -> Result<(), Self::Error> {
        self.encode_with_payload(0, to, &mut [] as &mut [u8])?;

        Ok(())
    }
}

impl<T: NegoexDataType<Error = io::Error>> NegoexDataType for Vec<T> {
    type Error = io::Error;

    fn size(&self) -> usize {
        4 /* offset */ +
        4 /* count */ +
        self.first().map(|e| e.size()).unwrap_or_default() * self.len()
    }

    fn decode(mut from: impl Read, message: &[u8]) -> Result<Self, Self::Error> {
        let message_offset = from.read_u32::<LittleEndian>()? as usize;

        let count = from.read_u32::<LittleEndian>()? as usize;

        let mut reader = &message[message_offset..];

        let mut elements = Vec::with_capacity(count);

        for _ in 0..count {
            elements.push(T::decode(&mut reader, message)?);
        }

        Ok(elements)
    }

    fn encode_with_payload(
        &self,
        offset: usize,
        mut to: impl Write,
        mut data: impl Write,
    ) -> Result<usize, Self::Error> {
        if self.is_empty() {
            to.write_u32::<LittleEndian>(0)?;
        } else {
            to.write_u32::<LittleEndian>(offset as u32)?;
        }

        to.write_u32::<LittleEndian>(self.len() as u32)?;

        let mut elements_headers = Vec::new();
        let mut elements_data = Vec::new();

        let mut written = 0;
        for element in self.iter() {
            written += element.size();
            element.encode_with_payload(offset + written, &mut elements_headers, &mut elements_data)?;
        }

        data.write_all(&elements_headers)?;
        data.write_all(&elements_data)?;

        Ok(written)
    }

    fn encode(&self, mut to: impl Write) -> Result<(), Self::Error> {
        let mut header = Vec::new();
        let mut data = Vec::new();

        self.encode_with_payload(0, &mut header, &mut data)?;

        to.write_all(&header)?;
        to.write_all(&data)?;

        Ok(())
    }
}

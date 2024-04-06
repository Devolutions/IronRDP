#[cfg(test)]
mod test;

use super::{
    read_license_header, BlobHeader, BlobType, LicenseHeader, PreambleType, BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE, MAC_SIZE,
};
use crate::{
    cursor::{ReadCursor, WriteCursor},
    PduDecode, PduEncode, PduResult,
};

const CONNECT_FLAGS_FIELD_SIZE: usize = 4;

/// [2.2.2.4] Server Platform Challenge (SERVER_PLATFORM_CHALLENGE)
///
/// [2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/41e129ad-0f35-43ad-a399-1b10e7d007a9
#[derive(Debug, PartialEq, Eq)]
pub struct ServerPlatformChallenge {
    pub license_header: LicenseHeader,
    pub encrypted_platform_challenge: Vec<u8>,
    pub mac_data: Vec<u8>,
}

impl ServerPlatformChallenge {
    const NAME: &'static str = "ServerPlatformChallenge";

    const FIXED_PART_SIZE: usize = CONNECT_FLAGS_FIELD_SIZE + MAC_SIZE + BLOB_LENGTH_SIZE + BLOB_TYPE_SIZE;
}

impl PduEncode for ServerPlatformChallenge {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.license_header.encode(dst)?;
        dst.write_u32(0); // connect_flags, ignored
        BlobHeader::new(BlobType::ANY, self.encrypted_platform_challenge.len()).encode(dst)?;
        dst.write_slice(&self.encrypted_platform_challenge);
        dst.write_slice(&self.mac_data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.license_header.size() + self.encrypted_platform_challenge.len()
    }
}

impl<'de> PduDecode<'de> for ServerPlatformChallenge {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let license_header = read_license_header(PreambleType::PlatformChallenge, src)?;

        ensure_size!(in: src, size: 4);
        let _connect_flags = src.read_u32();
        let blob_header = BlobHeader::decode(src)?;
        ensure_size!(in: src, size: blob_header.length);
        let encrypted_platform_challenge = src.read_slice(blob_header.length).into();
        ensure_size!(in: src, size: MAC_SIZE);
        let mac_data = src.read_slice(MAC_SIZE).into();

        Ok(Self {
            license_header,
            encrypted_platform_challenge,
            mac_data,
        })
    }
}

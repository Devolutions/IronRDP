//! Telemetry virtual channel extension PDUs [MS-RDPET][1] implementation.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/f2fd580b-75bf-42aa-9952-ae8d9ac8f658

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

/// [2.2.1] RDP_TELEMETRY_PDU
///
/// Connection milestones, each in milliseconds since the client initiated the connection.
///
/// [2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpet/47ee7bca-f270-40ac-bde2-8ceed6093624
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TelemetryPdu {
    /// When a credentials prompt was shown, or zero if none was.
    pub prompt_for_credentials_ms: u32,
    /// When the user supplied credentials, or zero if no credentials prompt was shown.
    pub prompt_for_credentials_done_ms: u32,
    /// When the client accepted the MS-RDPEGFX graphics pipeline dynamic virtual channel.
    pub graphics_channel_opened_ms: u32,
    /// When the client received the first MS-RDPEGFX message.
    pub first_graphics_received_ms: u32,
}

impl TelemetryPdu {
    const NAME: &'static str = "RDP_TELEMETRY_PDU";

    const ID: u8 = 0x01;

    const FIXED_PART_SIZE: usize = 1 /* Id */
        + 1 /* Length */
        + 4 /* PromptForCredentialsMillis */
        + 4 /* PromptForCredentialsDoneMillis */
        + 4 /* GraphicsChannelOpenedMillis */
        + 4 /* FirstGraphicsReceivedMillis */;

    /// The Length field value: the PDU's total size.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "asserted to fit the one-byte field"
    )]
    const LENGTH: u8 = {
        assert!(Self::FIXED_PART_SIZE <= u8::MAX as usize);
        Self::FIXED_PART_SIZE as u8
    };
}

impl Encode for TelemetryPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u8(Self::ID);
        dst.write_u8(Self::LENGTH);
        dst.write_u32(self.prompt_for_credentials_ms);
        dst.write_u32(self.prompt_for_credentials_done_ms);
        dst.write_u32(self.graphics_channel_opened_ms);
        dst.write_u32(self.first_graphics_received_ms);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for TelemetryPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let id = src.read_u8();
        if id != Self::ID {
            return Err(invalid_field_err!("Id", "unexpected telemetry PDU identifier", in: src));
        }

        let length = src.read_u8();
        if length != Self::LENGTH {
            return Err(invalid_field_err!("Length", "unexpected telemetry PDU length", in: src));
        }

        Ok(Self {
            prompt_for_credentials_ms: src.read_u32(),
            prompt_for_credentials_done_ms: src.read_u32(),
            graphics_channel_opened_ms: src.read_u32(),
            first_graphics_received_ms: src.read_u32(),
        })
    }
}

impl DvcEncode for TelemetryPdu {}

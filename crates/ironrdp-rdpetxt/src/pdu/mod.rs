//! Text Input Virtual Channel Extension PDUs defined by [MS-RDPETXT].
//!
//! Every PDU starts with a 6-byte header ([2.2.2] PDUs): a 4-byte **size** that counts the
//! 2-byte **pduId** and the body, then the **pduId**.
//! [`ServerPdu`] holds the PDUs the server sends on [`SERVER_TO_CLIENT_CHANNEL_NAME`], and
//! [`ClientPdu`] the ones the client sends on [`CLIENT_TO_SERVER_CHANNEL_NAME`].
//!
//! Decoding ignores body bytes past the fields this crate knows, because later minor versions
//! append fields ([1.7] Versioning and Capability Negotiation).
//! Fields added inside a structure are on the wire only when both peers support them, so
//! [`ServerPdu`] and [`ClientPdu`] encode and decode under a [`PduLayout`].
//!
//! [MS-RDPETXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
//! [2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
//! [1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
//! [`SERVER_TO_CLIENT_CHANNEL_NAME`]: crate::SERVER_TO_CLIENT_CHANNEL_NAME
//! [`CLIENT_TO_SERVER_CHANNEL_NAME`]: crate::CLIENT_TO_SERVER_CHANNEL_NAME

mod client;
mod server;
mod types;

use ironrdp_core::{DecodeResult, ReadCursor, cast_length, ensure_size, invalid_field_err};
use ironrdp_str::prefixed::{CchU32, NoNull, PrefixedString};

pub use self::client::*;
pub use self::server::*;
pub use self::types::*;

/// A UTF-16 string preceded by its length in code units, without a terminator.
pub type TextString = PrefixedString<CchU32, NoNull>;

/// A GUID in its 16-byte wire form ([MS-DTYP] section 2.3.4.3).
///
/// [MS-DTYP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-dtyp/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Guid(pub [u8; 16]);

/// Version (major, minor bit) that introduced **bcpTag** of [`CoreInputProfile`].
const BCP_TAG_VERSION: (u32, u32) = (1, 0x0000_0002);

/// Whether `version`, as (major, minor) announced by a peer, includes the update introduced by
/// `update` ([1.7] Versioning and Capability Negotiation).
///
/// A higher major version includes every update of the lower ones.
///
/// [1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
const fn has_update(version: (u32, u32), update: (u32, u32)) -> bool {
    version.0 > update.0 || (version.0 == update.0 && version.1 & update.1 != 0)
}

/// Fields that only some protocol versions carry.
///
/// A field is on the wire only when both peers announced the version that introduced it
/// ([1.7] Versioning and Capability Negotiation).
/// [`Encode`] and [`Decode`] use [`PduLayout::FULL`].
///
/// [1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
/// [`Encode`]: ironrdp_core::Encode
/// [`Decode`]: ironrdp_core::Decode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PduLayout {
    /// **bcpTag** of [`CoreInputProfile`], introduced by version 1, minor bit 0x00000002.
    pub bcp_tag: bool,
}

impl PduLayout {
    /// The layout of version 1 without updates.
    pub const BASE: Self = Self { bcp_tag: false };

    /// Every field this crate knows.
    pub const FULL: Self = Self { bcp_tag: true };

    /// The fields both peers support, given the (major, minor) version each announced.
    pub const fn negotiated(local: (u32, u32), peer: (u32, u32)) -> Self {
        Self {
            bcp_tag: has_update(local, BCP_TAG_VERSION) && has_update(peer, BCP_TAG_VERSION),
        }
    }
}

/// A PDU with the layout to encode it under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedPdu<P> {
    pub pdu: P,
    pub layout: PduLayout,
}

/// Size of the PDU header.
const HEADER_SIZE: usize = 4 /* size */ + 2 /* pduId */;

/// Reads one PDU from `src` and returns its **pduId** and body.
///
/// The body spans exactly what **size** announces, so a caller can move past PDUs it cannot
/// decode.
pub(crate) fn read_frame<'a>(src: &mut ReadCursor<'a>) -> DecodeResult<(u16, &'a [u8])> {
    ensure_size!(in: src, size: HEADER_SIZE);
    let size = usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!("size", "too large"))?;
    let Some(body_size) = size.checked_sub(2 /* pduId */) else {
        return Err(invalid_field_err!("size", "smaller than the PDU identifier", in: src));
    };
    let pdu_id = src.read_u16();
    ensure_size!(in: src, size: body_size);
    Ok((pdu_id, src.read_slice(body_size)))
}

/// Value of the **size** field for a body of `body_size` bytes.
fn header_size_field(body_size: usize) -> ironrdp_core::EncodeResult<u32> {
    cast_length!("size", 2 /* pduId */ + body_size)
}

/// Declares an enumeration of PDUs that share the [2.2.2] header.
macro_rules! pdu_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident($pdu:ident) = $id:literal,
            )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant($pdu),
            )*
        }

        impl $name {
            /// The **pduId** of this PDU.
            pub const fn pdu_id(&self) -> u16 {
                match self {
                    $(Self::$variant(_) => $id,)*
                }
            }

            /// Whether `pdu_id` identifies one of these PDUs.
            pub const fn is_known_pdu_id(pdu_id: u16) -> bool {
                matches!(pdu_id, $($id)|*)
            }

            /// Size of this PDU under `layout`.
            pub fn size_with(&self, layout: PduLayout) -> usize {
                $crate::pdu::HEADER_SIZE + self.body_size(layout)
            }

            /// Encodes this PDU under `layout`.
            pub fn encode_with(
                &self,
                dst: &mut ironrdp_core::WriteCursor<'_>,
                layout: PduLayout,
            ) -> ironrdp_core::EncodeResult<()> {
                ensure_size!(in: dst, size: self.size_with(layout));
                dst.write_u32($crate::pdu::header_size_field(self.body_size(layout))?);
                dst.write_u16(self.pdu_id());
                match self {
                    $(Self::$variant(pdu) => $crate::wire::Wire::write(pdu, dst, layout),)*
                }
            }

            /// Decodes a PDU laid out as `layout`.
            pub fn decode_with(src: &mut ReadCursor<'_>, layout: PduLayout) -> DecodeResult<Self> {
                let (pdu_id, body) = $crate::pdu::read_frame(src)?;
                let mut body = ReadCursor::new(body);
                match pdu_id {
                    $($id => $crate::wire::Wire::read(&mut body, layout).map(Self::$variant),)*
                    _ => Err(invalid_field_err!("pduId", "unsupported PDU identifier", in: src)),
                }
            }

            fn body_size(&self, layout: PduLayout) -> usize {
                match self {
                    $(Self::$variant(pdu) => $crate::wire::Wire::wire_size(pdu, layout),)*
                }
            }
        }

        $(
            impl From<$pdu> for $name {
                fn from(pdu: $pdu) -> Self {
                    Self::$variant(pdu)
                }
            }
        )*

        impl ironrdp_core::Encode for $name {
            fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
                self.encode_with(dst, PduLayout::FULL)
            }

            fn name(&self) -> &'static str {
                stringify!($name)
            }

            fn size(&self) -> usize {
                self.size_with(PduLayout::FULL)
            }
        }

        impl<'de> ironrdp_core::Decode<'de> for $name {
            fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
                Self::decode_with(src, PduLayout::FULL)
            }
        }

        impl ironrdp_core::Encode for VersionedPdu<$name> {
            fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
                self.pdu.encode_with(dst, self.layout)
            }

            fn name(&self) -> &'static str {
                stringify!($name)
            }

            fn size(&self) -> usize {
                self.pdu.size_with(self.layout)
            }
        }

        #[cfg(feature = "std")]
        impl ironrdp_dvc::DvcEncode for $name {}

        #[cfg(feature = "std")]
        impl ironrdp_dvc::DvcEncode for VersionedPdu<$name> {}
    };
}

pdu_enum! {
    /// A PDU sent from server to client on [`SERVER_TO_CLIENT_CHANNEL_NAME`].
    ///
    /// [`SERVER_TO_CLIENT_CHANNEL_NAME`]: crate::SERVER_TO_CLIENT_CHANNEL_NAME
    pub enum ServerPdu {
        RegisterRemoteTextTarget(RegisterRemoteTextTargetPdu) = 0x0300,
        RegisterRemoteKeyTarget(RegisterRemoteKeyTargetPdu) = 0x0301,
        RegisterRemoteEditControl(RegisterRemoteEditControlPdu) = 0x0302,
        RegisterRemoteCoreInputView(RegisterRemoteCoreInputViewPdu) = 0x0303,
        UnregisterRemoteTextTarget(UnregisterRemoteTextTargetPdu) = 0x0304,
        UnregisterRemoteKeyTarget(UnregisterRemoteKeyTargetPdu) = 0x0305,
        UnregisterRemoteEditControl(UnregisterRemoteEditControlPdu) = 0x0306,
        UnregisterRemoteCoreInputView(UnregisterRemoteCoreInputViewPdu) = 0x0307,
        EditControlFocus(EditControlFocusPdu) = 0x0308,
        HostFocus(HostFocusPdu) = 0x0309,
        HostForeground(HostForegroundPdu) = 0x030A,
        SelectionChanged(SelectionChangedPdu) = 0x030B,
        TextChanged(TextChangedPdu) = 0x030C,
        ControlConfigurationUpdated(ControlConfigurationUpdatedPdu) = 0x030D,
        ControlConversionModeUpdated(ControlConversionModeUpdatedPdu) = 0x030E,
        GeometryChanged(GeometryChangedPdu) = 0x030F,
        SoftwareKeyboardInvocationSignals(SoftwareKeyboardInvocationSignalsPdu) = 0x0310,
        ActiveViewChanged(ActiveViewChangedPdu) = 0x0311,
        AcknowledgeRemoteOperation(AcknowledgeRemoteOperationPdu) = 0x0312,
        AcknowledgeKeyEvent(AcknowledgeKeyEventPdu) = 0x0313,
        InputProfileChanged(InputProfileChangedPdu) = 0x0314,
        ViewOcclusionsHandled(ViewOcclusionsHandledPdu) = 0x0315,
        DoReconversion(DoReconversionPdu) = 0x0316,
        SoftwareKeyboardVisibility(SoftwareKeyboardVisibilityPdu) = 0x0317,
        HotKeyRegistration(HotKeyRegistrationPdu) = 0x0318,
        EditControlTextSegment(EditControlTextSegmentPdu) = 0x0319,
        NotifyServerVersion(NotifyServerVersionPdu) = 0x031A,
        CompositionTerminated(CompositionTerminatedPdu) = 0x0320,
        SoftwareKeyboardPolicy(SoftwareKeyboardPolicyPdu) = 0x0321,
        RemoteTextTargetThreadProperties(RemoteTextTargetThreadPropertiesPdu) = 0x0322,
        ForegroundHostInfoUpdated(ForegroundHostInfoUpdatedPdu) = 0x0500,
        UndoPendingKeyEvents(UndoPendingKeyEventsPdu) = 0x0501,
        RefreshClient(RefreshClientPdu) = 0x0502,
        SendKeyToHost(SendKeyToHostPdu) = 0x0503,
    }
}

pdu_enum! {
    /// A PDU sent from client to server on [`CLIENT_TO_SERVER_CHANNEL_NAME`].
    ///
    /// [`CLIENT_TO_SERVER_CHANNEL_NAME`]: crate::CLIENT_TO_SERVER_CHANNEL_NAME
    pub enum ClientPdu {
        KeyEvent(KeyEventPdu) = 0x0100,
        AcknowledgeHostOperation(AcknowledgeHostOperationPdu) = 0x0101,
        CharacterEvent(CharacterEventPdu) = 0x0102,
        FocusNavigatingEvent(FocusNavigatingEventPdu) = 0x0103,
        FocusDepartCompleted(FocusDepartCompletedPdu) = 0x0104,
        EnableWindow(EnableWindowPdu) = 0x0105,
        ActivationStateChange(ActivationStateChangePdu) = 0x0106,
        NonComponentUiConfigurationChange(NonComponentUiConfigurationChangePdu) = 0x0107,
        KeyEventPayload(KeyEventPayloadPdu) = 0x0108,
        UpdateText(UpdateTextPdu) = 0x0200,
        UpdateTextAndSelection(UpdateTextAndSelectionPdu) = 0x0201,
        SetSelection(SetSelectionPdu) = 0x0202,
        UpdateFormat(UpdateFormatPdu) = 0x0203,
        UpdateComposition(UpdateCompositionPdu) = 0x0204,
        SetCompositionInfo(SetCompositionInfoPdu) = 0x0205,
        ReconversionCandidates(ReconversionCandidatesPdu) = 0x0206,
        UpdateInputLocale(UpdateInputLocalePdu) = 0x0207,
        UpdateInputProfile(UpdateInputProfilePdu) = 0x0208,
        UpdateMode(UpdateModePdu) = 0x0209,
        SetConversionMode(SetConversionModePdu) = 0x020A,
        AcknowledgeOperation(AcknowledgeOperationPdu) = 0x020B,
        ErrorReport(ErrorReportPdu) = 0x020C,
        OccludingViews(OccludingViewsPdu) = 0x0400,
        SetEnabledInputProfiles(SetEnabledInputProfilesPdu) = 0x0600,
        AcknowledgeUndoPendingKeyEvents(AcknowledgeUndoPendingKeyEventsPdu) = 0x0601,
        RemoteIntegrationStatus(RemoteIntegrationStatusPdu) = 0x0602,
        ReregistrationRequest(ReregistrationRequestPdu) = 0x0603,
        NotifyClientVersion(NotifyClientVersionPdu) = 0x0604,
        ReportClientOptions(ReportClientOptionsPdu) = 0x0605,
    }
}

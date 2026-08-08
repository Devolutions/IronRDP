//! Auto-Detect Request and Response PDU types.
//!
//! Implements Connect-Time and Continuous network characteristics detection
//! per [\[MS-RDPBCGR\] 2.2.14].
//!
//! The server sends request PDUs to measure round-trip time and bandwidth.
//! The client responds with measured results. During connect-time, the server
//! sends random payload data (BW\_PAYLOAD) for bandwidth measurement. During
//! continuous detection, actual PDU traffic between BW\_START and BW\_STOP
//! replaces the payload messages.
//!
//! [\[MS-RDPBCGR\] 2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dc672839-4f4e-40b1-a71c-cd6a959baa38

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};

use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};

// ============================================================================
// Constants
// ============================================================================

/// Auto-Detect Request (server to client).
///
/// [\[MS-RDPBCGR\] 2.2.14.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/5a53eadd-64a2-430d-b197-56bdf7ac9ee9
pub const TYPE_ID_AUTODETECT_REQUEST: u8 = 0x00;

/// Auto-Detect Response (client to server).
///
/// [\[MS-RDPBCGR\] 2.2.14.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/9deccc61-ccef-48ed-bfc3-7ad44e2af274
pub const TYPE_ID_AUTODETECT_RESPONSE: u8 = 0x01;

/// Minimum header size shared by all autodetect PDUs.
const HEADER_MIN_SIZE: usize = 1 /* headerLength */
    + 1 /* headerTypeId */
    + 2 /* sequenceNumber */
    + 2 /* requestType or responseType */;

// ============================================================================
// Request Type Codes
// ============================================================================

/// RTT Measure Request during connect-time auto-detection.
///
/// [\[MS-RDPBCGR\] 2.2.14.1.1]
///
/// [\[MS-RDPBCGR\] 2.2.14.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/33b5dd38-a7c3-43d5-a717-ded2391ed599
pub const RTT_REQUEST_CONNECT_TIME: u16 = 0x1001;

/// RTT Measure Request during continuous auto-detection.
pub const RTT_REQUEST_CONTINUOUS: u16 = 0x0001;

/// Bandwidth Measure Start during connect-time auto-detection.
///
/// [\[MS-RDPBCGR\] 2.2.14.1.2]
///
/// [\[MS-RDPBCGR\] 2.2.14.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/1429c9e6-3e33-462b-b0d9-7dbff7faf979
pub const BW_START_CONNECT_TIME: u16 = 0x1014;

/// Bandwidth Measure Start for continuous detection over reliable UDP or TCP.
pub const BW_START_RELIABLE_UDP: u16 = 0x0014;

/// Bandwidth Measure Start for continuous detection over lossy UDP.
pub const BW_START_LOSSY_UDP: u16 = 0x0114;

/// Bandwidth Measure Payload (connect-time only).
///
/// [\[MS-RDPBCGR\] 2.2.14.1.3]
///
/// [\[MS-RDPBCGR\] 2.2.14.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6fe95264-b083-4548-822a-729cfffd9f1c
pub const BW_PAYLOAD: u16 = 0x0002;

/// Bandwidth Measure Stop during connect-time auto-detection (includes payload).
///
/// [\[MS-RDPBCGR\] 2.2.14.1.4]
///
/// [\[MS-RDPBCGR\] 2.2.14.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/515150db-4e7a-4c9b-88d8-63f9fe79981f
pub const BW_STOP_CONNECT_TIME: u16 = 0x002B;

/// Bandwidth Measure Stop for continuous detection over reliable UDP or TCP.
pub const BW_STOP_RELIABLE_UDP: u16 = 0x0429;

/// Bandwidth Measure Stop for continuous detection over lossy UDP.
pub const BW_STOP_LOSSY_UDP: u16 = 0x0629;

/// Which optional fields a Network Characteristics Result carries, as
/// `(baseRTT, bandwidth)`.
///
/// [MS-RDPBCGR] 2.2.14.1.5 assigns these by `requestType` alone, so encode,
/// `size` and decode all consult this rather than inspecting the values.
fn netchar_fields(request_type: u16) -> (bool, bool) {
    match request_type {
        NETCHAR_RESULT_RTT => (true, false),
        NETCHAR_RESULT_BW_RTT => (false, true),
        NETCHAR_RESULT_ALL => (true, true),
        _ => (false, false),
    }
}

/// Network Characteristics Result: baseRTT + averageRTT (no bandwidth).
///
/// [\[MS-RDPBCGR\] 2.2.14.1.5]
///
/// [\[MS-RDPBCGR\] 2.2.14.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/228ffc5c-b60c-4d3e-9781-ac613f822fdf
pub const NETCHAR_RESULT_RTT: u16 = 0x0840;

/// Network Characteristics Result: bandwidth + averageRTT (no baseRTT).
pub const NETCHAR_RESULT_BW_RTT: u16 = 0x0880;

/// Network Characteristics Result: all three fields (baseRTT + bandwidth + averageRTT).
pub const NETCHAR_RESULT_ALL: u16 = 0x08C0;

// ============================================================================
// Response Type Codes
// ============================================================================

/// RTT Measure Response.
///
/// [\[MS-RDPBCGR\] 2.2.14.2.1]
///
/// [\[MS-RDPBCGR\] 2.2.14.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/841649b2-de9d-4143-b91c-d81d7d02e269
pub const RTT_RESPONSE: u16 = 0x0000;

/// Bandwidth Measure Results during connect-time auto-detection.
///
/// [\[MS-RDPBCGR\] 2.2.14.2.2]
///
/// [\[MS-RDPBCGR\] 2.2.14.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6999bd6a-7eb2-4fba-9e5a-c932596056bf
pub const BW_RESULTS_CONNECT_TIME: u16 = 0x0003;

/// Bandwidth Measure Results during continuous detection or over tunnel.
pub const BW_RESULTS_CONTINUOUS: u16 = 0x000B;

/// Network Characteristics Sync (auto-reconnect shortcut).
///
/// [\[MS-RDPBCGR\] 2.2.14.2.3]
///
/// [\[MS-RDPBCGR\] 2.2.14.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d6c7fe90-13b5-4b19-8288-433927fe4809
pub const NETCHAR_SYNC: u16 = 0x0018;

// ============================================================================
// Server → Client Request PDUs
// ============================================================================

/// Auto-Detect Request from server to client.
///
/// Encapsulates one of five message types, discriminated by `request_type`.
///
/// [\[MS-RDPBCGR\] 2.2.14.1]
///
/// [\[MS-RDPBCGR\] 2.2.14.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/5a53eadd-64a2-430d-b197-56bdf7ac9ee9
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum AutoDetectRequest {
    /// [\[MS-RDPBCGR\] 2.2.14.1.1] RTT Measure Request
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/33b5dd38-a7c3-43d5-a717-ded2391ed599
    RttRequest { sequence_number: u16, request_type: u16 },

    /// [\[MS-RDPBCGR\] 2.2.14.1.2] Bandwidth Measure Start
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/1429c9e6-3e33-462b-b0d9-7dbff7faf979
    BandwidthMeasureStart { sequence_number: u16, request_type: u16 },

    /// [\[MS-RDPBCGR\] 2.2.14.1.3] Bandwidth Measure Payload (connect-time only)
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6fe95264-b083-4548-822a-729cfffd9f1c
    BandwidthMeasurePayload { sequence_number: u16, payload: Vec<u8> },

    /// [\[MS-RDPBCGR\] 2.2.14.1.4] Bandwidth Measure Stop
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/515150db-4e7a-4c9b-88d8-63f9fe79981f
    BandwidthMeasureStop {
        sequence_number: u16,
        request_type: u16,
        /// Optional payload (only when request_type is `BW_STOP_CONNECT_TIME`).
        payload: Option<Vec<u8>>,
    },

    /// [\[MS-RDPBCGR\] 2.2.14.1.5] Network Characteristics Result
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/228ffc5c-b60c-4d3e-9781-ac613f822fdf
    NetworkCharacteristicsResult {
        sequence_number: u16,
        request_type: u16,
        /// Lowest detected RTT in milliseconds (present when request_type is 0x0840 or 0x08C0).
        base_rtt_ms: Option<u32>,
        /// Current bandwidth in kilobits per second (present when request_type is 0x0880 or 0x08C0).
        bandwidth_kbps: Option<u32>,
        /// Current average RTT in milliseconds (always present).
        average_rtt_ms: u32,
    },
}

impl AutoDetectRequest {
    const NAME: &'static str = "AutoDetectRequest";

    /// Construct an RTT Measure Request for connect-time detection.
    pub fn rtt_connect_time(sequence_number: u16) -> Self {
        Self::RttRequest {
            sequence_number,
            request_type: RTT_REQUEST_CONNECT_TIME,
        }
    }

    /// Construct an RTT Measure Request for continuous detection.
    pub fn rtt_continuous(sequence_number: u16) -> Self {
        Self::RttRequest {
            sequence_number,
            request_type: RTT_REQUEST_CONTINUOUS,
        }
    }

    /// Construct a Bandwidth Measure Start for connect-time detection.
    pub fn bw_start_connect_time(sequence_number: u16) -> Self {
        Self::BandwidthMeasureStart {
            sequence_number,
            request_type: BW_START_CONNECT_TIME,
        }
    }

    /// Construct a Bandwidth Measure Start for continuous detection.
    pub fn bw_start_continuous(sequence_number: u16) -> Self {
        Self::BandwidthMeasureStart {
            sequence_number,
            request_type: BW_START_RELIABLE_UDP,
        }
    }

    /// Construct a Bandwidth Measure Payload with random data.
    pub fn bw_payload(sequence_number: u16, payload: Vec<u8>) -> Self {
        Self::BandwidthMeasurePayload {
            sequence_number,
            payload,
        }
    }

    /// Construct a Bandwidth Measure Stop for connect-time detection.
    ///
    /// `payload` must be non-empty: [MS-RDPBCGR] 2.2.14.1.4 requires `payloadLength`
    /// to be greater than zero for this request type, so an empty one has no
    /// conforming encoding and is refused by [`Encode`] rather than written as a
    /// zero length.
    pub fn bw_stop_connect_time(sequence_number: u16, payload: Vec<u8>) -> Self {
        Self::BandwidthMeasureStop {
            sequence_number,
            request_type: BW_STOP_CONNECT_TIME,
            payload: Some(payload),
        }
    }

    /// Construct a Bandwidth Measure Stop for continuous detection.
    pub fn bw_stop_continuous(sequence_number: u16) -> Self {
        Self::BandwidthMeasureStop {
            sequence_number,
            request_type: BW_STOP_RELIABLE_UDP,
            payload: None,
        }
    }

    /// Construct a Network Characteristics Result with all fields.
    pub fn netchar_result(sequence_number: u16, base_rtt_ms: u32, bandwidth_kbps: u32, average_rtt_ms: u32) -> Self {
        Self::NetworkCharacteristicsResult {
            sequence_number,
            request_type: NETCHAR_RESULT_ALL,
            base_rtt_ms: Some(base_rtt_ms),
            bandwidth_kbps: Some(bandwidth_kbps),
            average_rtt_ms,
        }
    }

    /// Construct a Network Characteristics Result carrying RTT only (no bandwidth).
    ///
    /// Emits the `NETCHAR_RESULT_RTT` form (baseRTT + averageRTT) for servers
    /// that measure round-trip time but do not run a bandwidth measurement.
    pub fn netchar_result_rtt(sequence_number: u16, base_rtt_ms: u32, average_rtt_ms: u32) -> Self {
        Self::NetworkCharacteristicsResult {
            sequence_number,
            request_type: NETCHAR_RESULT_RTT,
            base_rtt_ms: Some(base_rtt_ms),
            bandwidth_kbps: None,
            average_rtt_ms,
        }
    }

    /// Get the sequence number of this request.
    pub fn sequence_number(&self) -> u16 {
        match self {
            Self::RttRequest { sequence_number, .. }
            | Self::BandwidthMeasureStart { sequence_number, .. }
            | Self::BandwidthMeasurePayload { sequence_number, .. }
            | Self::BandwidthMeasureStop { sequence_number, .. }
            | Self::NetworkCharacteristicsResult { sequence_number, .. } => *sequence_number,
        }
    }
}

impl Encode for AutoDetectRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            Self::RttRequest {
                sequence_number,
                request_type,
            } => {
                dst.write_u8(0x06); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                dst.write_u16(*sequence_number);
                dst.write_u16(*request_type);
            }

            Self::BandwidthMeasureStart {
                sequence_number,
                request_type,
            } => {
                dst.write_u8(0x06); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                dst.write_u16(*sequence_number);
                dst.write_u16(*request_type);
            }

            Self::BandwidthMeasurePayload {
                sequence_number,
                payload,
            } => {
                dst.write_u8(0x08); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                dst.write_u16(*sequence_number);
                dst.write_u16(BW_PAYLOAD);
                dst.write_u16(u16::try_from(payload.len()).unwrap_or(u16::MAX));
                dst.write_slice(payload);
            }

            Self::BandwidthMeasureStop {
                sequence_number,
                request_type,
                payload,
            } => {
                // Whether the payload fields appear on the wire is decided by
                // `requestType`, not by whether a payload was supplied: MS-RDPBCGR
                // 2.2.14.1.4 puts `payloadLength` and `payload` on the connect-time stop
                // and on no other. Branching on the `Option` let this disagree with the
                // decoder, which reads those fields back for `BW_STOP_CONNECT_TIME` and
                // never for the UDP variants.
                if *request_type == BW_STOP_CONNECT_TIME {
                    // 2.2.14.1.4 does not merely make `payloadLength` present for the
                    // connect-time stop, it requires a value "greater than zero". An
                    // absent or empty payload has no conforming encoding, so it is
                    // refused rather than written as a zero length.
                    let data = payload.as_deref().unwrap_or(&[]);
                    if data.is_empty() {
                        return Err(invalid_field_err!(
                            "payload",
                            "connect-time Bandwidth Measure Stop requires a non-empty payload"
                        ));
                    }
                    dst.write_u8(0x08); // headerLength (with payload)
                    dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                    dst.write_u16(*sequence_number);
                    dst.write_u16(*request_type);
                    dst.write_u16(u16::try_from(data.len()).unwrap_or(u16::MAX));
                    dst.write_slice(data);
                } else {
                    dst.write_u8(0x06); // headerLength (no payload)
                    dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                    dst.write_u16(*sequence_number);
                    dst.write_u16(*request_type);
                }
            }

            Self::NetworkCharacteristicsResult {
                sequence_number,
                request_type,
                base_rtt_ms,
                bandwidth_kbps,
                average_rtt_ms,
            } => {
                let header_len = match request_type {
                    &NETCHAR_RESULT_ALL => 0x12u8,
                    _ => 0x0Eu8,
                };
                dst.write_u8(header_len);
                dst.write_u8(TYPE_ID_AUTODETECT_REQUEST);
                dst.write_u16(*sequence_number);
                dst.write_u16(*request_type);

                // Same rule as the stop above: which fields appear is decided by
                // `requestType`, not by which `Option`s happen to be set. MS-RDPBCGR
                // 2.2.14.1.5 assigns 0x0840 baseRTT + averageRTT, 0x0880 bandwidth +
                // averageRTT, and 0x08C0 all three, and the decoder reads them back on
                // exactly that basis. Branching on the `Option`s let the two disagree:
                // a `NETCHAR_RESULT_RTT` with no `base_rtt_ms` wrote a body the decoder
                // could not read, and one carrying `bandwidth_kbps` instead wrote the
                // bandwidth where the decoder expects baseRTT, corrupting it silently.
                let (want_base_rtt, want_bandwidth) = netchar_fields(*request_type);
                if want_base_rtt {
                    dst.write_u32(
                        base_rtt_ms
                            .ok_or_else(|| invalid_field_err!("baseRTT", "requestType requires a baseRTT value"))?,
                    );
                }
                if want_bandwidth {
                    dst.write_u32(
                        bandwidth_kbps
                            .ok_or_else(|| invalid_field_err!("bandwidth", "requestType requires a bandwidth value"))?,
                    );
                }
                dst.write_u32(*average_rtt_ms);
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::RttRequest { .. } | Self::BandwidthMeasureStart { .. } => HEADER_MIN_SIZE,

            Self::BandwidthMeasurePayload { payload, .. } => {
                HEADER_MIN_SIZE + 2 /* payloadLength */ + payload.len()
            }

            // Mirrors `encode`: the payload fields are keyed off `requestType`.
            Self::BandwidthMeasureStop {
                request_type, payload, ..
            } => {
                if *request_type == BW_STOP_CONNECT_TIME {
                    HEADER_MIN_SIZE + 2 /* payloadLength */ + payload.as_ref().map_or(0, Vec::len)
                } else {
                    HEADER_MIN_SIZE
                }
            }

            // Mirrors `encode`: which fields are present is keyed off `requestType`.
            Self::NetworkCharacteristicsResult { request_type, .. } => {
                let (want_base_rtt, want_bandwidth) = netchar_fields(*request_type);
                HEADER_MIN_SIZE + if want_base_rtt { 4 } else { 0 } + if want_bandwidth { 4 } else { 0 } + 4 /* averageRTT */
            }
        }
    }
}

impl<'de> Decode<'de> for AutoDetectRequest {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_MIN_SIZE);

        // headerLength consumed but not validated — the requestType determines the layout.
        let _header_length = src.read_u8();

        let header_type_id = src.read_u8();

        if header_type_id != TYPE_ID_AUTODETECT_REQUEST {
            return Err(invalid_field_err!(
                "headerTypeId",
                "expected TYPE_ID_AUTODETECT_REQUEST (0x00)"
            ));
        }

        let sequence_number = src.read_u16();
        let request_type = src.read_u16();

        match request_type {
            RTT_REQUEST_CONNECT_TIME | RTT_REQUEST_CONTINUOUS => Ok(Self::RttRequest {
                sequence_number,
                request_type,
            }),

            BW_START_CONNECT_TIME | BW_START_RELIABLE_UDP | BW_START_LOSSY_UDP => Ok(Self::BandwidthMeasureStart {
                sequence_number,
                request_type,
            }),

            BW_PAYLOAD => {
                ensure_size!(in: src, size: 2);
                let payload_length = src.read_u16();
                ensure_size!(in: src, size: usize::from(payload_length));
                let payload = src.read_slice(usize::from(payload_length)).to_vec();
                Ok(Self::BandwidthMeasurePayload {
                    sequence_number,
                    payload,
                })
            }

            BW_STOP_CONNECT_TIME => {
                // Connect-time stop has payloadLength + payload.
                ensure_size!(in: src, size: 2);
                let payload_length = src.read_u16();
                // A zero length does not conform: 2.2.14.1.4 requires a value greater
                // than zero, which is why `encode` refuses to emit one. It is still
                // accepted here, deliberately, because the two directions answer
                // different questions. Emitting asks what we are permitted to put on the
                // wire; accepting asks whether we can act on what a peer already sent.
                // The sequence number and request type are intact, so the Bandwidth
                // Measure Results reply this PDU asks for is fully determined, and
                // rejecting it would drop a measurement a server is blocking on rather
                // than correct anything.
                ensure_size!(in: src, size: usize::from(payload_length));
                let payload = src.read_slice(usize::from(payload_length)).to_vec();
                Ok(Self::BandwidthMeasureStop {
                    sequence_number,
                    request_type,
                    payload: Some(payload),
                })
            }

            BW_STOP_RELIABLE_UDP | BW_STOP_LOSSY_UDP => Ok(Self::BandwidthMeasureStop {
                sequence_number,
                request_type,
                payload: None,
            }),

            NETCHAR_RESULT_RTT => {
                // baseRTT + averageRTT (no bandwidth).
                ensure_size!(in: src, size: 8);
                let base_rtt_ms = src.read_u32();
                let average_rtt_ms = src.read_u32();
                Ok(Self::NetworkCharacteristicsResult {
                    sequence_number,
                    request_type,
                    base_rtt_ms: Some(base_rtt_ms),
                    bandwidth_kbps: None,
                    average_rtt_ms,
                })
            }

            NETCHAR_RESULT_BW_RTT => {
                // bandwidth + averageRTT (no baseRTT).
                ensure_size!(in: src, size: 8);
                let bandwidth_kbps = src.read_u32();
                let average_rtt_ms = src.read_u32();
                Ok(Self::NetworkCharacteristicsResult {
                    sequence_number,
                    request_type,
                    base_rtt_ms: None,
                    bandwidth_kbps: Some(bandwidth_kbps),
                    average_rtt_ms,
                })
            }

            NETCHAR_RESULT_ALL => {
                // baseRTT + bandwidth + averageRTT.
                ensure_size!(in: src, size: 12);
                let base_rtt_ms = src.read_u32();
                let bandwidth_kbps = src.read_u32();
                let average_rtt_ms = src.read_u32();
                Ok(Self::NetworkCharacteristicsResult {
                    sequence_number,
                    request_type,
                    base_rtt_ms: Some(base_rtt_ms),
                    bandwidth_kbps: Some(bandwidth_kbps),
                    average_rtt_ms,
                })
            }

            _ => Err(invalid_field_err!("requestType", "unknown autodetect request type")),
        }
    }
}

// ============================================================================
// Client → Server Response PDUs
// ============================================================================

/// Auto-Detect Response from client to server.
///
/// Encapsulates one of three message types, discriminated by `response_type`.
///
/// [\[MS-RDPBCGR\] 2.2.14.2]
///
/// [\[MS-RDPBCGR\] 2.2.14.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/fd28dcb8-671d-48bf-8a98-18be46785dab
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub enum AutoDetectResponse {
    /// [\[MS-RDPBCGR\] 2.2.14.2.1] RTT Measure Response
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/841649b2-de9d-4143-b91c-d81d7d02e269
    RttResponse { sequence_number: u16 },

    /// [\[MS-RDPBCGR\] 2.2.14.2.2] Bandwidth Measure Results
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/6999bd6a-7eb2-4fba-9e5a-c932596056bf
    BandwidthMeasureResults {
        sequence_number: u16,
        response_type: u16,
        /// Time delta between BW_START and BW_STOP receipt, in milliseconds.
        time_delta_ms: u32,
        /// Total bytes received between BW_START and BW_STOP.
        byte_count: u32,
    },

    /// [\[MS-RDPBCGR\] 2.2.14.2.3] Network Characteristics Sync (auto-reconnect shortcut)
    ///
    /// [\[MS-RDPBCGR\] 2.2.14.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d6c7fe90-13b5-4b19-8288-433927fe4809
    NetworkCharacteristicsSync {
        sequence_number: u16,
        /// Previously detected bandwidth in kilobits per second.
        bandwidth_kbps: u32,
        /// Previously detected RTT in milliseconds.
        rtt_ms: u32,
    },
}

impl AutoDetectResponse {
    const NAME: &'static str = "AutoDetectResponse";

    /// Get the sequence number of this response.
    pub fn sequence_number(&self) -> u16 {
        match self {
            Self::RttResponse { sequence_number }
            | Self::BandwidthMeasureResults { sequence_number, .. }
            | Self::NetworkCharacteristicsSync { sequence_number, .. } => *sequence_number,
        }
    }

    /// Compute bandwidth from BandwidthMeasureResults.
    ///
    /// Returns bandwidth in kilobits per second, or None if this is not
    /// a BandwidthMeasureResults variant or timeDelta is zero.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "bandwidth in kbps fits in u32 for any realistic network (max ~4 Tbps)"
    )]
    pub fn computed_bandwidth_kbps(&self) -> Option<u32> {
        match self {
            Self::BandwidthMeasureResults {
                time_delta_ms,
                byte_count,
                ..
            } => {
                if *time_delta_ms == 0 {
                    return None;
                }
                // bandwidth_kbps = (byte_count * 8) / time_delta_ms.
                let kbps = u64::from(*byte_count) * 8 / u64::from(*time_delta_ms);
                Some(kbps as u32)
            }
            _ => None,
        }
    }
}

impl Encode for AutoDetectResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            Self::RttResponse { sequence_number } => {
                dst.write_u8(0x06); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_RESPONSE);
                dst.write_u16(*sequence_number);
                dst.write_u16(RTT_RESPONSE);
            }

            Self::BandwidthMeasureResults {
                sequence_number,
                response_type,
                time_delta_ms,
                byte_count,
            } => {
                dst.write_u8(0x0E); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_RESPONSE);
                dst.write_u16(*sequence_number);
                dst.write_u16(*response_type);
                dst.write_u32(*time_delta_ms);
                dst.write_u32(*byte_count);
            }

            Self::NetworkCharacteristicsSync {
                sequence_number,
                bandwidth_kbps,
                rtt_ms,
            } => {
                dst.write_u8(0x0E); // headerLength
                dst.write_u8(TYPE_ID_AUTODETECT_RESPONSE);
                dst.write_u16(*sequence_number);
                dst.write_u16(NETCHAR_SYNC);
                dst.write_u32(*bandwidth_kbps);
                dst.write_u32(*rtt_ms);
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::RttResponse { .. } => HEADER_MIN_SIZE,
            Self::BandwidthMeasureResults { .. } | Self::NetworkCharacteristicsSync { .. } => {
                HEADER_MIN_SIZE + 4 /* field1 */ + 4 /* field2 */
            }
        }
    }
}

impl<'de> Decode<'de> for AutoDetectResponse {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_MIN_SIZE);

        // headerLength consumed but not validated — the response_type determines the layout.
        let _header_length = src.read_u8();

        let header_type_id = src.read_u8();

        if header_type_id != TYPE_ID_AUTODETECT_RESPONSE {
            return Err(invalid_field_err!(
                "headerTypeId",
                "expected TYPE_ID_AUTODETECT_RESPONSE (0x01)"
            ));
        }

        let sequence_number = src.read_u16();
        let response_type = src.read_u16();

        match response_type {
            RTT_RESPONSE => Ok(Self::RttResponse { sequence_number }),

            BW_RESULTS_CONNECT_TIME | BW_RESULTS_CONTINUOUS => {
                ensure_size!(in: src, size: 8);
                let time_delta_ms = src.read_u32();
                let byte_count = src.read_u32();
                Ok(Self::BandwidthMeasureResults {
                    sequence_number,
                    response_type,
                    time_delta_ms,
                    byte_count,
                })
            }

            NETCHAR_SYNC => {
                ensure_size!(in: src, size: 8);
                let bandwidth_kbps = src.read_u32();
                let rtt_ms = src.read_u32();
                Ok(Self::NetworkCharacteristicsSync {
                    sequence_number,
                    bandwidth_kbps,
                    rtt_ms,
                })
            }

            _ => Err(invalid_field_err!("responseType", "unknown autodetect response type")),
        }
    }
}

// ============================================================================
// MCS message channel framing
// ============================================================================
//
// Auto-detect is not a Share Data PDU. Per [MS-RDPBCGR] 2.2.14.3 / 2.2.14.4 it
// rides the MCS message channel framed by a Basic Security Header whose
// SEC_AUTODETECT_REQ / SEC_AUTODETECT_RSP flag identifies it, the same dispatch
// mechanism used by multitransport (see `rdp::multitransport`).

/// Server Auto-Detect Request PDU ([MS-RDPBCGR] 2.2.14.3).
///
/// Wraps an [`AutoDetectRequest`] with the `SEC_AUTODETECT_REQ` security header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct AutoDetectReqPdu {
    pub security_header: BasicSecurityHeader,
    pub request: AutoDetectRequest,
}

impl AutoDetectReqPdu {
    const NAME: &'static str = "AutoDetectReqPdu";

    /// Wrap a request with the `SEC_AUTODETECT_REQ` security header.
    pub fn new(request: AutoDetectRequest) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::AUTODETECT_REQ,
            },
            request,
        }
    }
}

impl Encode for AutoDetectReqPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.security_header.encode(dst)?;
        self.request.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        BasicSecurityHeader::FIXED_PART_SIZE + self.request.size()
    }
}

impl<'de> Decode<'de> for AutoDetectReqPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let security_header = BasicSecurityHeader::decode(src)?;

        if !security_header.flags.contains(BasicSecurityHeaderFlags::AUTODETECT_REQ) {
            return Err(invalid_field_err!("securityHeader", "expected SEC_AUTODETECT_REQ flag"));
        }

        let request = AutoDetectRequest::decode(src)?;

        Ok(Self {
            security_header,
            request,
        })
    }
}

/// Client Auto-Detect Response PDU ([MS-RDPBCGR] 2.2.14.4).
///
/// Wraps an [`AutoDetectResponse`] with the `SEC_AUTODETECT_RSP` security header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct AutoDetectRspPdu {
    pub security_header: BasicSecurityHeader,
    pub response: AutoDetectResponse,
}

impl AutoDetectRspPdu {
    const NAME: &'static str = "AutoDetectRspPdu";

    /// Wrap a response with the `SEC_AUTODETECT_RSP` security header.
    pub fn new(response: AutoDetectResponse) -> Self {
        Self {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::AUTODETECT_RSP,
            },
            response,
        }
    }
}

impl Encode for AutoDetectRspPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.security_header.encode(dst)?;
        self.response.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        BasicSecurityHeader::FIXED_PART_SIZE + self.response.size()
    }
}

impl<'de> Decode<'de> for AutoDetectRspPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let security_header = BasicSecurityHeader::decode(src)?;

        if !security_header.flags.contains(BasicSecurityHeaderFlags::AUTODETECT_RSP) {
            return Err(invalid_field_err!("securityHeader", "expected SEC_AUTODETECT_RSP flag"));
        }

        let response = AutoDetectResponse::decode(src)?;

        Ok(Self {
            security_header,
            response,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_pdu_round_trip() {
        let original = AutoDetectReqPdu::new(AutoDetectRequest::RttRequest {
            sequence_number: 7,
            request_type: RTT_REQUEST_CONTINUOUS,
        });
        assert_eq!(original.security_header.flags, BasicSecurityHeaderFlags::AUTODETECT_REQ);

        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<AutoDetectReqPdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn rsp_pdu_round_trip() {
        let original = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number: 7 });
        assert_eq!(original.security_header.flags, BasicSecurityHeaderFlags::AUTODETECT_RSP);

        let encoded = ironrdp_core::encode_vec(&original).unwrap();
        let decoded = ironrdp_core::decode::<AutoDetectRspPdu>(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn req_pdu_rejects_response_flag() {
        // A response-flagged frame must not decode as a request PDU.
        let rsp = AutoDetectRspPdu::new(AutoDetectResponse::RttResponse { sequence_number: 1 });
        let encoded = ironrdp_core::encode_vec(&rsp).unwrap();
        assert!(ironrdp_core::decode::<AutoDetectReqPdu>(&encoded).is_err());
    }

    // ========================================================================
    // Request encoding/decoding tests
    // ========================================================================

    const RTT_REQUEST_WIRE: &[u8] = &[
        0x06, // headerLength
        0x00, // headerTypeId = TYPE_ID_AUTODETECT_REQUEST
        0x01, 0x00, // sequenceNumber = 1
        0x01, 0x10, // requestType = RTT_REQUEST_CONNECT_TIME (0x1001)
    ];

    const BW_START_WIRE: &[u8] = &[
        0x06, // headerLength
        0x00, // headerTypeId = TYPE_ID_AUTODETECT_REQUEST
        0x02, 0x00, // sequenceNumber = 2
        0x14, 0x10, // requestType = BW_START_CONNECT_TIME (0x1014)
    ];

    const BW_PAYLOAD_WIRE: &[u8] = &[
        0x08, // headerLength
        0x00, // headerTypeId
        0x03, 0x00, // sequenceNumber = 3
        0x02, 0x00, // requestType = BW_PAYLOAD (0x0002)
        0x04, 0x00, // payloadLength = 4
        0xAA, 0xBB, 0xCC, 0xDD, // payload
    ];

    const BW_STOP_CONNECT_WIRE: &[u8] = &[
        0x08, // headerLength
        0x00, // headerTypeId
        0x04, 0x00, // sequenceNumber = 4
        0x2B, 0x00, // requestType = BW_STOP_CONNECT_TIME (0x002B)
        0x02, 0x00, // payloadLength = 2
        0xEE, 0xFF, // payload
    ];

    const BW_STOP_CONTINUOUS_WIRE: &[u8] = &[
        0x06, // headerLength
        0x00, // headerTypeId
        0x05, 0x00, // sequenceNumber = 5
        0x29, 0x04, // requestType = BW_STOP_RELIABLE_UDP (0x0429)
    ];

    const NETCHAR_ALL_WIRE: &[u8] = &[
        0x12, // headerLength
        0x00, // headerTypeId
        0x06, 0x00, // sequenceNumber = 6
        0xC0, 0x08, // requestType = NETCHAR_RESULT_ALL (0x08C0)
        0x0A, 0x00, 0x00, 0x00, // baseRTT = 10
        0xE8, 0x03, 0x00, 0x00, // bandwidth = 1000
        0x14, 0x00, 0x00, 0x00, // averageRTT = 20
    ];

    const NETCHAR_RTT_WIRE: &[u8] = &[
        0x0E, // headerLength
        0x00, // headerTypeId
        0x07, 0x00, // sequenceNumber = 7
        0x40, 0x08, // requestType = NETCHAR_RESULT_RTT (0x0840)
        0x08, 0x00, 0x00, 0x00, // baseRTT = 8
        0x12, 0x00, 0x00, 0x00, // averageRTT = 18
    ];

    #[test]
    fn decode_rtt_request() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(RTT_REQUEST_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::RttRequest {
                sequence_number,
                request_type,
            } => {
                assert_eq!(sequence_number, 1);
                assert_eq!(request_type, RTT_REQUEST_CONNECT_TIME);
            }
            other => panic!("expected RttRequest, got {other:?}"),
        }
    }

    #[test]
    fn encode_rtt_request() {
        let pdu = AutoDetectRequest::rtt_connect_time(1);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RTT_REQUEST_WIRE);
    }

    #[test]
    fn decode_bw_start() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(BW_START_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::BandwidthMeasureStart {
                sequence_number,
                request_type,
            } => {
                assert_eq!(sequence_number, 2);
                assert_eq!(request_type, BW_START_CONNECT_TIME);
            }
            other => panic!("expected BandwidthMeasureStart, got {other:?}"),
        }
    }

    #[test]
    fn encode_bw_start() {
        let pdu = AutoDetectRequest::bw_start_connect_time(2);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), BW_START_WIRE);
    }

    #[test]
    fn decode_bw_payload() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(BW_PAYLOAD_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::BandwidthMeasurePayload {
                sequence_number,
                payload,
            } => {
                assert_eq!(sequence_number, 3);
                assert_eq!(payload, vec![0xAA, 0xBB, 0xCC, 0xDD]);
            }
            other => panic!("expected BandwidthMeasurePayload, got {other:?}"),
        }
    }

    #[test]
    fn encode_bw_payload() {
        let pdu = AutoDetectRequest::bw_payload(3, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), BW_PAYLOAD_WIRE);
    }

    #[test]
    fn decode_bw_stop_connect_time() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(BW_STOP_CONNECT_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number,
                request_type,
                payload,
            } => {
                assert_eq!(sequence_number, 4);
                assert_eq!(request_type, BW_STOP_CONNECT_TIME);
                assert_eq!(payload, Some(vec![0xEE, 0xFF]));
            }
            other => panic!("expected BandwidthMeasureStop, got {other:?}"),
        }
    }

    #[test]
    fn encode_bw_stop_connect_time() {
        let pdu = AutoDetectRequest::bw_stop_connect_time(4, vec![0xEE, 0xFF]);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), BW_STOP_CONNECT_WIRE);
    }

    #[test]
    fn decode_bw_stop_continuous() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(BW_STOP_CONTINUOUS_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::BandwidthMeasureStop {
                sequence_number,
                request_type,
                payload,
            } => {
                assert_eq!(sequence_number, 5);
                assert_eq!(request_type, BW_STOP_RELIABLE_UDP);
                assert!(payload.is_none());
            }
            other => panic!("expected BandwidthMeasureStop, got {other:?}"),
        }
    }

    #[test]
    fn encode_bw_stop_continuous() {
        let pdu = AutoDetectRequest::bw_stop_continuous(5);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), BW_STOP_CONTINUOUS_WIRE);
    }

    #[test]
    fn decode_netchar_all() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(NETCHAR_ALL_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::NetworkCharacteristicsResult {
                sequence_number,
                request_type,
                base_rtt_ms,
                bandwidth_kbps,
                average_rtt_ms,
            } => {
                assert_eq!(sequence_number, 6);
                assert_eq!(request_type, NETCHAR_RESULT_ALL);
                assert_eq!(base_rtt_ms, Some(10));
                assert_eq!(bandwidth_kbps, Some(1000));
                assert_eq!(average_rtt_ms, 20);
            }
            other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
        }
    }

    #[test]
    fn encode_netchar_all() {
        let pdu = AutoDetectRequest::netchar_result(6, 10, 1000, 20);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), NETCHAR_ALL_WIRE);
    }

    #[test]
    fn decode_netchar_rtt() {
        let pdu = ironrdp_core::decode::<AutoDetectRequest>(NETCHAR_RTT_WIRE).unwrap();
        match pdu {
            AutoDetectRequest::NetworkCharacteristicsResult {
                sequence_number,
                request_type,
                base_rtt_ms,
                bandwidth_kbps,
                average_rtt_ms,
            } => {
                assert_eq!(sequence_number, 7);
                assert_eq!(request_type, NETCHAR_RESULT_RTT);
                assert_eq!(base_rtt_ms, Some(8));
                assert_eq!(bandwidth_kbps, None);
                assert_eq!(average_rtt_ms, 18);
            }
            other => panic!("expected NetworkCharacteristicsResult, got {other:?}"),
        }
    }

    #[test]
    fn encode_netchar_rtt() {
        let pdu = AutoDetectRequest::netchar_result_rtt(7, 8, 18);
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), NETCHAR_RTT_WIRE);
    }

    #[test]
    fn request_round_trip() {
        let cases = vec![
            AutoDetectRequest::rtt_connect_time(100),
            AutoDetectRequest::rtt_continuous(200),
            AutoDetectRequest::bw_start_connect_time(300),
            AutoDetectRequest::bw_start_continuous(400),
            AutoDetectRequest::bw_payload(500, vec![1, 2, 3, 4, 5]),
            AutoDetectRequest::bw_stop_connect_time(600, vec![0xFF; 10]),
            AutoDetectRequest::bw_stop_continuous(700),
            AutoDetectRequest::netchar_result(800, 5, 50000, 15),
            AutoDetectRequest::netchar_result_rtt(900, 5, 15),
        ];

        for original in cases {
            let encoded = ironrdp_core::encode_vec(&original).unwrap();
            let decoded = ironrdp_core::decode::<AutoDetectRequest>(&encoded).unwrap();
            assert_eq!(decoded, original, "round-trip failed for {original:?}");
        }
    }

    #[test]
    fn request_unknown_type_is_error() {
        let bad_wire: &[u8] = &[0x06, 0x00, 0x01, 0x00, 0xFF, 0xFF];
        assert!(ironrdp_core::decode::<AutoDetectRequest>(bad_wire).is_err());
    }

    #[test]
    fn request_wrong_header_type_is_error() {
        // headerTypeId = 0x01 (response) instead of 0x00 (request).
        let bad_wire: &[u8] = &[0x06, 0x01, 0x01, 0x00, 0x01, 0x00];
        assert!(ironrdp_core::decode::<AutoDetectRequest>(bad_wire).is_err());
    }

    // ========================================================================
    // Response encoding/decoding tests
    // ========================================================================

    const RTT_RESPONSE_WIRE: &[u8] = &[
        0x06, // headerLength
        0x01, // headerTypeId = TYPE_ID_AUTODETECT_RESPONSE
        0x01, 0x00, // sequenceNumber = 1
        0x00, 0x00, // responseType = RTT_RESPONSE
    ];

    const BW_RESULTS_WIRE: &[u8] = &[
        0x0E, // headerLength
        0x01, // headerTypeId
        0x04, 0x00, // sequenceNumber = 4
        0x03, 0x00, // responseType = BW_RESULTS_CONNECT_TIME
        0xE8, 0x03, 0x00, 0x00, // timeDelta = 1000
        0x00, 0x10, 0x00, 0x00, // byteCount = 4096
    ];

    const NETCHAR_SYNC_WIRE: &[u8] = &[
        0x0E, // headerLength
        0x01, // headerTypeId
        0x01, 0x00, // sequenceNumber = 1
        0x18, 0x00, // responseType = NETCHAR_SYNC
        0x88, 0x13, 0x00, 0x00, // bandwidth = 5000 kbps
        0x0F, 0x00, 0x00, 0x00, // rtt = 15 ms
    ];

    #[test]
    fn decode_rtt_response() {
        let pdu = ironrdp_core::decode::<AutoDetectResponse>(RTT_RESPONSE_WIRE).unwrap();
        match pdu {
            AutoDetectResponse::RttResponse { sequence_number } => {
                assert_eq!(sequence_number, 1);
            }
            other => panic!("expected RttResponse, got {other:?}"),
        }
    }

    #[test]
    fn encode_rtt_response() {
        let pdu = AutoDetectResponse::RttResponse { sequence_number: 1 };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), RTT_RESPONSE_WIRE);
    }

    #[test]
    fn decode_bw_results() {
        let pdu = ironrdp_core::decode::<AutoDetectResponse>(BW_RESULTS_WIRE).unwrap();
        match pdu {
            AutoDetectResponse::BandwidthMeasureResults {
                sequence_number,
                response_type,
                time_delta_ms,
                byte_count,
            } => {
                assert_eq!(sequence_number, 4);
                assert_eq!(response_type, BW_RESULTS_CONNECT_TIME);
                assert_eq!(time_delta_ms, 1000);
                assert_eq!(byte_count, 4096);
            }
            other => panic!("expected BandwidthMeasureResults, got {other:?}"),
        }
    }

    #[test]
    fn encode_bw_results() {
        let pdu = AutoDetectResponse::BandwidthMeasureResults {
            sequence_number: 4,
            response_type: BW_RESULTS_CONNECT_TIME,
            time_delta_ms: 1000,
            byte_count: 4096,
        };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), BW_RESULTS_WIRE);
    }

    #[test]
    fn computed_bandwidth() {
        let pdu = AutoDetectResponse::BandwidthMeasureResults {
            sequence_number: 1,
            response_type: BW_RESULTS_CONNECT_TIME,
            time_delta_ms: 1000,
            byte_count: 125_000, // 125KB in 1 second = 1000 kbps
        };
        assert_eq!(pdu.computed_bandwidth_kbps(), Some(1000));
    }

    #[test]
    fn computed_bandwidth_zero_delta() {
        let pdu = AutoDetectResponse::BandwidthMeasureResults {
            sequence_number: 1,
            response_type: BW_RESULTS_CONNECT_TIME,
            time_delta_ms: 0,
            byte_count: 100,
        };
        assert_eq!(pdu.computed_bandwidth_kbps(), None);
    }

    #[test]
    fn decode_netchar_sync() {
        let pdu = ironrdp_core::decode::<AutoDetectResponse>(NETCHAR_SYNC_WIRE).unwrap();
        match pdu {
            AutoDetectResponse::NetworkCharacteristicsSync {
                sequence_number,
                bandwidth_kbps,
                rtt_ms,
            } => {
                assert_eq!(sequence_number, 1);
                assert_eq!(bandwidth_kbps, 5000);
                assert_eq!(rtt_ms, 15);
            }
            other => panic!("expected NetworkCharacteristicsSync, got {other:?}"),
        }
    }

    #[test]
    fn encode_netchar_sync() {
        let pdu = AutoDetectResponse::NetworkCharacteristicsSync {
            sequence_number: 1,
            bandwidth_kbps: 5000,
            rtt_ms: 15,
        };
        let encoded = ironrdp_core::encode_vec(&pdu).unwrap();
        assert_eq!(encoded.as_slice(), NETCHAR_SYNC_WIRE);
    }

    #[test]
    fn response_round_trip() {
        let cases = vec![
            AutoDetectResponse::RttResponse { sequence_number: 42 },
            AutoDetectResponse::BandwidthMeasureResults {
                sequence_number: 100,
                response_type: BW_RESULTS_CONTINUOUS,
                time_delta_ms: 500,
                byte_count: 1_000_000,
            },
            AutoDetectResponse::NetworkCharacteristicsSync {
                sequence_number: 200,
                bandwidth_kbps: 10000,
                rtt_ms: 25,
            },
        ];

        for original in cases {
            let encoded = ironrdp_core::encode_vec(&original).unwrap();
            let decoded = ironrdp_core::decode::<AutoDetectResponse>(&encoded).unwrap();
            assert_eq!(decoded, original, "round-trip failed for {original:?}");
        }
    }

    #[test]
    fn response_unknown_type_is_error() {
        let bad_wire: &[u8] = &[0x06, 0x01, 0x01, 0x00, 0xFF, 0xFF];
        assert!(ironrdp_core::decode::<AutoDetectResponse>(bad_wire).is_err());
    }

    #[test]
    fn response_wrong_header_type_is_error() {
        // headerTypeId = 0x00 (request) instead of 0x01 (response).
        let bad_wire: &[u8] = &[0x06, 0x00, 0x01, 0x00, 0x00, 0x00];
        assert!(ironrdp_core::decode::<AutoDetectResponse>(bad_wire).is_err());
    }

    #[test]
    fn sequence_number_accessor() {
        let req = AutoDetectRequest::rtt_connect_time(42);
        assert_eq!(req.sequence_number(), 42);

        let rsp = AutoDetectResponse::RttResponse { sequence_number: 99 };
        assert_eq!(rsp.sequence_number(), 99);
    }
}

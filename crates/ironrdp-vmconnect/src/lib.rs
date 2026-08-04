#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! Hyper-V VM console front-end: **PCB → TLS → CredSSP → X.224**.
//!
//! ```text
//! stream → send_preconnection_blob → PcbSent → TLS (caller) → connect_front → Upgraded
//! ```
//!
//! The post-CredSSP X.224 request advertises only `HYBRID`. `HYBRID_EX` is unnecessary for
//! VMConnect and would imply an Early User Authorization Result exchange that this ordering does
//! not perform.

use core::time::Duration;

use ironrdp_async::{
    Framed, FramedRead, FramedWrite, NetworkClient, Upgraded, connect_begin, mark_as_upgraded, perform_credssp,
};
use ironrdp_connector::credssp::{CredsspSequence, KerberosConfig};
use ironrdp_connector::{
    ClientConnector, ClientConnectorState, ConnectorError, ConnectorErrorExt as _, ConnectorResult, Credentials,
    ServerName, State as _, custom_err, reason_err,
};
use ironrdp_core::{WriteBuf, encode_vec};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::pcb::{PcbVersion, PreconnectionBlob};
use tracing::{debug, instrument};

/// TCP port a Hyper-V VM console listens on.
pub const PORT: u16 = 2179;

/// Upper bound for transmitting the Preconnection Blob after the TCP connection is established.
///
/// MS-RDPEPS requires the complete preconnection PDU within ten seconds of TCP connection creation.
/// This crate stays runtime-agnostic; async callers should enforce the deadline around
/// [`send_preconnection_blob`] (for example with `tokio::time::timeout`).
pub const PCB_TRANSMIT_DEADLINE: Duration = Duration::from_secs(10);

// CredSSP runs before X.224. The protocol value only drives our CredSSP sequence bookkeeping.
const PRE_X224_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;
const POST_CREDSSP_PROTOCOL: SecurityProtocol = SecurityProtocol::HYBRID;

const ENHANCED_MODE_SUFFIX: &str = ";EnhancedMode=1";

/// Hyper-V VM console connection mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Use the guest's Enhanced Session RDP backend.
    #[default]
    Enhanced,
    /// Use the Hyper-V synthetic video and input console.
    Basic,
}

/// Receipt that the Preconnection Blob was written. Required by [`connect_front`].
#[derive(Debug)]
#[must_use = "pass this to connect_front after TLS"]
#[non_exhaustive]
pub struct PcbSent;

/// Encode a PCB V2 for the selected console mode.
pub fn encode_preconnection_blob(vm_id: &str, mode: Mode) -> ConnectorResult<Vec<u8>> {
    let payload = match mode {
        Mode::Enhanced => format!("{vm_id}{ENHANCED_MODE_SUFFIX}"),
        Mode::Basic => vm_id.to_owned(),
    };
    encode_vec(&PreconnectionBlob {
        id: 0,
        version: PcbVersion::V2,
        v2_payload: Some(payload),
    })
    .map_err(ConnectorError::encode)
}

/// Write the Preconnection Blob on a pre-TLS stream. Returns a [`PcbSent`] for [`connect_front`].
///
/// Callers should bound this write with [`PCB_TRANSMIT_DEADLINE`] from TCP connection creation.
#[instrument(skip_all, fields(%vm_id, ?mode))]
pub async fn send_preconnection_blob<S>(framed: &mut Framed<S>, vm_id: &str, mode: Mode) -> ConnectorResult<PcbSent>
where
    S: FramedWrite,
{
    let bytes = encode_preconnection_blob(vm_id, mode)?;

    debug!(length = bytes.len(), "Send Preconnection Blob");
    framed
        .write_all(&bytes)
        .await
        .map_err(|e| custom_err!("write preconnection blob", e))?;

    Ok(PcbSent)
}

/// After TLS: CredSSP, then X.224. Consumes [`PcbSent`]; returns [`Upgraded`] for
/// [`ironrdp_async::connect_finalize`].
///
/// This path always performs CredSSP and then negotiates HYBRID. A connector that disabled TLS or
/// CredSSP is rejected so X.224 cannot advertise a protocol set that disagrees with the wire
/// sequence.
#[instrument(skip_all)]
pub async fn connect_front<S, N>(
    _pcb_sent: PcbSent,
    framed: &mut Framed<S>,
    connector: &mut ClientConnector,
    network_client: &mut N,
    server_name: ServerName,
    server_public_key: &[u8],
    kerberos_config: Option<KerberosConfig>,
) -> ConnectorResult<Upgraded>
where
    S: Sync + FramedRead + FramedWrite,
    N: NetworkClient,
{
    prepare_connector(connector)?;

    debug!("Begin CredSSP (before X.224)");

    let mut buf = WriteBuf::new();
    let (sequence, ts_request) = CredsspSequence::init(
        connector.config.credentials.clone(),
        connector.config.domain.as_deref(),
        PRE_X224_CREDSSP_PROTOCOL,
        server_name,
        server_public_key.to_owned(),
        kerberos_config,
    )?;
    perform_credssp(framed, network_client, &mut buf, sequence, ts_request).await?;

    // Host authentication is complete. Do not forward its identity or secret into the guest-facing
    // RDP sequence; Enhanced Session guest sign-in is a separate authentication seam. This happens
    // only after successful CredSSP, so a failed authentication leaves the connector reusable.
    connector.config.credentials = Credentials::UsernamePassword {
        username: String::new(),
        password: String::new(),
    };
    connector.config.domain = None;
    connector.config.autologon = false;

    buf.clear();
    connector.initiate_with_security_protocol(POST_CREDSSP_PROTOCOL, &mut buf)?;
    framed
        .write_all(buf.filled())
        .await
        .map_err(|e| custom_err!("write X.224 connection request", e))?;

    let should_upgrade = connect_begin(framed, connector).await?;
    ensure_selected_credssp(&connector.state)?;

    let upgraded = mark_as_upgraded(should_upgrade, connector);
    connector.mark_credssp_as_done();
    Ok(upgraded)
}

/// Require TLS + CredSSP on the connector for the VMConnect front-end.
///
/// CredSSP runs before X.224 and TLS is already up when this path is used. Clearing
/// `enable_tls` / `enable_credssp` would make the later Negotiate Request advertise a protocol
/// set that disagrees with the bytes already exchanged. Every embedder (client, FFI, web) goes
/// through [`connect_front`], so this is the single choke point.
fn prepare_connector(connector: &mut ClientConnector) -> ConnectorResult<()> {
    if !connector.config.enable_tls {
        return Err(reason_err!(
            "vmconnect",
            "TLS is required for a Hyper-V console connection",
        ));
    }
    if !connector.config.enable_credssp {
        return Err(reason_err!(
            "vmconnect",
            "CredSSP is required for a Hyper-V console connection",
        ));
    }
    Ok(())
}

fn ensure_selected_credssp(state: &ClientConnectorState) -> ConnectorResult<()> {
    let selected = match state {
        ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol } => *selected_protocol,
        other => {
            return Err(reason_err!(
                "Initiation",
                "expected EnhancedSecurityUpgrade after Hyper-V X.224 initiation, got {}",
                other.name()
            ));
        }
    };

    if selected == SecurityProtocol::HYBRID {
        Ok(())
    } else {
        Err(reason_err!(
            "Initiation",
            "server must select HYBRID for a Hyper-V console, but it selected {selected}",
        ))
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_connector::{ClientConnector, ClientConnectorState, Credentials};
    use ironrdp_core::{WriteBuf, decode};
    use ironrdp_pdu::nego::{ConnectionRequest, SecurityProtocol};
    use ironrdp_pdu::pcb::PreconnectionBlob;
    use ironrdp_pdu::x224::X224;

    use super::{Mode, encode_preconnection_blob, ensure_selected_credssp, prepare_connector};

    #[test]
    fn pcb_payload_selects_console_mode() {
        const VM_ID: &str = "efd1efab-c750-4262-b1bb-af0f7733bdd6";

        for (mode, expected) in [
            (Mode::Enhanced, format!("{VM_ID};EnhancedMode=1")),
            (Mode::Basic, VM_ID.to_owned()),
        ] {
            let bytes = encode_preconnection_blob(VM_ID, mode).expect("encode");
            let pcb: PreconnectionBlob = decode(&bytes).expect("decode");

            assert_eq!(pcb.v2_payload.as_deref(), Some(expected.as_str()));
        }
    }

    #[test]
    fn accepts_hybrid_selected_after_enhanced_mode_authentication() {
        let state = ClientConnectorState::EnhancedSecurityUpgrade {
            selected_protocol: SecurityProtocol::HYBRID,
        };

        ensure_selected_credssp(&state).expect("CredSSP protocol");
    }

    #[test]
    fn rejects_hybrid_ex_selected_after_enhanced_mode_authentication() {
        let state = ClientConnectorState::EnhancedSecurityUpgrade {
            selected_protocol: SecurityProtocol::HYBRID_EX,
        };

        ensure_selected_credssp(&state).expect_err("HYBRID_EX must be rejected");
    }

    #[test]
    fn pcb_transmit_deadline_matches_ms_rdpeps() {
        assert_eq!(super::PCB_TRANSMIT_DEADLINE, core::time::Duration::from_secs(10));
    }

    fn test_connector(enable_tls: bool, enable_credssp: bool) -> ClientConnector {
        use ironrdp_pdu::gcc;
        use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;

        ClientConnector::new(
            ironrdp_connector::Config {
                desktop_size: ironrdp_connector::DesktopSize {
                    width: 800,
                    height: 600,
                },
                desktop_scale_factor: 0,
                enable_tls,
                enable_credssp,
                credentials: Credentials::UsernamePassword {
                    username: "u".into(),
                    password: "p".into(),
                },
                domain: None,
                client_build: 0,
                client_name: "test".into(),
                keyboard_type: gcc::KeyboardType::IbmEnhanced,
                keyboard_subtype: 0,
                keyboard_layout: 0,
                keyboard_functional_keys_count: 12,
                connection_type: gcc::ConnectionType::Lan,
                ime_file_name: String::new(),
                bitmap: None,
                dig_product_id: String::new(),
                client_dir: String::new(),
                platform: MajorPlatformType::UNSPECIFIED,
                hardware_id: None,
                request_data: None,
                autologon: false,
                enable_audio_playback: false,
                license_cache: None,
                compression_type: None,
                enable_server_pointer: false,
                pointer_software_rendering: false,
                multitransport_flags: None,
                performance_flags: Default::default(),
                timezone_info: Default::default(),
                alternate_shell: String::new(),
                work_dir: String::new(),
            },
            "127.0.0.1:2179".parse().unwrap(),
        )
    }

    #[test]
    fn prepare_connector_accepts_tls_and_credssp() {
        prepare_connector(&mut test_connector(true, true)).expect("required flags set");
    }

    #[test]
    fn prepare_connector_rejects_disabled_tls() {
        let err = prepare_connector(&mut test_connector(false, true)).expect_err("TLS required");
        assert!(err.to_string().contains("TLS"), "{err}");
    }

    #[test]
    fn prepare_connector_rejects_disabled_credssp() {
        let err = prepare_connector(&mut test_connector(true, false)).expect_err("CredSSP required");
        assert!(err.to_string().contains("CredSSP"), "{err}");
    }

    #[test]
    fn prepare_connector_advertises_hybrid_without_hybrid_ex() {
        let mut connector = test_connector(true, true);
        prepare_connector(&mut connector).expect("prepare connector");

        let mut output = WriteBuf::new();
        connector
            .initiate_with_security_protocol(SecurityProtocol::HYBRID, &mut output)
            .expect("encode X.224 request");
        let X224(request): X224<ConnectionRequest> = decode(output.filled()).expect("decode X.224 request");

        assert_eq!(request.protocol, SecurityProtocol::HYBRID);
    }

    #[test]
    fn guest_rdp_sequence_omits_host_username_cookie() {
        let mut connector = test_connector(true, true);
        prepare_connector(&mut connector).expect("prepare connector");
        connector.config.credentials = Credentials::UsernamePassword {
            username: String::new(),
            password: String::new(),
        };
        connector.config.domain = None;
        connector.config.autologon = false;

        let mut output = WriteBuf::new();
        connector
            .initiate_with_security_protocol(SecurityProtocol::HYBRID, &mut output)
            .expect("encode X.224 request");
        let X224(request): X224<ConnectionRequest> = decode(output.filled()).expect("decode X.224 request");

        assert_eq!(request.protocol, SecurityProtocol::HYBRID);
        assert_eq!(request.nego_data, None);
    }
}

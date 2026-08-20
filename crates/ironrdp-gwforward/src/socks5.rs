//! Minimal SOCKS5 server (RFC 1928) for CONNECT, no authentication.
//!
//! Supports the subset a generic client needs to tunnel through the gateway:
//! no-auth method negotiation, CONNECT to an IPv4 address or a domain name, and a
//! success/failure reply. UDP ASSOCIATE and BIND are not supported.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::error::{ForwardError, ForwardErrorKind, Result};

const VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NO_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const REP_SUCCESS: u8 = 0x00;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDRESS_NOT_SUPPORTED: u8 = 0x08;
const REP_HOST_UNREACHABLE: u8 = 0x04;

/// A requested CONNECT destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SocksTarget {
    /// Hostname or dotted-decimal IP address.
    pub(crate) host: String,
    /// Destination port.
    pub(crate) port: u16,
}

/// Run the SOCKS5 negotiation and read the CONNECT request.
///
/// On success, returns the requested target; the caller opens the tunnel and then calls
/// [`write_reply`] to report the outcome before relaying bytes.
pub(crate) async fn negotiate<S>(stream: &mut S) -> Result<SocksTarget>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    read_greeting(stream).await?;
    read_connect_request(stream).await
}

/// Write the SOCKS5 reply for a CONNECT attempt (`success` selects the reply code) and
/// flush it so the client can begin relaying immediately.
pub(crate) async fn write_reply<S>(stream: &mut S, success: bool) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let rep = if success { REP_SUCCESS } else { REP_HOST_UNREACHABLE };
    // BND.ADDR/BND.PORT are reported as 0.0.0.0:0; clients ignore them for CONNECT.
    let reply = [VERSION, rep, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0];
    stream
        .write_all(&reply)
        .await
        .map_err(|e| ForwardError::new("write socks5 reply", ForwardErrorKind::Io).with_source(e))?;
    stream
        .flush()
        .await
        .map_err(|e| ForwardError::new("flush socks5 reply", ForwardErrorKind::Io).with_source(e))?;
    Ok(())
}

/// Write a failure reply for a malformed or unsupported CONNECT request, then yield the
/// error. Used to answer the client before tearing the connection down.
pub(crate) async fn reject<S>(stream: &mut S, error: ForwardError) -> ForwardError
where
    S: AsyncWrite + Unpin,
{
    let rep = match error.kind() {
        ForwardErrorKind::Socks5 => REP_COMMAND_NOT_SUPPORTED,
        _ => REP_ADDRESS_NOT_SUPPORTED,
    };
    let reply = [VERSION, rep, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0];
    let _ = stream.write_all(&reply).await;
    let _ = stream.flush().await;
    error
}

async fn read_greeting<S>(stream: &mut S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = read_u8(stream).await?;
    if version != VERSION {
        return Err(ForwardError::new("socks5 version", ForwardErrorKind::Socks5));
    }
    let method_count = usize::from(read_u8(stream).await?);
    let mut methods = vec![0u8; method_count];
    read_exact(stream, &mut methods).await?;

    let offers_no_auth = methods.contains(&METHOD_NO_AUTH);
    let chosen = if offers_no_auth {
        METHOD_NO_AUTH
    } else {
        METHOD_NO_ACCEPTABLE
    };
    stream
        .write_all(&[VERSION, chosen])
        .await
        .map_err(|e| ForwardError::new("write socks5 greeting", ForwardErrorKind::Io).with_source(e))?;
    stream
        .flush()
        .await
        .map_err(|e| ForwardError::new("flush socks5 greeting", ForwardErrorKind::Io).with_source(e))?;

    if !offers_no_auth {
        return Err(ForwardError::new(
            "client offers no no-auth method",
            ForwardErrorKind::Socks5,
        ));
    }
    Ok(())
}

async fn read_connect_request<S>(stream: &mut S) -> Result<SocksTarget>
where
    S: AsyncRead + Unpin,
{
    let version = read_u8(stream).await?;
    if version != VERSION {
        return Err(ForwardError::new("socks5 request version", ForwardErrorKind::Socks5));
    }
    let command = read_u8(stream).await?;
    if command != CMD_CONNECT {
        return Err(ForwardError::new(
            "socks5 unsupported command",
            ForwardErrorKind::Socks5,
        ));
    }
    let _reserved = read_u8(stream).await?;
    let atyp = read_u8(stream).await?;

    let host = match atyp {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            read_exact(stream, &mut octets).await?;
            octets.map(|b| b.to_string()).join(".")
        }
        ATYP_DOMAIN => {
            let len = usize::from(read_u8(stream).await?);
            let mut bytes = vec![0u8; len];
            read_exact(stream, &mut bytes).await?;
            String::from_utf8(bytes)
                .map_err(|_| ForwardError::new("socks5 domain not utf8", ForwardErrorKind::Socks5))?
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            read_exact(stream, &mut octets).await?;
            let addr = core::net::Ipv6Addr::from(octets);
            addr.to_string()
        }
        _ => {
            return Err(ForwardError::new("socks5 address type", ForwardErrorKind::Socks5));
        }
    };

    let mut port_bytes = [0u8; 2];
    read_exact(stream, &mut port_bytes).await?;
    let port = u16::from_be_bytes(port_bytes);

    Ok(SocksTarget { host, port })
}

async fn read_u8<S>(stream: &mut S) -> Result<u8>
where
    S: AsyncRead + Unpin,
{
    let mut byte = [0u8; 1];
    read_exact(stream, &mut byte).await?;
    Ok(byte[0])
}

async fn read_exact<S>(stream: &mut S, buf: &mut [u8]) -> Result<()>
where
    S: AsyncRead + Unpin,
{
    stream
        .read_exact(buf)
        .await
        .map_err(|e| ForwardError::new("read socks5", ForwardErrorKind::Socks5).with_source(e))?;
    Ok(())
}

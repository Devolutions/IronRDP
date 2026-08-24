//! Minimal SOCKS5 server (RFC 1928) for CONNECT, no authentication.
//!
//! Supports the subset a generic client needs to tunnel through the gateway:
//! no-auth method negotiation, CONNECT to an IPv4 address, an IPv6 address, or a
//! domain name, and a success/failure reply. UDP ASSOCIATE and BIND are not supported.

use anyhow::Context as _;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

const VERSION: u8 = 0x05;
const METHOD_NO_AUTH: u8 = 0x00;
const METHOD_NO_ACCEPTABLE: u8 = 0xFF;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;
const RSV: u8 = 0x00;
pub(crate) const REP_SUCCESS: u8 = 0x00;
pub(crate) const REP_GENERAL_FAILURE: u8 = 0x01;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REP_ADDRESS_NOT_SUPPORTED: u8 = 0x08;

/// A requested CONNECT destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SocksTarget {
    /// Hostname or IP address, without IPv6 brackets.
    pub(crate) host: String,
    /// Destination port.
    pub(crate) port: u16,
}

/// SOCKS5 negotiation failure.
///
/// Greeting failures have already sent `05 FF` and must not be followed by a CONNECT
/// reply. Request failures carry the RFC 1928 reply code to send before closing.
#[derive(Debug)]
pub(crate) enum NegotiateFailure {
    Greeting(anyhow::Error),
    Request { error: anyhow::Error, reply: u8 },
}

/// Run the SOCKS5 negotiation and read the CONNECT request.
///
/// On success, returns the requested target; the caller opens the tunnel and then calls
/// [`write_reply`] to report the outcome before relaying bytes.
pub(crate) async fn negotiate<S>(stream: &mut S) -> Result<SocksTarget, NegotiateFailure>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    read_greeting(stream).await.map_err(NegotiateFailure::Greeting)?;
    read_connect_request(stream).await
}

/// Write the SOCKS5 reply for a CONNECT attempt and flush it so the client can begin
/// relaying immediately.
pub(crate) async fn write_reply<S>(stream: &mut S, rep: u8) -> anyhow::Result<()>
where
    S: AsyncWrite + Unpin,
{
    // BND.ADDR/BND.PORT are reported as 0.0.0.0:0; clients ignore them for CONNECT.
    let reply = [VERSION, rep, RSV, ATYP_IPV4, 0, 0, 0, 0, 0, 0];
    stream.write_all(&reply).await.context("write socks5 reply")?;
    stream.flush().await.context("flush socks5 reply")?;
    Ok(())
}

/// Write a CONNECT failure reply, then yield the original error.
pub(crate) async fn reject_request<S>(stream: &mut S, error: anyhow::Error, reply: u8) -> anyhow::Error
where
    S: AsyncWrite + Unpin,
{
    let _ = write_reply(stream, reply).await;
    error
}

async fn read_greeting<S>(stream: &mut S) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = read_u8(stream).await?;
    if version != VERSION {
        anyhow::bail!("socks5 version");
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
        .context("write socks5 greeting")?;
    stream.flush().await.context("flush socks5 greeting")?;

    if !offers_no_auth {
        anyhow::bail!("client offers no no-auth method");
    }
    Ok(())
}

async fn read_connect_request<S>(stream: &mut S) -> Result<SocksTarget, NegotiateFailure>
where
    S: AsyncRead + Unpin,
{
    let version = read_u8(stream).await.map_err(request_io)?;
    if version != VERSION {
        return Err(request_err("socks5 request version", REP_GENERAL_FAILURE));
    }
    let command = read_u8(stream).await.map_err(request_io)?;
    if command != CMD_CONNECT {
        return Err(request_err("socks5 unsupported command", REP_COMMAND_NOT_SUPPORTED));
    }
    let reserved = read_u8(stream).await.map_err(request_io)?;
    if reserved != RSV {
        return Err(request_err("socks5 reserved byte", REP_GENERAL_FAILURE));
    }
    let atyp = read_u8(stream).await.map_err(request_io)?;

    let host = match atyp {
        ATYP_IPV4 => {
            let mut octets = [0u8; 4];
            read_exact(stream, &mut octets).await.map_err(request_io)?;
            octets.map(|b| b.to_string()).join(".")
        }
        ATYP_DOMAIN => {
            let len = usize::from(read_u8(stream).await.map_err(request_io)?);
            let mut bytes = vec![0u8; len];
            read_exact(stream, &mut bytes).await.map_err(request_io)?;
            String::from_utf8(bytes).map_err(|_| request_err("socks5 domain not utf8", REP_GENERAL_FAILURE))?
        }
        ATYP_IPV6 => {
            let mut octets = [0u8; 16];
            read_exact(stream, &mut octets).await.map_err(request_io)?;
            let addr = core::net::Ipv6Addr::from(octets);
            addr.to_string()
        }
        _ => {
            return Err(request_err("socks5 address type", REP_ADDRESS_NOT_SUPPORTED));
        }
    };

    let mut port_bytes = [0u8; 2];
    read_exact(stream, &mut port_bytes).await.map_err(request_io)?;
    let port = u16::from_be_bytes(port_bytes);

    Ok(SocksTarget { host, port })
}

fn request_err(context: &'static str, reply: u8) -> NegotiateFailure {
    NegotiateFailure::Request {
        error: anyhow::anyhow!(context),
        reply,
    }
}

fn request_io(error: anyhow::Error) -> NegotiateFailure {
    NegotiateFailure::Request {
        error,
        reply: REP_GENERAL_FAILURE,
    }
}

async fn read_u8<S>(stream: &mut S) -> anyhow::Result<u8>
where
    S: AsyncRead + Unpin,
{
    let mut byte = [0u8; 1];
    read_exact(stream, &mut byte).await?;
    Ok(byte[0])
}

async fn read_exact<S>(stream: &mut S, buf: &mut [u8]) -> anyhow::Result<()>
where
    S: AsyncRead + Unpin,
{
    stream.read_exact(buf).await.context("read socks5")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn rejects_greeting_without_no_auth_method() {
        let (mut client, mut server) = duplex(8);
        let client_task = async {
            client.write_all(&[VERSION, 1, 0x02]).await.unwrap();
            let mut reply = [0u8; 2];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply, [VERSION, METHOD_NO_ACCEPTABLE]);
        };

        let (_, result) = tokio::join!(client_task, negotiate(&mut server));
        assert!(matches!(result, Err(NegotiateFailure::Greeting(_))));
    }

    #[tokio::test]
    async fn accepts_ipv6_connect() {
        let target = negotiate_request(&[
            VERSION,
            CMD_CONNECT,
            RSV,
            ATYP_IPV6,
            0x20,
            0x01,
            0x0d,
            0xb8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            0x01,
            0xbb,
        ])
        .await
        .unwrap();

        assert_eq!(
            target,
            SocksTarget {
                host: "2001:db8::1".to_owned(),
                port: 443,
            }
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_command() {
        let error = negotiate_request(&[VERSION, 0x02]).await.unwrap_err();

        assert_request_reply(error, REP_COMMAND_NOT_SUPPORTED);
    }

    #[tokio::test]
    async fn rejects_nonzero_reserved_byte() {
        let error = negotiate_request(&[VERSION, CMD_CONNECT, 1]).await.unwrap_err();

        assert_request_reply(error, REP_GENERAL_FAILURE);
    }

    #[tokio::test]
    async fn rejects_unsupported_address_type() {
        let error = negotiate_request(&[VERSION, CMD_CONNECT, RSV, 0xff]).await.unwrap_err();

        assert_request_reply(error, REP_ADDRESS_NOT_SUPPORTED);
    }

    async fn negotiate_request(request: &[u8]) -> Result<SocksTarget, NegotiateFailure> {
        let (mut client, mut server) = duplex(64);
        let client_task = async {
            client.write_all(&[VERSION, 1, METHOD_NO_AUTH]).await.unwrap();
            let mut reply = [0u8; 2];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply, [VERSION, METHOD_NO_AUTH]);
            client.write_all(request).await.unwrap();
        };

        let (_, result) = tokio::join!(client_task, negotiate(&mut server));
        result
    }

    fn assert_request_reply(error: NegotiateFailure, expected_reply: u8) {
        match error {
            NegotiateFailure::Greeting(_) => panic!("expected request failure"),
            NegotiateFailure::Request { reply, .. } => assert_eq!(reply, expected_reply),
        }
    }
}

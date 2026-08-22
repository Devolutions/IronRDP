use core::time::Duration;
use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpeudp::pdu::*;
use ironrdp_rdpeudp::*;
fn now() -> MonotonicInstant {
    MonotonicInstant::from_millis(0)
}

fn later(base: MonotonicInstant, ms: u64) -> MonotonicInstant {
    base + Duration::from_millis(ms)
}

/// Stand-in for the SHA-256 of a security cookie. Both ends of a test use the
/// same one, which is what a real client and server would also have.
const TEST_COOKIE_HASH: [u8; 32] = [0x5A; 32];

fn default_config(isn: u32) -> ConnectionConfig {
    ConnectionConfig {
        initial_sequence_number: isn,
        log_window_size: 6,
        upstream_mtu: 1232,
        downstream_mtu: 1232,
        idle_timeout: Duration::from_secs(65),
        keep_alive_interval: Duration::from_secs(8),
        cookie_hash: Some(TEST_COOKIE_HASH),
    }
}

/// A client SYN as `connect` builds it, for tests that drive the server side
/// by hand.
fn test_syn_data_ex() -> SynDataExPayload {
    SynDataExPayload {
        syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
        udp_ver: UdpVersion::V3,
        cookie_hash: Some(TEST_COOKIE_HASH),
    }
}

// ── Handshake tests ──

#[test]
fn client_connect_produces_syn() {
    let t = now();
    let mut conn = RdpeudpConnection::connect(default_config(100), t).expect("connect");

    let transmit = conn.poll_transmit(t).expect("should have SYN");
    assert!(!transmit.contents.is_empty());

    // Decode the SYN
    let datagram: V1Datagram = decode(&transmit.contents).expect("valid SYN");
    assert!(datagram.header.flags.contains(V1Flags::SYN));
    assert!(datagram.header.flags.contains(V1Flags::SYNEX));
    assert_eq!(
        datagram
            .syn_data
            .as_ref()
            .expect("has syn_data")
            .initial_sequence_number,
        100
    );

    // No more transmits
    assert!(conn.poll_transmit(t).is_none());

    // Should be in SynSent state
    assert!(!conn.is_established());
    assert!(!conn.is_closed());
}

#[test]
fn server_accept_produces_syn_ack() {
    let t = now();

    // Build a client SYN
    let client_syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(test_syn_data_ex()),
    };

    let mut conn = RdpeudpConnection::accept(default_config(200), &client_syn, t).expect("accept");

    let transmit = conn.poll_transmit(t).expect("should have SYN+ACK");

    // Decode the SYN+ACK
    let datagram: V1Datagram = decode(&transmit.contents).expect("valid SYN+ACK");
    assert!(datagram.header.flags.contains(V1Flags::SYN));
    assert!(datagram.header.flags.contains(V1Flags::ACK));
    assert!(datagram.header.flags.contains(V1Flags::SYNEX));
    assert_eq!(
        datagram
            .syn_data
            .as_ref()
            .expect("has syn_data")
            .initial_sequence_number,
        200
    );
}

#[test]
fn full_handshake_client_server() {
    let t = now();

    // Step 1: Client sends SYN
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn_transmit = client.poll_transmit(t).expect("client SYN");

    // Step 2: Server receives SYN and creates connection with SYN+ACK
    let syn_datagram: V1Datagram = decode(&syn_transmit.contents).expect("decode SYN");
    let mut server = RdpeudpConnection::accept(default_config(200), &syn_datagram, t).expect("accept");
    let syn_ack_transmit = server.poll_transmit(t).expect("server SYN+ACK");

    // Step 3: Client receives SYN+ACK → transitions to Established
    let mut syn_ack_bytes = syn_ack_transmit.contents;
    client
        .handle_datagram(&mut syn_ack_bytes, later(t, 50))
        .expect("handle SYN+ACK");

    assert!(client.is_established());

    // Client should emit Connected event
    let event = client.poll_event().expect("should have event");
    assert_eq!(event, Event::Connected);

    // Client sends final ACK
    let ack_transmit = client.poll_transmit(later(t, 50)).expect("client final ACK");

    // Step 4: Server receives final ACK → transitions to Established
    let mut ack_bytes = ack_transmit.contents;
    server
        .handle_datagram(&mut ack_bytes, later(t, 100))
        .expect("handle final ACK");

    assert!(server.is_established());

    let event = server.poll_event().expect("should have event");
    assert_eq!(event, Event::Connected);
}

#[test]
fn server_rejects_non_v2_syn() {
    let t = now();

    let syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V1,
            cookie_hash: None,
        }),
    };

    let result = RdpeudpConnection::accept(default_config(200), &syn, t);
    assert!(result.is_err());
}

/// MS-RDPEUDP 1.7's negotiate-down MUST clause: a client offering a version
/// above 3, which this crate does not recognize, must still get a V3
/// SYN+ACK rather than a refused connection, since 3 is our own highest (and
/// only) supported version, at or below anything higher that was offered.
#[test]
fn server_accepts_and_settles_on_v3_for_an_unrecognized_higher_version() {
    let t = now();

    let syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion(0x0102),
            cookie_hash: Some(TEST_COOKIE_HASH),
        }),
    };

    let mut conn = RdpeudpConnection::accept(default_config(200), &syn, t).expect("accept");
    let transmit = conn.poll_transmit(t).expect("SYN+ACK");

    let datagram: V1Datagram = decode(&transmit.contents).expect("decode SYN+ACK");
    let settled = datagram.syn_data_ex.expect("has syn_data_ex").udp_ver;
    assert_eq!(settled, UdpVersion::V3);
}

#[test]
fn send_before_established_returns_error() {
    let t = now();
    let mut conn = RdpeudpConnection::connect(default_config(100), t).expect("connect");

    let result = conn.send(vec![0xAA; 100]);
    assert!(matches!(
        result.unwrap_err().error.kind(),
        RdpeudpErrorKind::InvalidState
    ));
}

/// `send` must eventually refuse more data rather than growing the buffer
/// without limit when nothing is draining it, under sustained congestion or
/// an unresponsive peer.
#[test]
fn send_returns_send_buffer_full_once_the_bound_is_reached() {
    let (mut client, _server, _t) = establish_pair();

    // default_config's log_window_size is 6 (64 slots); the bound is 8x that.
    for _ in 0..512 {
        client.send(vec![0xAA; 4]).expect("send");
    }

    let result = client.send(vec![0xAA; 4]);
    let error = result.unwrap_err();
    assert!(matches!(error.error.kind(), RdpeudpErrorKind::SendBufferFull));
    // The rejected payload comes back rather than being silently dropped, so
    // a caller with somewhere to hold it can retry once capacity opens.
    assert_eq!(error.data, vec![0xAA; 4]);
}

/// `log_window_size` shifts a `u16` and a `usize` elsewhere in the state
/// machine, so an unvalidated out-of-range value panics on the first
/// handshake datagram rather than surfacing as this constructor's `Result`.
#[test]
fn connect_rejects_an_out_of_range_log_window_size() {
    let t = now();
    let mut config = default_config(100);
    config.log_window_size = 16;

    let result = RdpeudpConnection::connect(config, t);
    assert!(matches!(result.unwrap_err().kind(), RdpeudpErrorKind::InvalidState));
}

#[test]
fn accept_rejects_an_out_of_range_log_window_size() {
    let t = now();
    let mut config = default_config(200);
    config.log_window_size = 16;

    let syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(test_syn_data_ex()),
    };

    let result = RdpeudpConnection::accept(config, &syn, t);
    assert!(matches!(result.unwrap_err().kind(), RdpeudpErrorKind::InvalidState));
}

// ── Data transfer tests ──

/// Helper: perform a full handshake and return (client, server, time).
fn establish_pair() -> (RdpeudpConnection, RdpeudpConnection, MonotonicInstant) {
    let t = now();

    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn = client.poll_transmit(t).expect("SYN");

    let syn_dg: V1Datagram = decode(&syn.contents).expect("decode SYN");
    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, t).expect("accept");
    let syn_ack = server.poll_transmit(t).expect("SYN+ACK");

    let mut syn_ack_bytes = syn_ack.contents;
    client
        .handle_datagram(&mut syn_ack_bytes, later(t, 50))
        .expect("handle SYN+ACK");

    // Drain client events and final ACK
    while client.poll_event().is_some() {}
    let final_ack = client.poll_transmit(later(t, 50)).expect("final ACK");

    let mut ack_bytes = final_ack.contents;
    server
        .handle_datagram(&mut ack_bytes, later(t, 100))
        .expect("handle ACK");
    while server.poll_event().is_some() {}

    (client, server, t)
}

/// Each side's handshake round trip seeds its RTT estimate, rather than
/// leaving it unset until the first data-phase ACK.
#[test]
fn handshake_seeds_rtt_estimate() {
    let (client, server, _t) = establish_pair();

    // Client: SYN sent at t=0, SYN+ACK handled at t=50.
    assert_eq!(client.srtt(), Some(Duration::from_millis(50)));

    // Server: SYN+ACK sent at t=0 (inside `accept`), final ACK handled at t=100.
    assert_eq!(server.srtt(), Some(Duration::from_millis(100)));
}

/// Karn's algorithm (RFC 6298 Section 3): once the client's SYN has been
/// retransmitted, a SYN+ACK that arrives afterward cannot be tied to a single
/// transmission, so it must not seed an RTT sample.
#[test]
fn a_retransmitted_syn_skips_the_handshake_rtt_sample() {
    let t = now();
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let first_syn = client.poll_transmit(t).expect("SYN");

    let _retransmitted_syn = advance_to_retransmit(&mut client).expect("SYN retransmit");
    let deadline = client.poll_timeout().expect("retransmit timer still armed");

    let syn_dg: V1Datagram = decode(&first_syn.contents).expect("decode SYN");
    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, deadline).expect("accept");
    let syn_ack = server.poll_transmit(deadline).expect("SYN+ACK");

    let mut syn_ack_bytes = syn_ack.contents;
    client
        .handle_datagram(&mut syn_ack_bytes, later(deadline, 10))
        .expect("handle SYN+ACK");

    assert_eq!(client.srtt(), None, "an ambiguous answer must not seed a sample");
}

#[test]
fn send_data_after_established() {
    let (mut client, _server, t) = establish_pair();

    // Send data
    client.send(vec![0xDE, 0xAD, 0xBE, 0xEF]).expect("send");

    // Should produce a data transmit
    let transmit = client.poll_transmit(later(t, 200)).expect("data packet");
    assert!(!transmit.contents.is_empty());
}

#[test]
fn data_delivery_roundtrip() {
    let (mut client, mut server, t) = establish_pair();

    // Client sends data
    let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
    client.send(payload.clone()).expect("send");
    let data_pkt = client.poll_transmit(later(t, 200)).expect("data packet");

    // Server receives data
    let mut data_bytes = data_pkt.contents;
    server
        .handle_datagram(&mut data_bytes, later(t, 250))
        .expect("handle data");

    // Server should emit DataReceived event
    let event = server.poll_event().expect("should have event");
    assert_eq!(event, Event::DataReceived(payload));
}

#[test]
fn bidirectional_data() {
    let (mut client, mut server, t) = establish_pair();

    // Client → Server
    let c2s = vec![0x01, 0x02, 0x03];
    client.send(c2s.clone()).expect("send c2s");
    let pkt = client.poll_transmit(later(t, 200)).expect("c2s packet");
    let mut pkt_bytes = pkt.contents;
    server
        .handle_datagram(&mut pkt_bytes, later(t, 250))
        .expect("handle c2s");

    let event = server.poll_event().expect("c2s event");
    assert_eq!(event, Event::DataReceived(c2s));

    // Server → Client
    let s2c = vec![0x04, 0x05, 0x06];
    server.send(s2c.clone()).expect("send s2c");
    let pkt = server.poll_transmit(later(t, 300)).expect("s2c packet");
    let mut pkt_bytes = pkt.contents;
    client
        .handle_datagram(&mut pkt_bytes, later(t, 350))
        .expect("handle s2c");

    let event = client.poll_event().expect("s2c event");
    assert_eq!(event, Event::DataReceived(s2c));
}

// ── Close and timeout tests ──

#[test]
fn close_produces_event() {
    let (mut client, _server, _t) = establish_pair();
    client.close();

    assert!(client.is_closed());
    let event = client.poll_event().expect("close event");
    assert_eq!(event, Event::ConnectionClosed);
}

#[test]
fn send_after_close_returns_error() {
    let (mut client, _server, _t) = establish_pair();
    client.close();

    let result = client.send(vec![0x42]);
    assert!(matches!(
        result.unwrap_err().error.kind(),
        RdpeudpErrorKind::ConnectionClosed
    ));
}

#[test]
fn idle_timeout_closes_connection() {
    let (mut client, _server, t) = establish_pair();

    // Advance time past idle timeout
    let idle_time = later(t, 66_000); // 66 seconds > the 65 second timeout
    client.handle_timeout(idle_time);

    assert!(client.is_closed());
    let event = client.poll_event().expect("idle close event");
    assert_eq!(event, Event::ConnectionClosed);
}

#[test]
fn keep_alive_fires() {
    let (mut client, _server, t) = establish_pair();

    // Advance time past keep-alive interval
    let ka_time = later(t, 8_500); // 8.5 seconds > 8 second interval
    client.handle_timeout(ka_time);

    // The keep-alive leaves an acknowledgement pending, which the next
    // poll_transmit turns into a probe on the wire.
    assert!(client.poll_transmit(ka_time).is_some());
}

#[test]
fn poll_timeout_returns_earliest() {
    let (client, _server, _t) = establish_pair();

    let timeout = client.poll_timeout();
    assert!(timeout.is_some());
}

// ── Congestion backpressure test ──

#[test]
fn congestion_backpressure() {
    let (mut client, _server, t) = establish_pair();

    // One packet's worth per write, so the count below stays a packet count.
    // A larger write would be split and the arithmetic would not follow.
    let mtu_data = vec![0xAA; client.max_payload()];
    let mut sent_count = 0u32;

    for _ in 0..100 {
        if client.send(mtu_data.clone()).is_ok() {
            sent_count += 1;
        }
    }
    assert!(sent_count > 0);

    // Drain transmits until congestion window is full
    let mut transmit_count = 0u32;
    let t2 = later(t, 200);
    while client.poll_transmit(t2).is_some() {
        transmit_count += 1;
        if transmit_count > 200 {
            break; // safety valve
        }
    }

    // Should have been limited by the congestion window.
    // The initial window is 12320 bytes and each packet carries one full
    // payload, so a dozen or so go out before it closes.
    assert!(transmit_count > 0);
    assert!(transmit_count <= 15); // reasonable upper bound with overhead
}

#[test]
fn multiple_data_chunks_delivered_in_order() {
    let (mut client, mut server, t) = establish_pair();

    // Send multiple data chunks
    for i in 0u8..5 {
        client.send(vec![i; 100]).expect("send");
    }

    // Transmit and deliver each
    for i in 0u8..5 {
        let pkt = client.poll_transmit(later(t, 200 + u64::from(i) * 50));
        if let Some(pkt) = pkt {
            let mut pkt_bytes = pkt.contents;
            server
                .handle_datagram(&mut pkt_bytes, later(t, 225 + u64::from(i) * 50))
                .expect("handle data");
        }
    }

    // Verify all events arrived in order
    let mut received = Vec::new();
    while let Some(event) = server.poll_event() {
        if let Event::DataReceived(data) = event {
            received.push(data[0]); // first byte identifies the chunk
        }
    }

    assert_eq!(received, vec![0, 1, 2, 3, 4]);
}

#[test]
fn handle_datagram_after_close_returns_error() {
    let (mut client, _server, t) = establish_pair();
    client.close();

    let mut wire = vec![0u8; 16];
    let result = client.handle_datagram(&mut wire, later(t, 100));
    assert!(matches!(result.unwrap_err().kind(), RdpeudpErrorKind::ConnectionClosed));
}

#[test]
fn config_default() {
    let config = ConnectionConfig::default();
    assert_eq!(config.log_window_size, 6);
    assert_eq!(config.upstream_mtu, 1232);
    assert_eq!(config.downstream_mtu, 1232);
    assert_eq!(config.idle_timeout, Duration::from_secs(65));
    assert_eq!(config.keep_alive_interval, Duration::from_secs(8));
}

#[test]
fn debug_impl() {
    let t = now();
    let conn = RdpeudpConnection::connect(default_config(1), t);
    let debug = format!("{conn:?}");
    assert!(debug.contains("RdpeudpConnection"));
    assert!(debug.contains("Client"));
}

/// Regression: the idle timeout must not leave a re-armed timer behind.
///
/// `handle_timeout` collects the expired timers, then handles them in order.
/// The idle timer closes the connection and `close` clears every timer, but a
/// handler later in the same list used to run anyway and re-arm its own timer
/// on a connection that had already gone. `handle_timeout` returns early once
/// closed, so that deadline was never serviced: a driver polling it without
/// checking `is_closed` first would wake immediately, do nothing, and spin.
///
/// Found by the `rdpeudp_connection` fuzz oracle.
#[test]
fn idle_close_leaves_no_timer_armed() {
    let mut now = MonotonicInstant::from_millis(0);
    let mut conn = RdpeudpConnection::connect(default_config(100), now).expect("connect");
    while conn.poll_transmit(now).is_some() {}

    // Walk past the keep-alive interval so that timer is armed and due in the
    // same pass as the idle timer, which is the ordering that used to break.
    for step in [1_000u64, 8_000, 65_000] {
        now = now + Duration::from_millis(step);
        conn.handle_timeout(now);
        while conn.poll_transmit(now).is_some() {}
    }

    assert!(conn.is_closed(), "the idle timeout should have closed the connection");
    assert_eq!(
        conn.poll_timeout(),
        None,
        "a closed connection must not report a deadline"
    );

    // Advancing again must not resurrect one.
    now = now + Duration::from_secs(60);
    conn.handle_timeout(now);
    assert_eq!(conn.poll_timeout(), None, "a closed connection re-armed a timer");
}

/// Regression: a closed connection must not emit packets or arm timers.
///
/// `poll_transmit` drained queued handshake packets before checking the state,
/// and armed the keep-alive timer on the way past. `close` had already cleared
/// the timers and `handle_timeout` returns early once closed, so that deadline
/// stayed due forever and a caller polling it without checking `is_closed`
/// would wake immediately, do nothing, and spin.
///
/// Found by the `rdpeudp_connection` fuzz oracle.
#[test]
fn closed_connection_transmits_nothing_and_arms_nothing() {
    let now = MonotonicInstant::from_millis(0);

    // Close while the SYN is still queued, which is the state the oracle found.
    let mut conn = RdpeudpConnection::connect(default_config(100), now).expect("connect");
    conn.close();

    assert!(
        conn.poll_transmit(now).is_none(),
        "a closed connection emitted a packet"
    );
    assert_eq!(conn.poll_timeout(), None, "a closed connection armed a timer");

    // And it stays that way once the clock moves on.
    let later = now + Duration::from_secs(30);
    conn.handle_timeout(later);
    assert!(conn.poll_transmit(later).is_none());
    assert_eq!(conn.poll_timeout(), None);
}

// ── Handshake retransmission ──
//
// [MS-RDPEUDP] 1.3.1: the SYN, SYN+ACK and ACK are delivered by persistent
// retransmits whatever mode the transport is running in. 3.1.5.4.1 stops
// after somewhere between three and five unanswered tries.

/// Drive the connection to its next retransmit deadline and collect whatever
/// it decides to send.
fn advance_to_retransmit(conn: &mut RdpeudpConnection) -> Option<Vec<u8>> {
    let deadline = conn.poll_timeout()?;
    conn.handle_timeout(deadline);
    conn.poll_transmit(deadline).map(|transmit| transmit.contents)
}

#[test]
fn unanswered_syn_is_retransmitted() {
    let t = now();
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");

    let first = client.poll_transmit(t).expect("SYN");
    let again = advance_to_retransmit(&mut client).expect("SYN again");

    assert_eq!(first.contents, again, "the retransmitted SYN differs from the original");
    assert!(!client.is_closed());
}

#[test]
fn unanswered_syn_ack_is_retransmitted() {
    let t = now();

    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn = client.poll_transmit(t).expect("SYN");
    let syn_dg: V1Datagram = decode(&syn.contents).expect("decode SYN");

    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, t).expect("accept");
    let first = server.poll_transmit(t).expect("SYN+ACK");
    let again = advance_to_retransmit(&mut server).expect("SYN+ACK again");

    assert_eq!(first.contents, again);
    assert!(!server.is_closed());
}

#[test]
fn a_syn_that_is_never_answered_closes_the_connection() {
    let t = now();
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    client.poll_transmit(t).expect("SYN");

    // The retransmit timer keeps firing until the limit is reached. The idle
    // timer would also close this connection eventually, so the assertion
    // below is that we gave up on the retransmit count first.
    let mut retransmits = 0;
    while !client.is_closed() {
        let deadline = client.poll_timeout().expect("a timer is armed");
        assert!(
            deadline < t + client_idle_timeout(),
            "gave up on the idle timeout rather than the retransmit limit"
        );

        client.handle_timeout(deadline);
        if client.poll_transmit(deadline).is_some() {
            retransmits += 1;
        }
    }

    assert_eq!(retransmits, 5, "[MS-RDPEUDP] 3.1.5.4.1 allows between three and five");
    assert_eq!(client.poll_event(), Some(Event::ConnectionClosed));
}

fn client_idle_timeout() -> Duration {
    default_config(0).idle_timeout
}

#[test]
fn a_repeated_syn_draws_another_syn_ack() {
    let t = now();

    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn = client.poll_transmit(t).expect("SYN");
    let syn_dg: V1Datagram = decode(&syn.contents).expect("decode SYN");

    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, t).expect("accept");
    let syn_ack = server.poll_transmit(t).expect("SYN+ACK");

    // The client did not see that SYN+ACK, so it repeats its SYN.
    let mut repeated = syn.contents;
    server
        .handle_datagram(&mut repeated, later(t, 500))
        .expect("a repeated SYN is not a protocol violation");

    let answer = server.poll_transmit(later(t, 500)).expect("another SYN+ACK");
    assert_eq!(answer.contents, syn_ack.contents);
    assert!(!server.is_established());
}

#[test]
fn a_repeated_syn_ack_draws_another_final_ack() {
    let t = now();

    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn = client.poll_transmit(t).expect("SYN");
    let syn_dg: V1Datagram = decode(&syn.contents).expect("decode SYN");

    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, t).expect("accept");
    let syn_ack = server.poll_transmit(t).expect("SYN+ACK");

    let mut syn_ack_bytes = syn_ack.contents.clone();
    client
        .handle_datagram(&mut syn_ack_bytes, later(t, 50))
        .expect("handle SYN+ACK");
    while client.poll_event().is_some() {}

    let final_ack = client.poll_transmit(later(t, 50)).expect("final ACK");
    assert!(client.is_established());

    // That ACK was lost, so the server repeats its SYN+ACK. The client is on
    // v2 by now and must still recognise it.
    let mut repeated = syn_ack.contents;
    client
        .handle_datagram(&mut repeated, later(t, 600))
        .expect("a repeated SYN+ACK is not a protocol violation");

    let answer = client.poll_transmit(later(t, 600)).expect("the final ACK again");
    assert_eq!(answer.contents, final_ack.contents);
    assert!(client.is_established(), "the connection did not fall back a state");
}

#[test]
fn data_is_not_mistaken_for_a_repeated_syn_ack() {
    let (mut client, mut server, t) = establish_pair();

    server.send(vec![0x11; 32]).expect("send");
    let mut data = server.poll_transmit(later(t, 200)).expect("data").contents;

    client.handle_datagram(&mut data, later(t, 250)).expect("handle data");

    assert_eq!(client.poll_event(), Some(Event::DataReceived(vec![0x11; 32])));
}

// ── Receive window lifecycle ──

/// Shuttle packets between the two endpoints until neither has more to say.
fn pump(client: &mut RdpeudpConnection, server: &mut RdpeudpConnection, at: MonotonicInstant) {
    for _ in 0..10_000 {
        let mut moved = false;

        while let Some(transmit) = client.poll_transmit(at) {
            let mut bytes = transmit.contents;
            server.handle_datagram(&mut bytes, at).expect("server handles");
            moved = true;
        }

        while let Some(transmit) = server.poll_transmit(at) {
            let mut bytes = transmit.contents;
            client.handle_datagram(&mut bytes, at).expect("client handles");
            moved = true;
        }

        if !moved {
            return;
        }
    }

    panic!("the endpoints never went quiet");
}

fn drain_received(conn: &mut RdpeudpConnection) -> Vec<Vec<u8>> {
    let mut received = Vec::new();
    while let Some(event) = conn.poll_event() {
        if let Event::DataReceived(chunk) = event {
            received.push(chunk);
        }
    }
    received
}

/// [MS-RDPEUDP2] 3.1.1.2.2 moves the receive window's lower bound when an
/// acknowledgment naming those packets is sent, as well as when an AckOfAcks
/// arrives. Nothing is lost in this exchange, so the receiver never sends an
/// ACK vector and the sender never sends an AckOfAcks: the bound has to move
/// on the first of those two events or the window fills and stays full.
#[test]
fn a_clean_transfer_outlasts_the_receive_window() {
    let (mut client, mut server, t) = establish_pair();

    // Four times the 64-slot window implied by log_window_size 6.
    let chunk_count = 256usize;
    for i in 0..chunk_count {
        let byte = u8::try_from(i % 256).expect("modulo 256 fits in u8");
        client.send(vec![byte; 4]).expect("send");
    }

    pump(&mut client, &mut server, later(t, 200));

    let received = drain_received(&mut server);
    assert_eq!(received.len(), chunk_count, "the receive window stalled");

    for (i, chunk) in received.iter().enumerate() {
        let byte = u8::try_from(i % 256).expect("modulo 256 fits in u8");
        assert_eq!(chunk, &vec![byte; 4], "chunk {i} arrived out of order");
    }
}

/// Run both endpoints forward through their own timers up to `until`,
/// delivering everything they emit along the way.
fn advance(client: &mut RdpeudpConnection, server: &mut RdpeudpConnection, until: MonotonicInstant) {
    for _ in 0..1_000 {
        let next = [client.poll_timeout(), server.poll_timeout()]
            .into_iter()
            .flatten()
            .min();

        let Some(at) = next.filter(|at| *at <= until) else {
            return;
        };

        client.handle_timeout(at);
        server.handle_timeout(at);
        pump(client, server, at);
    }

    panic!("the endpoints never stopped scheduling work");
}

/// New data acknowledged retires whichever packet the retransmit timer's
/// deadline was set for. A later, still-outstanding packet must not then be
/// measured against a deadline meant for a different packet, or it can be
/// declared lost within a few milliseconds instead of a full RTO.
#[test]
fn an_ack_restarts_the_retransmit_timer_for_the_still_outstanding_packet() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xA1; 8]).expect("send");
    let a = client.poll_transmit(later(t, 500)).expect("packet A");
    let a_deadline = client.poll_timeout().expect("retransmit timer armed for A");

    client.send(vec![0xB1; 8]).expect("send");
    let _b = client.poll_transmit(later(t, 600)).expect("packet B");
    // Sending B does not touch an already-armed timer.
    assert_eq!(client.poll_timeout(), Some(a_deadline));

    let mut a_bytes = a.contents;
    server.handle_datagram(&mut a_bytes, later(t, 520)).expect("handle A");
    let ack_deadline = server.poll_timeout().expect("delayed-ACK timer armed");
    server.handle_timeout(ack_deadline);
    let ack = server.poll_transmit(ack_deadline).expect("acknowledgment");

    // Delivered 5ms before A's stale deadline: barely any protection left
    // for B under the bug, comfortably inside a fresh RTO under the fix.
    let mut ack_bytes = ack.contents;
    client
        .handle_datagram(&mut ack_bytes, later(t, 795))
        .expect("handle ack");

    let new_deadline = client.poll_timeout().expect("retransmit timer still armed for B");
    assert!(
        new_deadline > a_deadline,
        "the timer must restart from B's own outstanding state, not stay at A's stale deadline"
    );
    assert!(
        new_deadline.duration_since(later(t, 795)) >= Duration::from_millis(200),
        "the restarted deadline should reflect close to a full RTO, not a few leftover milliseconds"
    );
}

/// A packet lost on its own, with too few behind it to trip the reordering
/// threshold, has only the retransmit timer to recover it. That timer used to
/// hand the decision to a time threshold derived from an RTO it had just
/// doubled, so the threshold always sat ahead of the elapsed time and the
/// packet was never retransmitted at all.
#[test]
fn a_lone_lost_packet_is_recovered() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xA1; 8]).expect("send");
    client.send(vec![0xA2; 8]).expect("send");

    // The first packet never reaches the server.
    let _dropped = client.poll_transmit(later(t, 100)).expect("first data packet");
    let delivered = client.poll_transmit(later(t, 100)).expect("second data packet");

    let mut bytes = delivered.contents;
    server.handle_datagram(&mut bytes, later(t, 150)).expect("handle data");

    // Nothing has been delivered to the application: the second chunk is
    // waiting behind the hole.
    assert!(drain_received(&mut server).is_empty());

    advance(&mut client, &mut server, later(t, 10_000));

    assert_eq!(
        drain_received(&mut server),
        vec![vec![0xA1; 8], vec![0xA2; 8]],
        "the lost packet was never retransmitted"
    );
}

/// A cumulative ACK claims every packet through its sequence number, so it
/// cannot describe a window with a hole in it. The piggybacked path sent one
/// regardless, which tells the sender its lost packet arrived.
#[test]
fn a_hole_is_never_reported_as_a_cumulative_ack() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xA1; 8]).expect("send");
    client.send(vec![0xA2; 8]).expect("send");

    let _dropped = client.poll_transmit(later(t, 100)).expect("first data packet");
    let delivered = client.poll_transmit(later(t, 100)).expect("second data packet");

    let mut bytes = delivered.contents;
    server.handle_datagram(&mut bytes, later(t, 150)).expect("handle data");

    // Give the server data of its own, so its acknowledgment rides along on a
    // data packet rather than going out standalone.
    server.send(vec![0xB1; 8]).expect("send");
    let reply = server.poll_transmit(later(t, 200)).expect("server data packet");

    let mut reply_bytes = reply.contents;
    let (_, packet_bytes) = decode_with_prefix(&mut reply_bytes).expect("prefix");
    let packet: V2Packet = decode(packet_bytes).expect("decode");

    assert!(packet.data_body.is_some(), "the acknowledgment should be piggybacked");
    assert!(packet.ack.is_none(), "a cumulative ACK cannot describe a hole");
    assert!(packet.ack_vector.is_some(), "the hole needs a vector to describe it");
    assert!(packet.header.flags.contains(V2Flags::ACKVEC));
    assert!(!packet.header.flags.contains(V2Flags::ACK));
}

/// A cumulative ACK claiming a sequence number the sender never assigned is
/// malformed or hostile. Accepting it would drain every outstanding entry in
/// the send window as if delivered, discarding real in-flight data on a
/// single bad datagram.
#[test]
fn an_out_of_range_cumulative_ack_does_not_discard_outstanding_data() {
    let (mut client, _server, t) = establish_pair();

    client.send(vec![0xE1; 8]).expect("send");
    let _sent = client.poll_transmit(later(t, 100)).expect("data packet");

    // A cumulative ACK claiming far more than the client has ever sent.
    let bogus_ack = V2Packet {
        header: V2Header {
            flags: V2Flags::empty(),
            log_window_size: 6,
        },
        ack: Some(AckPayload {
            seq_num: 0xFFFF,
            received_ts: 0,
            send_ack_time_gap: 0,
            delay_ack_time_scale: 0,
            delay_ack_time_additions: Vec::new(),
        }),
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: None,
        ack_vector: None,
        data_body: None,
    };
    let encoded = encode_vec(&bogus_ack).expect("encode");
    let mut wire = Vec::new();
    encode_with_prefix(&encoded, false, &mut wire).expect("prefix");
    client
        .handle_datagram(&mut wire, later(t, 160))
        .expect("handle bogus ack");

    // The data is still tracked as outstanding and gets retransmitted, which
    // proves the bogus ACK did not drain it from the send window. Compared by
    // decoded payload, not raw bytes: a retransmission piggybacks whatever
    // header and ACK state is current, so the framing legitimately differs
    // from the original send.
    let mut retransmitted = advance_to_retransmit(&mut client).expect("data packet retransmitted");
    let (_, packet_bytes) = decode_with_prefix(&mut retransmitted).expect("prefix");
    let packet: V2Packet = decode(packet_bytes).expect("decode");
    assert_eq!(
        packet.data_body.expect("retransmission carries the data").data,
        vec![0xE1; 8],
        "the outstanding packet must survive an out-of-range ACK"
    );
}

/// Acknowledging a packet resolves it and drains it off the front of the send
/// window, so the timing has to be read before that happens. Reading it after
/// finds nothing and no sample reaches the estimator from this exchange.
///
/// The handshake round trip already seeded the estimator (see
/// `handshake_seeds_rtt_estimate`), so this data-phase acknowledgment is the
/// *second* sample: it refines the existing estimate rather than setting it
/// from nothing.
#[test]
fn an_acknowledgement_produces_an_rtt_sample() {
    let (mut client, mut server, t) = establish_pair();

    // Handshake sample: SYN at t=0, SYN+ACK handled at t=50.
    assert_eq!(client.srtt(), Some(Duration::from_millis(50)));

    client.send(vec![0xC1; 16]).expect("send");
    let data = client.poll_transmit(later(t, 100)).expect("data packet");

    let mut bytes = data.contents;
    server.handle_datagram(&mut bytes, later(t, 140)).expect("handle data");

    // The delayed-ACK timer arms when the data arrives (t=140) and the
    // server's handshake-seeded 100ms srtt gives a 50ms delay, so it fires
    // at t=190: a real 50ms gap between receipt and send, encoded on the
    // wire rather than reported as zero.
    server.handle_timeout(later(t, 190));
    let ack = server.poll_transmit(later(t, 190)).expect("acknowledgment");

    // The ACK reaches the client 60ms after the server sent it, at t=250:
    // after t=190, not before it.
    let mut ack_bytes = ack.contents;
    client
        .handle_datagram(&mut ack_bytes, later(t, 250))
        .expect("handle ack");

    // Client-side elapsed since the data went out is 250 - 100 = 150ms.
    // Subtracting the real 50ms processing gap gives a 100ms sample; folding
    // the whole gap in unsubtracted, as an always-zero gap would, gives a
    // wrong 150ms sample instead. RFC 6298 Section 2.3 blends the correct
    // 100ms sample with the existing 50ms estimate:
    // srtt = 7/8 * 50ms + 1/8 * 100ms = 56.25ms.
    let srtt = client.srtt().expect("an RTT sample");
    assert_eq!(srtt, Duration::from_micros(56_250));
}

// ── Protocol version negotiation ──
//
// [MS-RDPEUDP] 1.3.2.2: the MS-RDPEUDP data transfer messages "MUST be used
// only when the version negotiated in the UDP connection initialization phase
// is version 1 or version 2". Only version 3 reaches MS-RDPEUDP2, which is the
// data transfer this crate implements, so version 3 is the only value it can
// honestly advertise or accept.

fn syn_data_ex_of(transmit: &Transmit) -> SynDataExPayload {
    let datagram: V1Datagram = decode(&transmit.contents).expect("decode");
    datagram.syn_data_ex.expect("SYNDATAEX")
}

#[test]
fn the_syn_advertises_version_3_with_the_cookie_hash() {
    let t = now();
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");

    let syn = client.poll_transmit(t).expect("SYN");
    let syn_data_ex = syn_data_ex_of(&syn);

    assert_eq!(syn_data_ex.udp_ver, UdpVersion::V3);
    assert_eq!(
        syn_data_ex.cookie_hash,
        Some(TEST_COOKIE_HASH),
        "[MS-RDPEUDP] 2.2.2.9 requires the hash alongside version 3"
    );
}

#[test]
fn the_syn_ack_advertises_version_3_without_a_cookie_hash() {
    let t = now();

    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    let syn = client.poll_transmit(t).expect("SYN");
    let syn_dg: V1Datagram = decode(&syn.contents).expect("decode SYN");

    let mut server = RdpeudpConnection::accept(default_config(200), &syn_dg, t).expect("accept");
    let syn_ack = server.poll_transmit(t).expect("SYN+ACK");
    let syn_data_ex = syn_data_ex_of(&syn_ack);

    assert_eq!(syn_data_ex.udp_ver, UdpVersion::V3);
    assert_eq!(
        syn_data_ex.cookie_hash, None,
        "2.2.2.9: the hash belongs in the client's SYN and \"MUST NOT be present in any other case\""
    );
}

#[test]
fn connect_refuses_to_build_a_version_3_syn_without_a_cookie_hash() {
    let config = ConnectionConfig {
        cookie_hash: None,
        ..default_config(100)
    };

    RdpeudpConnection::connect(config, now()).expect_err("a version 3 SYN must carry the hash");
}

#[test]
fn a_server_rejects_a_syn_offering_a_version_below_3() {
    let t = now();

    let syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            // 0x0002 means the MS-RDPEUDP data transfer, not MS-RDPEUDP2.
            udp_ver: UdpVersion::V2,
            cookie_hash: None,
        }),
    };

    RdpeudpConnection::accept(default_config(200), &syn, t).expect_err("version 2 is not RDPEUDP2");
}

#[test]
fn a_server_rejects_a_syn_whose_cookie_hash_does_not_match() {
    let t = now();

    let syn = V1Datagram {
        header: FecHeader {
            sn_source_ack: 0xFFFF_FFFF,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 100,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V3,
            cookie_hash: Some([0xFF; 32]),
        }),
    };

    RdpeudpConnection::accept(default_config(200), &syn, t).expect_err("the hash is for a different cookie");
}

#[test]
fn a_client_rejects_a_syn_ack_that_settles_below_version_3() {
    let t = now();
    let mut client = RdpeudpConnection::connect(default_config(100), t).expect("connect");
    client.poll_transmit(t).expect("SYN");

    let syn_ack = V1Datagram {
        header: FecHeader {
            sn_source_ack: 100,
            receive_window_size: 64,
            flags: V1Flags::SYN | V1Flags::ACK | V1Flags::SYNEX,
        },
        ack_vector: None,
        ack_of_acks: None,
        syn_data: Some(SynDataPayload {
            initial_sequence_number: 200,
            upstream_mtu: 1232,
            downstream_mtu: 1232,
        }),
        correlation_id: None,
        syn_data_ex: Some(SynDataExPayload {
            syn_ex_flags: SynExFlags::VERSION_INFO_VALID,
            udp_ver: UdpVersion::V2,
            cookie_hash: None,
        }),
    };

    let mut bytes = encode_vec(&syn_ack).expect("encode");
    client
        .handle_datagram(&mut bytes, later(t, 50))
        .expect_err("the server settled on the MS-RDPEUDP data transfer");
    assert!(!client.is_established());
}

/// A transfer longer than the window survives a loss partway through.
///
/// A retransmission carries a fresh DataSeqNum, so the lost one is never
/// filled and [MS-RDPEUDP2] 3.1.1.2.2 will not let the receiver's lower bound
/// advance over it. The AckOfAcks payload is the only way past, and 3.1.5.3
/// has the sender advertise it on exactly this event: "the Sender declares the
/// packet loss and updates its own next sequence number to wait on. It then
/// sends the AckOfAcks payload ... so that the Receiver can stop waiting for
/// any packets with lower sequence numbers to arrive."
///
/// Sending the peer's own numbers back instead leaves its window pinned on the
/// hole, and the transfer stops one window later.
#[test]
fn a_transfer_survives_a_loss_in_the_middle() {
    let (mut client, mut server, t) = establish_pair();

    // Well past the 64-slot window implied by log_window_size 6.
    let chunk_count = 200usize;
    for i in 0..chunk_count {
        let byte = u8::try_from(i % 256).expect("modulo 256 fits in u8");
        client.send(vec![byte; 4]).expect("send");
    }

    // The first packet never reaches the server.
    let _lost = client.poll_transmit(later(t, 100)).expect("first data packet");

    advance(&mut client, &mut server, later(t, 30_000));

    let received = drain_received(&mut server);
    assert_eq!(
        received.len(),
        chunk_count,
        "the transfer stalled: the receiver's window never moved past the hole"
    );

    for (i, chunk) in received.iter().enumerate() {
        let byte = u8::try_from(i % 256).expect("modulo 256 fits in u8");
        assert_eq!(chunk, &vec![byte; 4], "chunk {i} arrived out of order");
    }
}

// ── Dummy packets ──
//
// [MS-RDPEUDP2] 3.1.1.1.5: "If Packet_Type_Index is set to 8, then a dummy
// packet follows the PacketPrefixByte. A dummy packet is treated as a normal
// RDP-UDP2 packet by the UDP transport. However, loss of this packet MUST not
// generate a retransmit. In addition, the contents MUST be ignored by higher
// layers using the UDP transport."

/// Re-frame a v2 packet as a dummy, which is a property of the prefix byte
/// rather than anything in the header.
fn as_dummy(transmit: Transmit) -> Vec<u8> {
    let mut bytes = transmit.contents;
    let (_, packet) = decode_with_prefix(&mut bytes).expect("prefix");
    let packet = packet.to_vec();

    let mut wire = Vec::new();
    encode_with_prefix(&packet, true, &mut wire).expect("re-frame as dummy");
    wire
}

#[test]
fn a_dummy_packets_contents_never_reach_the_application() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xD0; 16]).expect("send");
    let data = client.poll_transmit(later(t, 100)).expect("data packet");

    let mut dummy = as_dummy(data);
    server.handle_datagram(&mut dummy, later(t, 150)).expect("handle dummy");

    assert!(
        drain_received(&mut server).is_empty(),
        "the contents of a dummy packet must be ignored by higher layers"
    );
}

/// The transport still accounts for a dummy, because 3.1.1.1.5 has it
/// "treated as a normal RDP-UDP2 packet by the UDP transport". Its sequence
/// number is filled and acknowledged like any other, so the sender is not left
/// retransmitting it forever.
#[test]
fn a_dummy_packet_is_still_acknowledged() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xD0; 16]).expect("send");
    let data = client.poll_transmit(later(t, 100)).expect("data packet");

    let mut dummy = as_dummy(data);
    server.handle_datagram(&mut dummy, later(t, 150)).expect("handle dummy");

    // The delayed-ACK timer is the sign the transport took it as a real packet.
    let deadline = server.poll_timeout().expect("a timer is armed");
    server.handle_timeout(deadline);
    let ack = server.poll_transmit(deadline).expect("an acknowledgment");

    let mut ack_bytes = ack.contents;
    let (_, packet_bytes) = decode_with_prefix(&mut ack_bytes).expect("prefix");
    let packet: V2Packet = decode(packet_bytes).expect("decode");

    assert!(
        packet.ack.is_some() || packet.ack_vector.is_some(),
        "a dummy packet should still be acknowledged"
    );
}

/// A dummy carries a ChannelSeqNum that belongs to no real datum, so taking it
/// into reassembly would leave the delivery cursor waiting on data nobody will
/// ever send. Real data on either side of it must still flow.
#[test]
fn a_dummy_does_not_disturb_the_channel_sequence() {
    let (mut client, mut server, t) = establish_pair();

    client.send(vec![0xA1; 8]).expect("send");
    let mut first = client.poll_transmit(later(t, 100)).expect("first data packet").contents;
    server.handle_datagram(&mut first, later(t, 110)).expect("handle data");
    assert_eq!(drain_received(&mut server), vec![vec![0xA1; 8]]);

    // A synthetic dummy, which is what a real sender emits: it takes a
    // DataSeqNum of its own and its ChannelSeqNum names nothing.
    let dummy = V2Packet {
        header: V2Header {
            flags: V2Flags::DATA,
            log_window_size: 6,
        },
        ack: None,
        overhead_size: None,
        delay_ack_info: None,
        ack_of_acks: None,
        data_header: Some(DataHeader { data_seq_num: 150 }),
        ack_vector: None,
        data_body: Some(DataBody {
            channel_seq_num: 900,
            data: vec![0xFF; 8],
        }),
    };

    let encoded = encode_vec(&dummy).expect("encode");
    let mut wire = Vec::new();
    encode_with_prefix(&encoded, true, &mut wire).expect("prefix as dummy");
    server.handle_datagram(&mut wire, later(t, 120)).expect("handle dummy");

    assert!(
        drain_received(&mut server).is_empty(),
        "the contents of a dummy packet must be ignored"
    );

    // Real data keeps flowing behind it.
    client.send(vec![0xA2; 8]).expect("send");
    let mut second = client
        .poll_transmit(later(t, 130))
        .expect("second data packet")
        .contents;
    server.handle_datagram(&mut second, later(t, 140)).expect("handle data");

    assert_eq!(
        drain_received(&mut server),
        vec![vec![0xA2; 8]],
        "a dummy must not disturb the channel sequence"
    );
}

// ── Segmentation ──
//
// [MS-RDPEUDP2] does not segment. 3.1.1.2.4.2 has the receiver strip the
// ChannelSeqNum and forward "the rest of the data payload to the upper layer
// immediately", and nothing in the format marks a first or last fragment, so a
// packet's payload arrives whole or not at all. Anything longer than one
// packet has to be split before it becomes packets.

/// Every datagram the connection emits fits the MTU, however much the caller
/// writes at once.
///
/// The caller above is a TLS session, which emits records up to about 16 KiB
/// and knows nothing about datagrams. Passing one of those through as a single
/// packet produces an IP-fragmented datagram thirteen times the MTU, where one
/// lost fragment destroys the whole thing and no retransmission can recover a
/// fragment.
#[test]
fn a_write_larger_than_a_packet_is_split_to_fit() {
    let (mut client, mut server, t) = establish_pair();

    let payload: Vec<u8> = (0..16_384u32)
        .map(|i| u8::try_from(i % 256).expect("modulo 256 fits in u8"))
        .collect();
    client.send(payload.clone()).expect("send");

    let mut packets = 0usize;
    let mut at = later(t, 100);
    for _ in 0..2_000 {
        let mut moved = false;

        while let Some(transmit) = client.poll_transmit(at) {
            assert!(
                transmit.contents.len() <= 1232,
                "a {} byte datagram exceeds the 1232 byte MTU",
                transmit.contents.len()
            );
            packets += 1;

            let mut bytes = transmit.contents;
            server.handle_datagram(&mut bytes, at).expect("server handles");
            moved = true;
        }

        while let Some(transmit) = server.poll_transmit(at) {
            let mut bytes = transmit.contents;
            client.handle_datagram(&mut bytes, at).expect("client handles");
            moved = true;
        }

        if !moved {
            let Some(next) = [client.poll_timeout(), server.poll_timeout()]
                .into_iter()
                .flatten()
                .min()
            else {
                break;
            };

            at = next;
            client.handle_timeout(at);
            server.handle_timeout(at);
        }
    }

    assert!(packets > 1, "16 KiB should not have gone out as one packet");

    let received: Vec<u8> = drain_received(&mut server).concat();
    assert_eq!(received, payload, "the split payload did not reassemble");
}

/// The split is sized from the negotiated MTU, not from a fixed guess.
#[test]
fn the_payload_limit_follows_the_negotiated_mtu() {
    let (client, _server, _t) = establish_pair();

    let max_payload = client.max_payload();
    assert!(max_payload > 0);
    assert!(
        max_payload + 7 + 136 <= 1232,
        "the payload limit leaves no room for framing"
    );
}

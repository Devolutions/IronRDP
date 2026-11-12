#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::test_support::connect_gateway_tcp;
use tokio::net::TcpListener;

#[tokio::test]
async fn gateway_tcp_sockets_disable_nagle() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind gateway listener");
    let address = listener.local_addr().expect("gateway listener address");

    let address_stream = connect_gateway_tcp(&address.to_string())
        .await
        .expect("connect by address");
    let hostname_stream = connect_gateway_tcp(&format!("localhost:{}", address.port()))
        .await
        .expect("connect by hostname");

    // oracle: mutant leaving Nagle's algorithm enabled on gateway TCP sockets.
    assert!(address_stream.nodelay().expect("read TCP_NODELAY"));
    assert!(hostname_stream.nodelay().expect("read TCP_NODELAY"));
}

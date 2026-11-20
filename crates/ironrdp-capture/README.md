# ironrdp-capture

`ironrdp-capture` decodes Ethernet pcapng captures into reassembled RDP TCP flows and decrypts supported TLS sessions from NSS-compatible key logs.
It also unwraps supported RD Gateway WebSocket and RPC-over-HTTP tunnels into their inner RDP streams.

The crate is an internal analysis building block for offline tools such as `ironrdp-capture-replay`.
It retains TLS key material only in memory and does not write decrypted payloads or secrets.

RD Gateway (MS-TSGU) WebSocket and RPC-over-HTTP captures are supported when the outer HTTPS session is decrypted with an embedded or supplied TLS key log.
For WebSocket captures, the decoder removes WebSocket frames and `HTTP_DATA_PACKET` wrappers from an `RDG_OUT_DATA /remoteDesktopGateway/` upgrade and returns the inner RDP stream.
For RPC-over-HTTP captures, it unwraps the paired `RPC_IN_DATA` and `RPC_OUT_DATA` channels, including their HTTP heads and DCE/RPC `TsProxy` stubs, into the inner RDP stream.
A single IN or OUT channel is not a complete tunnel.

The decoder supports full TLS 1.2 and TLS 1.3 captures, including resumed sessions, for the AES-GCM cipher suites it handles.
TLS 1.2 requires `CLIENT_RANDOM`, while TLS 1.3 requires `CLIENT_HANDSHAKE_TRAFFIC_SECRET`, `SERVER_HANDSHAKE_TRAFFIC_SECRET`, `CLIENT_TRAFFIC_SECRET_0`, and `SERVER_TRAFFIC_SECRET_0`.
Mid-stream TLS captures without a ClientHello and ServerHello remain unsupported.

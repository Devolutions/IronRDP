# ironrdp-capture-replay

`ironrdp-capture-replay` replays supported direct-TCP RDP captures offline.
It is an internal library built on the replay routing pipeline.

RD Gateway (MS-TSGU) WebSocket and RPC-over-HTTP captures are supported after the outer HTTPS session is decrypted from an embedded or supplied TLS key log.
When that plaintext starts with an `RDG_OUT_DATA /remoteDesktopGateway/` upgrade, the WebSocket frames and `HTTP_DATA_PACKET` wrappers are removed and the recovered inner stream is replayed like a direct capture.
When the capture instead carries a pair of `RPC_IN_DATA` and `RPC_OUT_DATA` RPC-over-HTTP channels, those HTTP heads and DCE/RPC `TsProxy` stubs are unwrapped into the same inner RDP stream.
A single IN or OUT channel is not a complete tunnel.
Full TLS 1.2 and TLS 1.3 captures, including resumed sessions, are supported for the AES-GCM cipher suites handled by the replay decryptor.
TLS 1.2 requires `CLIENT_RANDOM`, while TLS 1.3 requires `CLIENT_HANDSHAKE_TRAFFIC_SECRET`, `SERVER_HANDSHAKE_TRAFFIC_SECRET`, `CLIENT_TRAFFIC_SECRET_0`, and `SERVER_TRAFFIC_SECRET_0`.
Mid-stream TLS captures without a ClientHello and ServerHello remain unsupported.

Use the binary-only [`ironrdp-capture-export`](../ironrdp-capture-export) package to write rendered frames and diagnostics.

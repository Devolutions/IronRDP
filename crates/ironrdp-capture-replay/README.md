# ironrdp-capture-replay

`ironrdp-capture-replay` replays supported direct-TCP RDP captures offline and exports framebuffer snapshots without writing TLS secrets, decrypted payloads, or raw capture data.
It is an internal analysis tool built on the replay routing pipeline.

RD Gateway (MS-TSGU) WebSocket and RPC-over-HTTP captures are supported after the outer HTTPS session is decrypted from an embedded or supplied TLS key log.
When that plaintext starts with an `RDG_OUT_DATA /remoteDesktopGateway/` upgrade, the WebSocket frames and `HTTP_DATA_PACKET` wrappers are removed and the recovered inner stream is replayed like a direct capture.
When the capture instead carries a pair of `RPC_IN_DATA` and `RPC_OUT_DATA` RPC-over-HTTP channels, those HTTP heads and DCE/RPC `TsProxy` stubs are unwrapped into the same inner RDP stream.
A single IN or OUT channel is not a complete tunnel.
Full TLS 1.2 and TLS 1.3 captures, including resumed sessions, are supported for the AES-GCM cipher suites handled by the replay decryptor.
TLS 1.2 requires `CLIENT_RANDOM`, while TLS 1.3 requires `CLIENT_HANDSHAKE_TRAFFIC_SECRET`, `SERVER_HANDSHAKE_TRAFFIC_SECRET`, `CLIENT_TRAFFIC_SECRET_0`, and `SERVER_TRAFFIC_SECRET_0`.
Mid-stream TLS captures without a ClientHello and ServerHello remain unsupported.

## Exporting frames

Run the CLI with a pcapng capture that contains the TLS key-log material required by the replay pipeline.

```shell
cargo run -p ironrdp-capture-replay --bin ironrdp-capture-replay -- capture.pcapng replay-output
```

Capture exports include secrets only for TLS flows the exporter can see.
Gateway-tunneled RDP is a separate TLS flow, so pass its original NSS-compatible key log with `--keylog`:

```shell
cargo run -p ironrdp-capture-replay --bin ironrdp-capture-replay -- --keylog tls-keys.log capture.pcapng replay-output
```

The key log remains in memory and is never written to replay output.

The command succeeds only when replay produces visual framebuffer updates.
It writes PNG files zero-padded to at least six digits (`frame_000000.png`, `frame_000001.png`, ...) in replay order, along with payload-free `frame_meta.psv`, `events.tsv`, `gaps.tsv`, and `dynamic-channels.tsv` files.
Each `frame_meta.psv` row maps a sequence filename to its source packet, dimensions, and full-frame update geometry.

The output directory must be empty by default.
Pass `--replace` to replace an existing directory after a complete export is staged successfully.

The current router exports snapshots only for graphics updates it can render.
It does not synthesize frames for routing-only captures or implement unsupported graphics codecs and channels.

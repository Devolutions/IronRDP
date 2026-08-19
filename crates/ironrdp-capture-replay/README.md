# ironrdp-capture-replay

`ironrdp-capture-replay` replays supported direct-TCP RDP captures offline and exports framebuffer snapshots without writing TLS secrets, decrypted payloads, or raw capture data.
It is an internal analysis tool built on the replay routing pipeline.

RD Gateway (MS-TSGU) captures are supported: when the selected flow is a TLS session whose plaintext starts with an `RDG_OUT_DATA` WebSocket upgrade, the gateway framing (WebSocket messages carrying `HTTP_DATA_PACKET` packets) is unwrapped first and the recovered inner RDP stream is replayed like a direct capture.

## Exporting frames

Run the CLI with a pcapng capture that contains the TLS key-log material required by the replay pipeline.

```shell
cargo run -p ironrdp-capture-replay --bin ironrdp-capture-replay -- capture.pcapng replay-output
```

Captures exported from Wireshark only embed TLS secrets for flows Wireshark can see.
A gateway-tunneled RDP session is a second TLS session inside the tunnel, which Wireshark does not dissect, so its secrets are missing from the exported capture.
Pass the original key log (for example an LSA `tls-lsa.log`) with `--keylog` to supply those secrets:

```shell
cargo run -p ironrdp-capture-replay --bin ironrdp-capture-replay -- --keylog tls-lsa.log capture.pcapng replay-output
```

The command succeeds only when replay produces visual framebuffer updates.
It writes PNG files zero-padded to at least six digits (`frame_000000.png`, `frame_000001.png`, ...) in replay order, along with payload-free `frame_meta.psv`, `events.tsv`, `gaps.tsv`, and `dynamic-channels.tsv` files.
Each `frame_meta.psv` row maps a sequence filename to its source packet, dimensions, and full-frame update geometry.

The output directory must be empty by default.
Pass `--replace` to replace an existing directory after a complete export is staged successfully.

The current router exports snapshots only for graphics updates it can render.
It does not synthesize frames for routing-only captures or implement unsupported graphics codecs and channels.

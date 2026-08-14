# ironrdp-capture-replay

`ironrdp-capture-replay` replays supported direct-TCP RDP captures offline and exports framebuffer snapshots without writing TLS secrets, decrypted payloads, or raw capture data.
It is an internal analysis tool built on the replay routing pipeline.

## Exporting frames

Run the CLI with a pcapng capture that contains the TLS key-log material required by the replay pipeline.

```shell
cargo run -p ironrdp-capture-replay --bin ironrdp-capture-replay -- capture.pcapng replay-output
```

The command succeeds only when replay produces visual framebuffer updates.
It writes `frame_0000.png`-style PNG files in replay order, along with payload-free `frame_meta.psv`, `events.tsv`, `gaps.tsv`, and `dynamic-channels.tsv` files.
Each `frame_meta.psv` row maps a sequence filename to its source packet, dimensions, and full-frame update geometry.

The output directory must be empty by default.
Pass `--replace` to replace an existing directory after a complete export is staged successfully.

The current router exports snapshots only for graphics updates it can render.
It does not synthesize frames for routing-only captures or implement unsupported graphics codecs and channels.

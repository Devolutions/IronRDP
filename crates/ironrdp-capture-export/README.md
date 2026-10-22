# ironrdp-capture-export

`ironrdp-capture-export` is a binary-only tool that writes rendered framebuffer frames and payload-free diagnostics from supported direct-TCP RDP captures.
It keeps the PNG encoder and filesystem export dependencies out of `ironrdp-capture-replay`, which consumers can link as a replay library.

Run it with a pcapng capture that contains the TLS key-log material required by the replay pipeline.

```shell
cargo run -p ironrdp-capture-export -- capture.pcapng replay-output
```

For gateway-tunneled RDP, pass the original NSS-compatible TLS key log.

```shell
cargo run -p ironrdp-capture-export -- --keylog tls-keys.log capture.pcapng replay-output
```

The key log remains in memory and is never written to replay output.
The command writes zero-padded PNG files and `frame_meta.psv`, `events.tsv`, `gaps.tsv`, and `dynamic-channels.tsv`.
The output directory must be empty unless `--replace` is supplied.

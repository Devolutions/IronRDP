# ironrdp-stress

Live RDP graphics-pipeline resize grading harness.

Connects to a real RDP server over TCP + TLS/CredSSP, negotiates EGFX and Display
Control, then drives resolution changes while scoring the decoded framebuffer
(black tiles, stale tiles, RemoteFX seam energy). It talks to `ironrdp-session`
directly, so a failure points at the protocol/decode path rather than at a
client's canvas plumbing.

This is an internal analysis tool, in the same class as `ironrdp-capture-replay`
and `ironrdp-bench`. It is not an example, and it is not part of `cargo test`:
it needs a live host.

```shell
cargo run -p ironrdp-stress -- \
    --host rdp.example.com -u Administrator --rounds 10 --out-dir /tmp/rdp-stress
```

The password is read from `--password` or, preferably, `RDP_PASSWORD`.

# bench-corpus

Local corpus of `IRDPREC1` captures for the record/replay benchmark. **Contents are gitignored** —
captures are screen content recorded from real RDP servers (privacy) and are large, regenerable
binaries.

Each capture is three sidecar files:

- `<name>.irdprec` — decrypted server→client active-session byte stream.
- `<name>.json` — negotiated session state (channel IDs, desktop size, compression, …).
- `<name>.checksum.json` — canonical final-framebuffer CRC32 (the deterministic correctness gate).

Record one with:

```
ironrdp-viewer -u <user> -p <pass> -d <domain> --clipboard-type disable \
  --record-traffic bench-corpus/<name>.irdprec --exit-after-secs 6 <host>
```

Replay / verify: `ironrdp-replay-bench` (native), `Devolutions.IronRdp.ReplayBench` (.NET), or
`crates/ironrdp-web/bench-harness` (web). See `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.

**Privacy gate:** only record deterministic demo VMs / synthetic content. Never commit captures.

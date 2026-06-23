# bench-corpus

Local corpus of `IRDPREC1` captures for the record/replay benchmark. **Contents are gitignored** —
captures are screen content recorded from real RDP servers (privacy) and are large, regenerable
binaries.

Each capture is three sidecar files:

- `<name>.irdprec` — decrypted server→client active-session byte stream.
- `<name>.json` — negotiated session state (channel IDs, desktop size, compression, …).
- `<name>.checksum.json` — canonical final-framebuffer CRC32 (the deterministic correctness gate).

Replay / verify: `ironrdp-replay-bench` (native), `Devolutions.IronRdp.ReplayBench` (.NET), or
`crates/ironrdp-web/bench-harness` (web). See `docs/plans/2026-06-03-ironrdp-benchmark-design.md`.

## 1080p benchmark workflow

Use a disposable, deterministic RDP test target. Pass the target and credentials explicitly.

### Record a capture

Pick a deterministic test VM or demo desktop, then record a short 1920×1080 session:

```powershell
$server = "rdp.example.test"
$username = "Administrator"
$password = "example-password"
$name = "rdp-1080p"

cargo run -p ironrdp-viewer --no-default-features --features native-tls -- `
  --desktop-width 1920 --desktop-height 1080 `
  --clipboard-type disable --no-server-pointer `
  --record-traffic "bench-corpus/$name.irdprec" `
  --exit-after-secs 8 `
  -u $username -p $password $server
```

If the server requires a separate domain, add `-d <domain>` before `-p $password`.

The recorder writes:

```text
bench-corpus/<name>.irdprec
bench-corpus/<name>.json
bench-corpus/<name>.checksum.json
```

### Native replay correctness gate

```powershell
cargo run -p ironrdp-replay-bench -- `
  --input "bench-corpus/<name>.irdprec" `
  --warmup 1 --iterations 5
```

The replay is valid only when the checksum reports `MATCH`.

### Web Canvas2D replay benchmark

```powershell
rustup target add wasm32-unknown-unknown
wasm-pack build crates/ironrdp-web --target web --dev --features bench

Push-Location crates/ironrdp-web/bench-harness
npm install
npx playwright install chromium
node run.mjs --capture /bench-corpus/<name>.irdprec --passes 15
Pop-Location
```

The wasm `rustflags` (`getrandom_backend="wasm_js"`) are already set in `.cargo/config.toml`, so no
extra `RUSTFLAGS` are needed.

The web harness prints the replay checksum, a canvas pixel hash, total time, and per-stage timings.
Use the `draw` stage to compare Canvas2D presenter changes.

**Privacy gate:** only record deterministic demo VMs / synthetic content. Never commit captures.

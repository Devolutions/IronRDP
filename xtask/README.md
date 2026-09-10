# IronRDP project automation

Free-form automation following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification.

## Benchmark capture corpus

Fetch the pinned benchmark capture corpus before running capture-based benchmarks:

```PowerShell
cargo xtask bench corpus-fetch
```

List the pinned capture identifiers, filenames, digests, and upstream scenario intent with:

```PowerShell
cargo xtask bench corpus-list
```

Replay every verified cached capture and enforce its qualified expectation with:

```PowerShell
cargo xtask bench replay
```

Use `--capture <id>` to run one manifest entry.
The command never fetches data and fails when a cache entry is missing or its digest does not match the manifest.
It prints only payload-free outcome categories and verifies every successful replay's routing counters, graphics dimensions, output fingerprint, lifecycle, and exact gap metadata fingerprint.
`cargo xtask ci` fetches and verifies the pinned corpus before replaying it.

The fetch command downloads only the files listed in `crates/ironrdp-bench/corpus.toml` from immutable raw URLs at the manifest's Git commit.
It requires the cross-platform `curl` command to be available on `PATH`.
It verifies every download and warm-cache entry with the manifest SHA-256 digest before storing files under `bench-data/wireshark-rdp/<revision>/captures`.
Keep generated benchmark output under `bench-data/benchmark-output`, separate from the corpus cache.
The ignored `bench-data` directory must never be committed.
Capture-based commands must work from the verified local cache and must not perform network I/O after this explicit fetch step.

## Codec and server encoding benchmarks

Run focused Criterion workloads with:

```PowerShell
cargo bench -p ironrdp-bench -p ironrdp-bulk --bench bench --bench bulk_compression --locked
```

The graphics fixtures contain deterministic nonzero ARGB data and expose each encoder's output length.
Bulk compression measures supported 4 KiB cold and stateful-history streams separately, resetting state before every measured repeat.
Fresh codec contexts are initialized outside the timed cold and passthrough operations, while a history operation processes four packets with one context.
The separately named 16 KiB passthrough cases document the production size threshold and do not claim to measure compression.

Build the server-encoding binary before measuring it with Hyperfine so compilation is outside the measured process:

```PowerShell
cargo build --release -p ironrdp-bench --bin perfenc --locked
hyperfine --warmup 1 '.\target\release\perfenc.exe --width 1920 --height 1080 input.rgbx'
```

`perfenc` reads headerless RGBX frames, allocates and reads each frame, then encodes it with one persistent server encoder.
Its default is unpaced and emits one final payload-free summary after it confirms that every input frame produced output.
Pass `--fps <FPS>` only for interactive playback pacing.

The capture replay workloads are intentionally qualified partial replays, not full-session success measurements.
Criterion prepares and strictly verifies only the selected cached capture before timing begins.
Each timed replay creates fresh session state and validates lifecycle, routing counters, gap metadata, and framebuffer updates without hashing every framebuffer.
The preflight verifies the full output fingerprint, while the standalone command performs that strict verification in its single replay execution.

```PowerShell
cargo bench -p ironrdp-bench --bench capture_replay -- 'partial-replay/no-nla-accepted/processing' --exact
cargo bench -p ironrdp-bench --bench capture_replay -- 'partial-replay/no-nla-smartcard/processing' --exact
cargo build --release -p ironrdp-bench --bin capture-replay-bench --locked
hyperfine --warmup 1 '.\target\release\capture-replay-bench.exe --capture no-nla-accepted'
```

An unmatched Criterion filter is not a benchmark result.
Use only `no-nla-accepted` and `no-nla-smartcard`; both are active partial replays with eight declared static-channel gaps.
Use `cargo xtask bench replay` to regression-test the complete pinned corpus, not to produce a single-capture timing score.

To update the corpus, inspect the upstream capture inventory, revise the manifest revision, inventory, scenario intent metadata, replay expectations, and SHA-256 digests together, then run `cargo xtask bench corpus-fetch` followed by `cargo xtask bench replay`.
Do not commit captures, TLS key material, decrypted payloads, screenshots, or generated output.

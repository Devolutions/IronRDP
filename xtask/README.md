# IronRDP project automation

Free-form automation following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification.

## Benchmark capture corpus

```PowerShell
cargo xtask bench corpus-fetch
cargo xtask bench corpus-list
cargo xtask bench corpus-check [--capture <ID>]
```

All three commands run the `corpus-bench` binary from `ironrdp-bench`, which owns the corpus code and its dependencies.
`corpus-fetch` uses `curl` to download the captures listed in `crates/ironrdp-bench/corpus.toml` from the pinned upstream commit and verifies each SHA-256 digest.
Verified captures are cached under the ignored `bench-data/wireshark-rdp/<revision>/captures` directory.

`corpus-check` replays every cached capture, or the selected one, with full verification and compares the result with its `expect` table in the manifest.
Captures listing the `connector` benchmark are also compared with their `[capture.connector]` table.
It never fetches captures, and a missing or corrupt cache entry fails the check.
On a mismatch, it prints the observed expectation as TOML ready to paste into the manifest, followed by at most 16 payload-free gap locations.
`cargo xtask ci` fetches the corpus before running this check.

## Benchmarks

Run the codec and bulk compression benchmarks:

```PowerShell
cargo bench -p ironrdp-bench -p ironrdp-bulk --bench bench --bench bulk_compression --locked
```

Build `perfenc` before measuring it, so compilation stays outside the measured process:

```PowerShell
cargo build --release -p ironrdp-bench --bin perfenc --locked
hyperfine --warmup 1 '.\target\release\perfenc.exe --width 1920 --height 1080 input.rgbx'
```

`perfenc` is unpaced by default; `--fps` paces it for interactive playback.

After fetching and checking the corpus, time capture replay of one capture that lists `replay` in its manifest `benchmark` kinds:

```PowerShell
cargo xtask bench replay --capture no-nla-accepted
```

The benchmark verifies the full output once, then times replays that skip hashing the output.
For a whole-process measurement, time one replay with `corpus-bench`:

```PowerShell
cargo build --release -p ironrdp-bench --bin corpus-bench --locked
hyperfine --warmup 1 '.\target\release\corpus-bench.exe replay --capture no-nla-accepted'
```

`corpus-check` rejects a capture listing `replay` unless its expectation renders an active or terminated session.

Time the production connector from the captured connection confirm to `Connected` for one capture that lists `connector`:

```PowerShell
cargo xtask bench connector --capture no-nla-accepted
```

The connector uses a client configuration derived from the captured client frames and consumes only the decrypted server frames.
It does not run the TLS handshake, CredSSP, or the session after `Connected`; the replay benchmark measures that session.
A capture listing `connector` must have a `[capture.connector]` table with the expected step count, outbound frame count, consumed server frames, and SHA-256 of the connector output.
The benchmark verifies the full output once, then times connections that skip framing and hashing it.
`corpus-bench connector --capture <ID>` runs one connection, and `--strict` adds the full verification.

To update the corpus, change the manifest revision, entries, and digests together, then run `cargo xtask bench corpus-fetch` and `cargo xtask bench corpus-check`.
Review every changed expectation before committing it; a recorded expectation describes current replay behavior, not proof of correctness.
Never commit captures, TLS key material, decrypted payloads, or generated output.

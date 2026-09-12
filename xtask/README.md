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

To update the corpus, inspect the upstream capture inventory, revise the manifest revision, inventory, scenario intent metadata, replay expectations, and SHA-256 digests together, then run `cargo xtask bench corpus-fetch` followed by `cargo xtask bench replay`.
Do not commit captures, TLS key material, decrypted payloads, screenshots, or generated output.

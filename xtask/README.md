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

The fetch command downloads only the files listed in `crates/ironrdp-bench/corpus.toml` from immutable raw URLs at the manifest's Git commit.
It verifies every download and warm-cache entry with the manifest SHA-256 digest before storing files under `dependencies/wireshark-rdp/<revision>/captures`.
Keep generated benchmark output under `dependencies/benchmark-output`, separate from the corpus cache.
The ignored `dependencies` directory must never be committed.
Capture-based commands must work from the verified local cache and must not perform network I/O after this explicit fetch step.

To update the corpus, inspect the upstream capture inventory, revise the manifest revision, inventory, scenario intent metadata, and SHA-256 digests together, then run `cargo xtask bench corpus-fetch`.
Do not commit captures, TLS key material, decrypted payloads, screenshots, or generated output.

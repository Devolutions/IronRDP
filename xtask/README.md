# IronRDP project automation

Free-form automation following [`cargo xtask`](https://github.com/matklad/cargo-xtask) specification.

## Benchmark capture corpus

```PowerShell
cargo xtask bench corpus-fetch
cargo xtask bench corpus-list
```

Both commands run the `corpus-bench` binary from `ironrdp-bench`, which owns the corpus code and its dependencies.
`corpus-fetch` uses `curl` to download the captures listed in `crates/ironrdp-bench/corpus.toml` from the pinned upstream commit and verifies each SHA-256 digest.
Verified captures are cached under the ignored `bench-data/wireshark-rdp/<revision>/captures` directory.

To update the corpus, change the manifest revision, entries, and digests together, then run `cargo xtask bench corpus-fetch`.
Never commit captures, TLS key material, decrypted payloads, or generated output.

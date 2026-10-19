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
It never fetches captures, and a missing or corrupt cache entry fails the check.
On a mismatch, it prints the observed expectation as TOML ready to paste into the manifest, followed by at most 16 payload-free gap locations.
`cargo xtask ci` fetches the corpus before running this check.

To update the corpus, change the manifest revision, entries, and digests together, then run `cargo xtask bench corpus-fetch` and `cargo xtask bench corpus-check`.
Review every changed expectation before committing it; a recorded expectation describes current replay behavior, not proof of correctness.
Never commit captures, TLS key material, decrypted payloads, or generated output.

# ironrdp-bench

Benchmark IDs are baseline keys: renaming a benchmark, its group, or a capture `id` discards measured history, so keep existing names when refactoring.

## Capture replay

A capture enters the replay benchmark by listing `replay` in its `benchmark` array in `corpus.toml`, and `Corpus::parse` rejects the listing unless the expectation renders an active or terminated session with graphics updates.
A new benchmark kind touches four places: `BENCHMARK_KINDS` and the eligibility rule in `Corpus::parse`, a `corpus-bench` subcommand, a registration loop in `benches/corpus.rs`, and an `xtask bench` subcommand.
Prepare and fully verify a capture inside its `bench_function` closure, before `b.iter`, so selecting one benchmark loads only its capture.
The timed closure runs only production work, such as `replay_with_options` without the output fingerprint; verification inside it measures the harness instead of IronRDP.
When `corpus-check` prints a replacement `[capture.expect]` table, paste it only after identifying the change that caused the drift, and name that change in the commit message.
Only `corpus-bench fetch` touches the network; every other path reads the verified `bench-data/wireshark-rdp` cache and fails when an entry is missing.
Never commit captures, TLS key logs, decrypted payloads, or screenshots; `cargo xtask check captures` rejects only tracked capture files.

## Fixtures and dependencies

Before timing, assert that each fixture exercises the intended path: bulk inputs of 16 KiB or more pass through uncompressed, and all-zero images encode trivially.
Dependencies used only by `perfenc` stay optional behind the `perfenc` feature, because the corpus tooling builds with `--no-default-features` and must not compile the server encoder or Tokio.

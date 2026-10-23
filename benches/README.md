# IronRDP benchmarks

## RemoteFX decode

Benchmarks for the RemoteFX (RFX) **decode** path, on native targets and on WASM.

### Running

Native, via the standard Criterion harness:

```sh
cargo xtask bench rfx-decode
```

Criterion accepts a regex to select a subset, so you can iterate on one stage without re-running the full suite:

```sh
cargo xtask bench rfx-decode --filter 'stage/rlgr'
```

#### Validating a change

Use Criterion baselines to compare two runs.
Save a baseline on the unmodified code, make the change, then re-run against the baseline.
This example names the baseline `before`.

```sh
# On the unmodified code.
cargo xtask bench rfx-decode --save-baseline before

# ... make the change ...

cargo xtask bench rfx-decode --baseline before
```

#### WASM

Requires Node.js:

```sh
cargo xtask wasm install
cargo xtask bench rfx-decode-wasm
```

`wasm install` adds the `wasm32-unknown-unknown` target.
The bench task installs the locked Node dependencies, builds `benches/rfx-decode-wasm` in release mode, and runs `run.mjs` on the result.
Add `--simd` to build with `+simd128`.

`run.mjs` reports whether the module was built with SIMD and how many SIMD instructions it contains.
It then times full-frame decoding and each decode stage.
Run it before and after a change to see the effect.

The runner uses raw `WebAssembly` exports rather than `wasm-bindgen` glue.
Stage inputs are restored in batches before timing; no estimated copy cost is subtracted.
Batching amortizes JS call and timer overhead.
Stage times cover one component, except YCbCr conversion, which covers all three components of a tile.

### Fixtures

Fixtures are generated at runtime.
A synthetic screen is rendered, encoded with `rfx_encode_component`, and decoded as a video-mode stream with headers sent once.
The round-trip test (`cargo test -p rfx-decode-fixtures`) guards them.

Synthetic workloads:

| pattern | content | why |
| --- | --- | --- |
| `gradient` | smooth 2D ramp | low-frequency color variation |
| `desktop` | solid background, windows, borders, taskbar | idle or lightly used session |
| `text` | dense small glyphs on white | terminal, editor, spreadsheet |
| `photo` | smooth blobs plus fine detail | wallpaper, image viewer |
| `noise` | pseudorandom | entropy-decoder stress workload |

Everything is deterministic (a fixed xorshift, integer-only rendering), so fixtures are identical across runs, machines and targets.
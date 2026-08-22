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
The bench task then installs the Node dependencies, builds `benches/rfx-decode-wasm` in release mode, and runs `run.mjs` on the result.
Add `--simd` to build with `+simd128`.

`run.mjs` reports whether the module was built with SIMD and how many SIMD instructions it contains.
It then times full-frame decoding and each decode stage.
Run it before and after a change to see the effect.

The WASM side is driven through the raw `WebAssembly` API with plain `extern "C"` exports rather than `wasm-bindgen`.
The numbers therefore measure the codec, not the bindings.

### Fixtures

Fixtures are generated at runtime.
A synthetic screen is rendered, encoded with `rfx_encode_component`, and the resulting bitstream is decoded.
The round-trip test (`cargo test -p rfx-decode-fixtures`) guards them.

Patterns, in rough order of decode cost:

| pattern | content | why |
| --- | --- | --- |
| `gradient` | smooth 2D ramp | almost everything quantizes away |
| `desktop` | solid background, windows, borders, taskbar | idle or lightly used session |
| `text` | dense small glyphs on white | terminal, editor, spreadsheet |
| `photo` | smooth blobs plus fine detail | wallpaper, image viewer |
| `noise` | pseudorandom | not realistic; bounds the entropy decoder |

Everything is deterministic (a fixed xorshift, integer-only rendering), so fixtures are identical across runs, machines and targets.
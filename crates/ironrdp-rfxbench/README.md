# ironrdp-rfxbench

Benchmarks for the RemoteFX (RFX) **decode** path, on native targets and on WASM.

`ironrdp-bench` measures the encoder. This crate measures the decoder, which runs
in WASM for browser-based clients.

## Running

Native, via the standard Criterion harness:

```sh
cargo bench -p ironrdp-rfxbench --bench rfx_decode
```

Criterion accepts a regex to select a subset, so you can iterate on
one stage without re-running the full suite:

```sh
cargo bench -p ironrdp-rfxbench --bench rfx_decode -- 'stage/rlgr'
```

### Validating a change

Use Criterion's baselines to compare two runs. First, establish a baseline. Then
make a change and re-run the benchmarks against the saved baseline. In this
example we name the baseline `before`.


```sh
# On the unmodified code.
cargo bench -p ironrdp-rfxbench --bench rfx_decode -- --save-baseline before

# ... make the change ...

cargo bench -p ironrdp-rfxbench --bench rfx_decode -- --baseline before
```

WASM:

```sh
rustup target add wasm32-unknown-unknown
cd crates/ironrdp-rfxbench/wasm
npm install

# Build with and without SIMD for comparison.
cargo build -p ironrdp-rfxbench --release --target wasm32-unknown-unknown --lib
cp ../../../target/wasm32-unknown-unknown/release/ironrdp_rfxbench.wasm build/baseline.wasm
RUSTFLAGS="-C target-feature=+simd128" \
  cargo build -p ironrdp-rfxbench --release --target wasm32-unknown-unknown --lib
cp ../../../target/wasm32-unknown-unknown/release/ironrdp_rfxbench.wasm build/simd128.wasm

node run.mjs build/baseline.wasm build/simd128.wasm
```

`run.mjs` takes one or more modules, reports the SIMD instruction counts found
in each, and times them side by side. If given exactly two modules, it also
prints the speedup of the second over the first. To validate a change on WASM,
build the modified code into a second `.wasm` and pass both.

The WASM side is driven through the raw `WebAssembly` API with plain
`extern "C"` exports rather than `wasm-bindgen`, so the numbers are the codec
and not the bindings.

## Fixtures

Fixtures are auto-generated: a synthetic screen is rendered, encoded with
`rfx_encode_component`, and the resulting bitstream is decoded.

Patterns, in rough order of decode cost:

| pattern | content | why |
| --- | --- | --- |
| `gradient` | smooth 2D ramp | almost everything quantizes away |
| `desktop` | solid background, windows, borders, taskbar | idle or lightly-used session |
| `text` | dense small glyphs on white | terminal, editor, spreadsheet |
| `photo` | smooth blobs plus fine detail | wallpaper, image viewer |
| `noise` | pseudorandom | not realistic; bounds the entropy decoder |

Everything is deterministic (a fixed xorshift, integer-only rendering) so
fixtures are identical across runs, machines and targets.

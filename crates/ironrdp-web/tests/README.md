# Test the WebSocket builder API

From the repository root, build the Node bindings and run the tests:

```sh
wasm-pack build crates/ironrdp-web --dev --target nodejs --out-dir ../../target/ironrdp-web-tests -- --locked
node --test crates/ironrdp-web/tests/websocket_protocols.cjs
```

These tests call the public WASM `SessionBuilder` API with mocked browser objects.
They cover protocol validation, configuration recovery, defensive copying, and
WebSocket error propagation without a server. They also run as part of
`cargo xtask web check` in the Web Client CI job.

## Canvas pixels

The native extraction and bounds regressions run with:

```sh
cargo test -p ironrdp-web --locked --test image --test canvas
```

`canvas_pixels.rs` runs the production rendering code in a browser, including
clipping, narrow updates, full-width copying, alpha processing, sparse first
updates, and resize. With Chromium and a compatible ChromeDriver installed, run
from this crate directory:

```sh
RUSTFLAGS='-Ctarget-feature=+simd128 --cfg getrandom_backend="wasm_js"' \
    wasm-pack test --headless --chrome --test canvas_pixels
RUSTFLAGS='-Ctarget-feature=-simd128 --cfg getrandom_backend="wasm_js"' \
    wasm-pack test --headless --chrome --test canvas_pixels
```

The `--chrome` option selects the Chromium WebDriver protocol. To select a custom
Chromium binary, supply its path as `goog:chromeOptions.binary` in `webdriver.json`;
set `CHROMEDRIVER` to the matching driver path. Clear `CARGO_ENCODED_RUSTFLAGS` if
set, since Cargo otherwise gives it precedence over `RUSTFLAGS`.

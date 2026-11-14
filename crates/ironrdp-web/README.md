# WASM bindings for web

## 🛠️ Build with `wasm-pack build`

```
wasm-pack build
```

## Tests

Run the audio and channel-attachment tests on the host with `cargo test -p ironrdp-web --lib audio`.
Run the same tests and the JavaScript callback-contract tests in WebAssembly with `wasm-pack test --node --lib --locked` from this directory.
The Wasm tests exercise JavaScript objects and typed arrays in Node.js; they do not require browser APIs or test live playback.
`cargo xtask web check` also runs these Wasm tests.

See [tests/README.md](tests/README.md) for the WebSocket builder API test instructions.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

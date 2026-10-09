# WASM bindings for web

## 🛠️ Build with `wasm-pack build`

```
wasm-pack build
```

## Test the WebSocket builder API

From the repository root, build the Node bindings and run the tests:

```sh
wasm-pack build crates/ironrdp-web --dev --target nodejs --out-dir ../../target/ironrdp-web-tests -- --locked
node --test crates/ironrdp-web/tests/websocket_protocols.cjs
```

These tests call the public WASM `SessionBuilder` API with mocked browser objects.
They cover protocol validation, configuration recovery, defensive copying, and
WebSocket error propagation without a server. They also run as part of
`cargo xtask web check` in the Web Client CI job.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

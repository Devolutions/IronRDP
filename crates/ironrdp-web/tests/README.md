# Test the web session configuration

From the repository root, build the Node bindings and run the tests:

```sh
wasm-pack build crates/ironrdp-web --dev --target nodejs --out-dir ../../target/ironrdp-web-tests -- --locked
node --test crates/ironrdp-web/tests/websocket_protocols.cjs crates/ironrdp-web/tests/display_config.cjs
```

These tests call the public WASM `SessionBuilder` API with mocked browser objects.
They cover protocol validation, configuration recovery, defensive copying, and WebSocket error propagation without a server.
The display tests inspect settings sent during connection.
The Web Client CI job runs them through `cargo xtask web check`.

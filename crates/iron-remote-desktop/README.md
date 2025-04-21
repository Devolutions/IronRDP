# Iron Remote Desktop — Helper Crate

Helper crate for building WASM modules compatible with the `iron-remote-desktop` WebComponent.

Implement the `RemoteDesktopApi` on a Rust type, and call the `make_bridge!` on
it to generate the WASM API that is expected by `iron-remote-desktop`.

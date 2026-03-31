# Iron Remote Desktop — Helper Crate

Helper crate for building WASM modules compatible with the `iron-remote-desktop` web component.

Implement the `RemoteDesktopApi` trait on a Rust type and call `make_bridge!` on it to generate
the WASM API expected by `iron-remote-desktop`.

See the `ironrdp-web` crate for a complete example.

## Design Philosophy

`iron-remote-desktop` is **protocol-agnostic**. The traits in this crate (`Session`,
`SessionBuilder`) define only features that are universal across all remote protocols:
input, rendering, clipboard, resize, and connection lifecycle.

**Protocol-specific features must not be added to these traits.** They belong in the backend
crate and must be surfaced via the extension mechanism:

- `Session::invoke_extension` / `SessionBuilder::extension` are the pass-through points.
- This crate defines a concrete `Extension` type used by the traits; each backend defines its own
  extension identifiers and payload formats and interprets the values carried inside `Extension`.
- `iron-remote-desktop` treats `Extension` as an opaque envelope and never inspects backend-specific
  extension values.

A method belongs in these traits if **either** of the following is true:

1. **The web component itself needs to call it** to implement transparent, protocol-independent
   behaviour (e.g., `supports_unicode_keyboard_shortcuts` is called by the component to adapt
   keyboard handling, without the consumer being involved).
2. **The feature is universal** — every reasonable remote protocol backend would implement it
   in a meaningful way (e.g., resize, clipboard text, cursor style).

If neither applies, the method is protocol-specific and must go through extensions.

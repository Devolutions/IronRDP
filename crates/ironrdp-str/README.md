# ironrdp-str

Typed wire-aware string primitives for RDP protocol fields.

RDP encodes strings as UTF-16LE on the wire, with three independent dimensions in how string fields are laid out:

1. **Length prefix**: none (fixed-size), `cch` (WCHAR count), or `cb` (byte count)
2. **Null terminator**: present and counted in prefix, present but not counted, or absent
3. **Multi-string**: single string vs. `MULTI_SZ`

This crate provides typed wrappers for each combination, along with foundational free functions that are the only correct source of truth for wire-length calculations.

## Architectural invariant: defer native conversion

**String values must not be eagerly converted to Rust `String` on decode.**
Every string type in this crate stores the wire representation internally — as a flat `Vec<u16>` of UTF-16 code units — and only converts to a Rust-native `String` when the caller explicitly calls `to_native()`, `to_native_lossy()`, or `into_native()`.

Two concrete benefits drive this invariant:

**Efficient decode.**
Converting UTF-16LE wire bytes to a `Vec<u16>` is a single `memcpy` (via `bytemuck` on little-endian targets).
Producing a `String` from that requires a second allocation and a full scan to validate or transcode the code units.
Many callers decode a PDU, inspect one or two fields, and discard the rest — paying for transcoding every string field up front would be wasteful.

**Zero-cost decode-encode passthrough.**
Proxy and relay components typically decode a PDU and re-encode it verbatim, possibly forwarding it to another peer.
When the wire representation is retained, re-encoding a string that was not touched is also a single `memcpy` — the bytes going out are identical to the bytes that came in.
Eager conversion to `String` would round-trip through UTF-8 and back to UTF-16LE, changing the representation unnecessarily and requiring two additional allocations per field.

### What this means in practice

- `decode_owned` / `decode` store `Wire(Vec<u16>)` internally; no `String` is ever allocated.
- `to_native()` / `into_native()` validate and allocate on demand; call them only when you need a `String`.
- `to_native_lossy()` accepts lone surrogates by replacing them with U+FFFD; prefer this for display or logging.
- Construction from Rust code (e.g. `new("hello")`) uses `Native(String)` internally; encoding that path is equally efficient because the UTF-16 units are computed lazily during `encode()`.

## Critical invariant: wire-length arithmetic

**Never use `.len()` or `.chars().count()` on a Rust `&str` to derive any wire length.**
Both are wrong for non-BMP input (e.g. U+1F600 GRINNING FACE is one scalar value but two UTF-16 code units). The only correct source is [`utf16_code_units`].

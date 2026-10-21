# WASM bindings for web

## 🛠️ Build with `wasm-pack build`

```
wasm-pack build
```

The production `cargo xtask web build` enables `simd128` for pixel copying and
alpha normalization. Builds without that feature use scalar code; this is a
compile-time fallback, so a SIMD module still requires a compatible browser.

## Canvas updates

Updates are clipped to the framebuffer before choosing a copy path. Whole-row
extraction adds at most 25% extra pixels; narrower updates keep their visible
rectangle. Full-width updates of at least 256 KiB combine copying and alpha
normalization. Uploads preserve the source framebuffer and make pixels opaque.

New and resized canvases start transparent. Untouched pixels retain their prior
Canvas values, including transparency before their first upload. Removing
incidental whole-row overdraw can therefore leave more untouched pixels transparent.

## Tests

See [tests/README.md](tests/README.md) for the WebSocket builder API test instructions.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

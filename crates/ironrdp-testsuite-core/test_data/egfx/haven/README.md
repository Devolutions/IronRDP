# Haven Server 2025 EGFX fixtures

Five binary fixtures captured from a live Windows Server 2025 (KVM) session via
[Haven](https://github.com/GlassHaven/Haven)'s `rdp-cli`, published by GlassOnTin as
release [`egfx-pr1238-fixtures-2026-04-30`](https://github.com/GlassHaven/Haven/releases/tag/egfx-pr1238-fixtures-2026-04-30).
Original capture by @GlassOnTin, recorded via Haven's dumper at `GlassHaven/Haven@bd2e52d7`,
replay tool at `GlassHaven/Haven@c6cdcc49`.

Each file is the post-ZGFX-decompressed bytes of a single `GfxPdu` (MS-RDPEGFX 2.2.2),
exactly as it appeared on the wire before decompression, unmodified from the release.

| File | PDU | Dest rect (left, top, right, bottom) | Exclusive width x height |
|------|-----|---------------------------------------|---------------------------|
| `wts1_64x64_clearcodec_tile.bin` | WireToSurface1 (ClearCodec) | (128, 192, 192, 256) | 64x64 |
| `wts1_576x128_nscodec_subregion.bin` | WireToSurface1 (ClearCodec, NSCodec sub-band) | (64, 64, 640, 192) | 576x128 |
| `wts1_64x32_taskbar_strip.bin` | WireToSurface1 (ClearCodec) | (0, 768, 64, 800) | 64x32 |
| `wts1_8x4_micro_tile.bin` | WireToSurface1 (ClearCodec) | (374, 521, 382, 525) | 8x4 |
| `create_surface_1280x800_context.bin` | CreateSurface | -- | surface 0 = 1280x800 |

`wts1_576x128_nscodec_subregion.bin`'s top-level `codec_id` is ClearCodec (0x8), same as
the others; `Codec1Type` has no separate NSCodec value at the `WireToSurface1` layer.
"nscodec" in the filename refers to an NSCodec-compressed sub-band nested inside this
particular ClearCodec payload (MS-RDPEGFX's ClearCodec residual bands can themselves be
NSCodec-encoded), not a different top-level codec selection.

## License

Haven is licensed AGPL-3.0. These files are binary captures of Microsoft's own RDP wire
protocol output (facts about the protocol as transmitted, not creative work of Haven's),
vendored the same way `ironrdp-replay-client/test_data/` vendors third-party capture data.

This contribution to IronRDP, the fixture files and the code that uses them, is licensed
`MIT OR Apache-2.0`, same as the rest of this crate.

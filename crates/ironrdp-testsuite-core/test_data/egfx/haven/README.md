# Haven Windows EGFX fixtures

Binary fixtures captured from live Windows (Server 2025 and Windows 11 24H2 KVM) sessions via
[Haven](https://github.com/GlassHaven/Haven)'s `rdp-cli`.
Original captures by @GlassOnTin, recorded via Haven's EGFX PDU dumper (`EGFX_PDU_DUMP_DIR`).

Each file is the post-ZGFX-decompressed bytes of a single `GfxPdu` (MS-RDPEGFX 2.2.2),
exactly as it appeared on the wire before decompression.

## ClearCodec Fixtures (PR #1813)

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

## Progressive RemoteFX Difference-Encoded Fixtures (Issue #1240 / PR #1698 Regression Suite)

Captures of live `WireToSurface2` PDUs carrying difference-encoded RemoteFX Progressive tiles
(`TILE_FIRST` blocks with `flags & RFX_TILE_DIFFERENCE == 1` / `0x01` per MS-RDPRFX 2.2.2.3.1.2
and 3.1.8.1.7.1), emitted during graphical refinement:

| File | PDU | Dest rect | Tiles | Tile Type & Difference Flags |
|------|-----|-----------|-------|------------------------------|
| `wts2_64x64_diff_2tiles.bin` | WireToSurface2 (RemoteFxProgressive, ctx=18) | (192, 181, 256, 245) | 2 | `TileFirst` at (3,2), (3,3): `flags=0x01` (`RFX_TILE_DIFFERENCE`), `quality=0xFF` |
| `wts2_64x128_diff_3tiles.bin` | WireToSurface2 (RemoteFxProgressive, ctx=24) | (192, 181, 256, 309) | 3 | `TileFirst` at (3,2), (3,3), (3,4): `flags=0x01` (`RFX_TILE_DIFFERENCE`), `quality=0xFF` |
| `wts2_37x560_diff_column_9tiles.bin` | WireToSurface2 (RemoteFxProgressive, ctx=7) | (1243, 192, 1280, 752) | 9 | `TileFirst` column along x=19, y=3..=11: `flags=0x01` (`RFX_TILE_DIFFERENCE`), `quality=0xFF` |
| `wts2_progressive_tile_first_mixed_25tiles.bin` | WireToSurface2 (RemoteFxProgressive, ctx=3) | (80, 192, 1280, 752) | 25 | `TileFirst` mixed set: 16 base tiles (`flags=0x00`) + 9 difference tiles (`flags=0x01`, `quality=0x00`) |

## License

Haven is licensed AGPL-3.0. These files are binary captures of Microsoft's own RDP wire
protocol output (facts about the protocol as transmitted, not creative work of Haven's),
vendored the same way `ironrdp-replay-client/test_data/` vendors third-party capture data.

This contribution to IronRDP, the fixture files and the code that uses them, is licensed
`MIT OR Apache-2.0`, same as the rest of this crate.

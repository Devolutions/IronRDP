# ironrdp-nscodec

NSCodec ([MS-RDPNSC]) implementation for IronRDP.

NSCodec is a legacy bitmap codec used in the RDP "Surface Bits" command path. It
predates RemoteFX but remains the only legacy codec advertised by the macOS
Microsoft Remote Desktop / Windows App client's bitmap codec list, so servers
wanting non-raw bitmap delivery to that client need it.

## Feature Flags

- **`encoder`** -- Opt-in; pulls in the server-side encoder
  (`ironrdp_nscodec::encoder::encode`) and `ironrdp-graphics` for the
  `PixelFormat` input enum.

With no features (`default-features = false`), the crate compiles to an empty
shell — enable `encoder` to get the actual code.

## Status

Encoder side only. Implements the codec defined in MS-RDPNSC §3.1.5:

1. RGB → YCoCg color-space conversion (lossy on chroma when CLL > 0).
2. Per-plane RLE compression (custom MS-RDPNSC byte-level RLE).
3. 20-byte frame header + concatenated Y, Co, Cg, A planes.

Chroma subsampling (`ChromaSubsamplingLevel = 1`, 4:2:0) is not yet
implemented; the encoder always emits `ChromaSubsamplingLevel = 0`.

[MS-RDPNSC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpnsc/68df0993-2c44-4d57-8aef-cdab1c1c43a8

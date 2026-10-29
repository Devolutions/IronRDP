# IronRDP EGFX native decoders

Native implementations of the `ironrdp_egfx::decode::H264Decoder` trait.

On Windows, `MediaFoundationH264Decoder` decodes with the Microsoft H.264 decoder transform shipped
with Windows (`CLSID_MSH264DecoderMFT`), so no codec has to be bundled:

- Input is an Annex B byte stream or AVC (length-prefixed) NAL units.
- The transform runs in low-latency mode, so each picture is returned for the sample that carries it.
- Output is NV12, converted to RGBA with the BT.601 limited-range matrix used by MS-RDPEVOR or the
  BT.709 full-range matrix, and padded to whole macroblocks like other `H264Decoder`s.
- `decode_yuv420` returns the planes instead, which the graphics pipeline converts itself and
  recombines for AVC444.
- Format changes mid-stream (new resolution) are followed.

The transform lives on a worker thread initialized for the multithreaded apartment, so the decoder
is `Send` and can be used from any thread. Windows editions without the Media Feature Pack have no
decoder; creating one then fails.

Other platforms currently have no native decoder.

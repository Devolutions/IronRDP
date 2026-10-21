# IronRDP RDPEVOR

`ironrdp-rdpevor` implements the client side of the Remote Desktop Protocol: Video Optimized Remoting Virtual Channel Extension ([MS-RDPEVOR]).

The server streams content it classifies as video as H.264 over two dynamic channels:
`Microsoft::Windows::RDS::Video::Control::v08.01` starts and stops presentations, and `Microsoft::Windows::RDS::Video::Data::v08.01` carries the samples.
The video is placed on the desktop with the geometry mapping it names on the [MS-RDPEGT] channel (see [`ironrdp-rdpegt`](../ironrdp-rdpegt)).

The crate provides the PDU codecs, `VideoControlClient` and `VideoDataClient`, and `VideoPresenter`, which they share:

- A Start Presentation request is answered only when the presenter obtains an H.264 decoder from its `DecoderFactory` (any `ironrdp_egfx::decode::H264Decoder`).
  Requests for another subtype are ignored, as the specification requires, so the server never streams to a client that cannot decode.
- Samples are reassembled from their packets, decoded as they complete, and scheduled from their timestamps against a caller-supplied monotonic clock.
  When several pictures are due, only the latest is shown.
- `VideoPresenter::poll` scales the due picture from the encoded size to the tracked rectangle, clips it to the visible region and to the desktop, and returns RGBA updates for the caller to composite.
- Missing or out-of-order packets and decoding failures queue a network error notification, at most one per second, so the server sends a key frame; samples are skipped until it arrives.

The crate performs no I/O.
Malformed messages are logged and ignored, reassembled samples are bounded to 16 MiB, and at most eight decoded pictures wait for their presentation time.

[MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
[MS-RDPEGT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/

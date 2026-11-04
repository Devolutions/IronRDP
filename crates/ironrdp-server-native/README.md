# IronRDP Server native backends

Native display backends producing [`ironrdp-server`] updates without adding platform I/O to the server engine.
The Windows cursor backend is enabled by the opt-in `cursor` feature; other platforms currently have no backend.

Poll `cursor::CursorCapture::capture` from the interactive session's desktop and forward each returned `DisplayUpdate` through your display backend.
`None` means the shape and visibility are unchanged.
Create a fresh capture on each client activation to resend the initial state.
Capture preserves hotspots and monochrome AND/XOR masks, including transparency and inversion.
It uses cache index zero and leaves pointer-size negotiation to the server encoder.
Capture errors invalidate the cached state so the next successful capture resends visibility or shape.
The caller chooses whether to hide the pointer while capture is unavailable.
Cursor position, injected-input echo suppression, desktop selection, polling, and transport remain the embedder's responsibility.

Alpha cursor bytes retain the native premultiplied BGRA representation.
End-to-end translucent cursor compatibility is not yet verified; IronRDP's software pointer decoder applies another premultiplication.
Synthetic bitmap tests do not establish native RDP client interoperability.
The feature is off by default and adds no Windows or server dependency on other targets.

[`ironrdp-server`]: https://docs.rs/ironrdp-server

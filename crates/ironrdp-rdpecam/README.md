# IronRDP RDPECAM

`ironrdp-rdpecam` implements the client-side protocol layer for the Remote Desktop Protocol: Video Capture Virtual Channel Extension ([MS-RDPECAM]).
It provides version 1 PDU codecs, the camera enumeration channel, per-device dynamic-channel listeners, and a backend contract for activation, media negotiation, and sample delivery.

The crate performs no device I/O.
An embedder explicitly supplies the redirected devices and implements `CameraBackend`; no camera is advertised or activated implicitly.
The codecs and state machines validate media descriptions, stream indexes, state transitions, channel IDs, and sample sizes.

The PDU codecs represent all version 1 media identifiers.
The backend state machine accepts only uncompressed YUY2, NV12, I420, RGB24, and RGB32 samples with their exact packed frame size.
H.264 and MJPEG capture are not enabled because this layer does not validate complete compressed pictures.
Media-type lists are bounded to 4,096 entries and sample messages to 64 MiB.
Version 2 camera-control properties and a native capture backend are not implemented.

The IronRDP ActiveX control neither registers the RDPECAM channels nor provides a native capture backend, so camera redirection remains disabled there.

[MS-RDPECAM]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

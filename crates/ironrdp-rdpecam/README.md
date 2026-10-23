# IronRDP RDPECAM

`ironrdp-rdpecam` implements the client-side protocol layer for the Remote Desktop Protocol: Video Capture Virtual Channel Extension ([MS-RDPECAM]).
It provides version 1 and version 2 PDU codecs, the camera enumeration channel, per-device dynamic-channel listeners, and a backend contract for activation, media negotiation, device properties, and sample delivery.

The crate performs no device I/O.
An embedder explicitly supplies the redirected devices and implements `CameraBackend`; no camera is advertised or activated implicitly.
The codecs and state machines validate media descriptions, stream indexes, state transitions, channel IDs, and sample sizes.

The client requests version 2 and accepts version 1 from older servers.
Every device-channel message carries the negotiated version (`DeviceMessage`), and the version 2 property messages are rejected on a version 1 channel.
Property requests are answered from the backend's property list: unknown property sets get `SetNotFound`, unknown properties `ItemNotFound`, and out-of-range or unsupported-mode writes `InvalidRequest`.

Windows opens several device channels with the same name at once, activating the device on one and sending requests on others.
All the channels of a device therefore share one backend and one device state (activation count and selected streams), and each channel releases the activations it requested when it closes.

Sample Requests are answered asynchronously.
`start_streams` hands the backend a `SampleSink` that counts the server's outstanding requests per stream and sends exactly one Sample Response or Sample Error Response per request, on the channel that sent the latest request, through a `SampleUplink` callback, so capture threads deliver frames as they arrive.
At most 64 requests may be outstanding per stream.

The state machine accepts uncompressed YUY2, NV12, I420, RGB24, and RGB32 samples with their exact packed frame size, and compressed H.264 and MJPEG samples.
Compressed samples are bounded and must start with an Annex B start code (H.264) or a JPEG start-of-image marker (MJPEG); their payload is decoded by the server.
Media-type lists are bounded to 4,096 entries, property lists to 256 entries, and sample messages to 64 MiB.

The Windows Media Foundation capture backend lives in [`ironrdp-rdpecam-native`](../ironrdp-rdpecam-native).

[MS-RDPECAM]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

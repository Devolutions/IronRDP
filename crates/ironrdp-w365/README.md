# ironrdp-w365

`ironrdp-w365` implements the client side of three dynamic virtual channels that Windows 365 Cloud PCs open to mirror the physical device:

- `Microsoft::Windows::RDS::BatteryStatusRedirection` shows the local battery state in the Cloud PC taskbar.
- `Microsoft::Windows::RDS::PowerRequestRedirection` asks the physical device to stay awake while the Cloud PC holds power requests, such as during a presentation.
- `Microsoft::Windows::RDS::ScreenCaptureRedirection` uploads a screenshot taken on the client so that the Cloud PC saves it.

No Microsoft Open Specification documents these channels.
The PDU layouts and the client behavior follow the Windows App 2.0.1485.0 client.
`PROTOCOL.md`, next to this README in the repository, describes the layouts and the client behavior.

Each module provides the PDU codecs and a `DvcProcessor`.
The crate performs no I/O: it reads no battery state, holds no power request, and captures no screen.
The integrating client supplies battery readings and screenshots, acts on power-request changes and save results through handler traits, and delivers the returned channel messages through its session transport.
Windows App does not open the battery channel in RemoteApp sessions, so a client registers it only for full desktops.

# ironrdp-w365

`ironrdp-w365` implements the client side of four dynamic virtual channels that Windows 365 Cloud PCs open:

- `Microsoft::Windows::RDS::BatteryStatusRedirection` shows the local battery state in the Cloud PC taskbar.
- `Microsoft::Windows::RDS::PowerRequestRedirection` asks the physical device to stay awake while the Cloud PC holds power requests, such as during a presentation.
- `Microsoft::Windows::RDS::ScreenCaptureRedirection` uploads a screenshot taken on the client so that the Cloud PC saves it.
- `Microsoft::Windows::RDS::VirtualDesktopControl::Switch` lets a Cloud PC in Boot or Switch mode send the user back to the local desktop and open local Windows Settings pages, such as Bluetooth pairing, on the physical device.

No Microsoft Open Specification documents these channels.
The PDU layouts and the client behavior follow the Windows App 2.0.1485.0 client.
`PROTOCOL.md`, next to this README in the repository, describes the layouts and the client behavior, along with the interactive Entra authentication channel that `ironrdp-client` implements.

Each module provides the PDU codecs and a `DvcProcessor`.
The crate performs no I/O: it reads no battery state, holds no power request, captures no screen, and opens no settings page.
The integrating client supplies battery readings and screenshots, acts on power-request changes, save results, and switch requests through handler traits, and delivers the returned channel messages through its session transport.
Windows App does not open the battery channel in RemoteApp sessions, so a client registers it only for full desktops.
A client registers the virtual desktop control channel only when it runs in a Boot or Switch context, and supplies the launch URI that advertises which local actions it supports.

Known limitation: when the server answers the virtual desktop control version request with a version outside the range the client offers, Windows App closes the channel, but a `DvcProcessor` cannot close its own channel.
The processor instead stays open and ignores every later PDU, so the Cloud PC's requests go unanswered until the server closes the channel.

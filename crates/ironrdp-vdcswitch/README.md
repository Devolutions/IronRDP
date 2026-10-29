# ironrdp-vdcswitch

`ironrdp-vdcswitch` implements the client side of the `Microsoft::Windows::RDS::VirtualDesktopControl::Switch` dynamic virtual channel.
Windows 365 Cloud PCs use it in Boot and Switch mode to send the user back to the local desktop and to open local Windows Settings pages, such as Bluetooth pairing, on the physical device.

No Microsoft Open Specification documents this channel.
The PDU layouts and the client behavior follow the Windows App 2.0.1485.0 client.
`PROTOCOL.md`, next to this README in the repository, describes the layouts and the client behavior.

The crate provides the PDU codecs and a `DvcProcessor` that negotiates the protocol version and passes server requests to a caller-supplied `VdcSwitchHandler`.
It performs no I/O and opens no settings page itself: the handler decides what switching to the local desktop and each local action mean on the host.
An integrating client registers the processor only when it runs in a Boot or Switch context, supplies the launch URI that advertises which local actions it supports, and delivers the returned channel messages through its session transport.

Known limitation: when the server answers with a protocol version outside the range the client offers, Windows App closes the channel, but a `DvcProcessor` cannot close its own channel.
The processor instead stays open and ignores every later PDU, so the Cloud PC's requests go unanswered until the server closes the channel.

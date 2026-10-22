# IronRDP RDPEUSB native backend

Host USB device backend for the [MS-RDPEUSB] (URBDRC) client in `ironrdp-rdpeusb`, built on [nusb], and on Windows on the in-box RemoteFX USB redirection drivers when policy allows it.

`urbdrc_listener` returns the DVC listener to register on the DRDYNVC client.
The backend runs USB I/O on its own threads and reports through `UrbdrcEvent`s, which the session loop applies to the channels it owns: pending completions, `ADD_VIRTUAL_CHANNEL` for hotplugged devices, and channel closes for unplugged or retracted devices.
An event can still be queued after its device channel was closed or its connection replaced by a reconnection that reuses channel IDs; `UrbdrcEvent::into_current` returns `None` for those.
`ironrdp-client` wires this up behind its `usb` feature.

## Device selection

`UsbRedirectionConfig` decides which devices are redirected.
It is parsed from the FreeRDP `/usb` rule syntax (`id:VID:PID#...`, `addr:BUS:ADDR#...`, `auto`, `filter:CLASS#...` or `filter:none`, with hexadecimal numbers), or from the `.rdp` `usbdevicestoredirect` property (`*`, `USB\VID_xxxx&PID_yyyy` hardware IDs, `USB\VID_xxxx&PID_yyyy\INSTANCE` device instance IDs, and `-` prefixed exclusions).
A device instance ID selects that single device: on Windows it is compared with the PnP instance ID, elsewhere with one built from the serial number, so on Linux and macOS it only matches devices whose Windows instance ID is their serial number.
Entries without a product ID are rejected, because they would select every device of the vendor ahead of the classes kept local.
`auto` and `*` keep audio, HID, mass storage, hub and smart card devices local by default, as FreeRDP does: those are better served by the high-level redirections, and redirecting them would take the local keyboard and mouse away.
Hubs are never redirected.
Device setup class GUIDs in `usbdevicestoredirect` cannot be matched from USB descriptors and are skipped.
`local_devices` lists the attached devices with the values the rules match against.

## Device coverage

RDPEUSB redirects whole devices, so a device is only offered to the server when every interface of its active configuration can be claimed.
What can be claimed depends on the platform, because nusb uses the generic user-mode USB API of each OS.

- **Windows, with RemoteFX USB redirection allowed**: any device can be redirected, through the drivers mstsc uses (see [below](#windows-remotefx-usb-redirection)).
- **Windows, otherwise**: only devices whose function driver is **WinUSB** can be redirected, or composite devices (`usbccgp`) whose interfaces all use WinUSB.
  Devices with WinUSB firmware descriptors (Microsoft OS descriptors), such as many development boards, DFU and ADB/fastboot interfaces, work out of the box.
  Other devices need their driver replaced with WinUSB first, for example with [Zadig], which requires administrator rights and makes the device unusable locally until the original driver is restored.
  Devices owned by Windows class drivers are not reachable: HID (keyboards, mice, security keys), mass storage, smart card readers, printers, audio, cameras, USB serial and Bluetooth.
- **Linux**: kernel drivers are detached from the claimed interfaces, so most devices can be redirected; the user needs write access to `/dev/bus/usb`, usually through a udev rule.
  The kernel driver is reattached when the redirection ends.
- **macOS**: devices claimed by a kernel or system extension are not reachable, which covers most class devices.

## Windows RemoteFX USB redirection

Windows ships the drivers mstsc redirects USB devices with: a filter on every USB hub (`TsUsbFlt`) and a generic redirection driver (`TsUsbGD`).
A process claims devices through the filter, which has them re-enumerated with the generic driver in place of their own; the generic driver then takes the server's `TS_URB`s as they are and submits them to the USB stack.
This reaches every device, including class devices, and supports isochronous transfers and the cancellation of single requests.
Their control interface is undocumented: this backend follows the behavior of the Windows 11 drivers.
It was tested by redirecting a YubiKey 5 to Windows Server 2025, whose OTP, FIDO and CCID interfaces all enumerated in the session.

The `fUsbRedirectionEnableMode` policy decides who may claim devices: 0 (the default) nobody, 1 elevated administrators, 2 every user.
It is set by the Group Policy "Allow RDP redirection of other supported RemoteFX USB devices from this computer" (Computer Configuration > Administrative Templates > Windows Components > Remote Desktop Services > Remote Desktop Connection Client > RemoteFX USB Device Redirection), and takes effect after a restart.
When the current process may not claim devices, the nusb backend is used.

- The redirection rules select devices before they are claimed, from their descriptors; `ADD_DEVICE` is built from the same descriptors.
- Only devices plugged directly into a filtered hub can be claimed; the filter itself refuses hubs and some keyboards and mice.
- Keystrokes from a redirected keyboard interface, such as a security key's OTP, reach the server's console rather than the RDP session, as with mstsc.
- The server offers USB redirection only with the RD Session Host role installed and Plug and Play redirection allowed (`fDisablePNPRedir` unset or 0); a server in remote administration mode does not.
- A claimed device is unusable locally until it is released: when its device channel closes, when the server retracts it (it is then not claimed again during the connection), or when the connection ends.
- Claimed devices are looked for every 500 ms and newly plugged devices every 2 s.
- No-ack isochronous OUT transfers that fail are not followed by a pipe reset, unlike mstsc.

## Limitations

These apply to the nusb backend.

- Isochronous transfers (audio and video streaming) are rejected per packet with `USBD_STATUS_NOT_SUPPORTED`, and no-ack isochronous OUT transfers are dropped.
- The active configuration cannot be changed: WinUSB does not allow it, and on other platforms it would invalidate every claimed interface.
  Selecting the active configuration, and alternate settings, works.
- Port reset and cycle requests are acknowledged without resetting the device, like the FreeRDP client, because re-enumeration would end the redirection.
- nusb cancels bulk and interrupt transfers per endpoint.
  When the server cancels one request, other transfers cancelled with it resume in order, preserving the data already moved.
  An IN transfer that received a short packet or filled its buffer, or an OUT transfer that wrote everything, completes instead.
- nusb cannot cancel a control transfer once it is submitted.
  Cancelling one completes it at once with `USBD_STATUS_CANCELED`, but the request still runs on the device until it completes or times out.
  A control transfer without a timeout, or with an infinite one, is given 5 minutes.
  Cancelled controls retain their budget and device claims until the native operation finishes; late results cannot complete a reused request ID.
- Each device has a 64 MiB budget for requests in flight, charged with each request's buffer plus 1 KiB, which also caps them at 65536; requests beyond that fail with `USBD_STATUS_INSUFFICIENT_RESOURCES`.
- IN transfers are limited to 16 MiB.
  Data a device sends beyond the requested length terminates its device channel, as MS-RDPEUSB requires.
- On Windows, interface-directed Microsoft OS feature descriptor requests are rejected with `USBD_STATUS_NOT_SUPPORTED` unless the feature index matches the interface number, because WinUSB refuses them otherwise.
- Windows reads device-level descriptors through the hub driver, and on Windows hotplugged devices are opened after a short delay so that their driver can bind first.
- The bus number in `addr:` rules is the platform bus number on Linux and macOS.
  On Windows it is the position of the host controller among the controllers present, sorted by instance ID, so it changes when a controller (a dock, for example) is added or removed; `local_devices` shows the current numbers.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125
[nusb]: https://docs.rs/nusb
[Zadig]: https://zadig.akeo.ie/

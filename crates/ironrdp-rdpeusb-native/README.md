# IronRDP RDPEUSB native backend

Host USB device backend for the [MS-RDPEUSB] (URBDRC) client in `ironrdp-rdpeusb`, built on [nusb].

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

- **Windows**: only devices whose function driver is **WinUSB** can be redirected, or composite devices (`usbccgp`) whose interfaces all use WinUSB.
  Devices with WinUSB firmware descriptors (Microsoft OS descriptors), such as many development boards, DFU and ADB/fastboot interfaces, work out of the box.
  Other devices need their driver replaced with WinUSB first, for example with [Zadig], which requires administrator rights and makes the device unusable locally until the original driver is restored.
  Devices owned by Windows class drivers are not reachable: HID (keyboards, mice, security keys), mass storage, smart card readers, printers, audio, cameras, USB serial and Bluetooth.
  mstsc reaches those through the RemoteFX USB filter driver (`tsusbflt`), whose control interface is private to the Remote Desktop client and cannot be reused.
- **Linux**: kernel drivers are detached from the claimed interfaces, so most devices can be redirected; the user needs write access to `/dev/bus/usb`, usually through a udev rule.
  The kernel driver is reattached when the redirection ends.
- **macOS**: devices claimed by a kernel or system extension are not reachable, which covers most class devices.

## Limitations

- Isochronous transfers (audio and video streaming) are rejected per packet with `USBD_STATUS_NOT_SUPPORTED`, and no-ack isochronous OUT transfers are dropped.
- The active configuration cannot be changed: WinUSB does not allow it, and on other platforms it would invalidate every claimed interface.
  Selecting the active configuration, and alternate settings, works.
- Port reset and cycle requests are acknowledged without resetting the device, like the FreeRDP client, because re-enumeration would end the redirection.
- nusb cancels bulk and interrupt transfers per endpoint. When the server cancels one request, the other transfers pending on that endpoint that had not moved any data are submitted again in order, and partially completed ones are reported as short transfers.
- nusb cannot cancel a control transfer once it is submitted. Cancelling one only drops its completion; the request still runs on the device until it completes or times out (30 seconds unless the server gave a timeout).
- IN transfers are limited to 16 MiB, and data a device sends beyond the requested length is reported as `USBD_STATUS_BUFFER_OVERRUN`.
- On Windows, interface-directed Microsoft OS feature descriptor requests are rejected with `USBD_STATUS_NOT_SUPPORTED` unless the feature index matches the interface number, because WinUSB refuses them otherwise.
- Windows reads device-level descriptors through the hub driver, and on Windows hotplugged devices are opened after a short delay so that their driver can bind first.
- The bus number in `addr:` rules is the platform bus number on Linux and macOS. On Windows it is the position of the host controller among the controllers present, sorted by instance ID, so it changes when a controller (a dock, for example) is added or removed; `local_devices` shows the current numbers.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[MS-RDPEUSB]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125
[nusb]: https://docs.rs/nusb
[Zadig]: https://zadig.akeo.ie/

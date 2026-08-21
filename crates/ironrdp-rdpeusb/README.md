# IronRDP RDPEUSB

Implements [Remote Desktop Protocol: USB Devices Virtual Channel Extension][spec]
used to redirect USB devices from a terminal client to a terminal server.

The `usb` module translates protocol-independent operations and descriptor
semantics from `ironrdp-usb` into complete backend-facing RDPEUSB transfer
requests. It is sans-I/O and does not allocate request IDs or manage pending
request lifetimes.

[spec]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

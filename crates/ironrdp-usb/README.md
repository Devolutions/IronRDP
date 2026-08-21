# IronRDP USB

Protocol-independent USB data structures and sans-I/O semantics for IronRDP.

This foundational crate provides USB-standard data models, parsing, validation, and query operations shared by protocol adapters.
It describes USB operations and data, but does not execute them: it performs no device I/O, and knows nothing about usbredir, RDPEUSB, async runtimes, request routing, or server state.

The crate is `no_std` and dependency-free.
Descriptor views borrow their input, while transfer types are generic over caller-owned buffer and packet storage.
The crate itself does not allocate.

Parsing is limited to byte layouts defined by USB itself, such as setup packets and descriptors.
Parsing usbredir packets, RDPEUSB PDUs, or any other transport framing belongs in the corresponding protocol crate.

The typed descriptor model currently follows the device framework of [USB 2.0], described in its chapter 9.
USB 3.x values and descriptor identities remain representable and losslessly traversable, but detailed SuperSpeed descriptor semantics are intentionally deferred until a non-RDPEUSB consumer requires them.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP
[USB 2.0]: https://www.usb.org/document-library/usb-20-specification

# IronRDP RDPEGT

`ironrdp-rdpegt` implements the client side of the Remote Desktop Protocol: Geometry Tracking Virtual Channel Extension ([MS-RDPEGT]).

The server uses the `Microsoft::Windows::RDS::Geometry::v08.01` channel to tell the client where content rendered on the client, such as video sent over [MS-RDPEVOR], belongs on the virtual desktop.
The crate provides the `MAPPED_GEOMETRY_PACKET` codec and `GeometryClient`, which keeps the active geometry mappings and reports changes to an optional observer.

The crate performs no I/O.
Malformed packets are logged and ignored.
Rectangles must be ordered, region data must use `RDH_RECTANGLES`, region rectangle lists are bounded to 4,096 entries, and at most 1,024 mappings are tracked.

[MS-RDPEGT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/
[MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/

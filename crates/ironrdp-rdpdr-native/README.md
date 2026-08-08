# IronRDP RDPDR native backends

Native RDPDR backend implementations.

On Windows, `WindowsRdpdrBackendFactory` configures one explicitly selected
logical volume root for the portable `ironrdp-rdpdr` backend contract. The
initial-drive name returned by `initial_drives` must be passed to
`Rdpdr::with_drives`.

The Windows implementation is deliberately narrow. It supports handle-relative
create/open, close, synchronous bounded reads and writes, and basic file
metadata queries and updates. It does not implement directory enumeration,
notifications, locks, security descriptors, streams, control requests, or
volume queries. Those requests receive `STATUS_NOT_SUPPORTED`.

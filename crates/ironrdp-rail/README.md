# IronRDP RAIL

Direction-agnostic wire codec for the `RAIL` static virtual channel specified by [MS-RDPERP] section 2.2.2.

The default `std` feature integrates encoded PDUs with `ironrdp-svc`.
The `alloc` feature supports `no_std` consumers with heap allocation.

[MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/

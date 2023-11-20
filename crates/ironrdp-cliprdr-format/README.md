# IronRDP CLIPRDR formats decoding/encoding library

This Library provides the conversion logic between RDP-specific clipboard formats and
widely used formats like PNG for images, plain string for HTML etc.

### Overflows

This crate has been audited by us and is guaranteed overflow-free on 32 and 64 bits architectures.
It would be easy to cause an overflow on a 16-bit architecture.
However, it’s hard to imagine an RDP client running on such machines.
Size of pointers on such architectures greatly limits the maximum size of the bitmap buffers.
It’s likely the RDP client will choke on a big payload before overflowing because of this crate.

# IronRDP CLIPRDR formats decoding/encoding library

This Library provides the conversion logic between RDP-specific clipboard formats and
widely used formats like PNG for images, plain string for HTML etc.

### INVARIANTS
- This crate expects the target machine's pointer size (usize) to be equal or greater than 32bits
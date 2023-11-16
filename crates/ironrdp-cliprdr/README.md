# IronRDP CLIPRDR

Implementation of clipboard static virtual channel(`CLIPRDR`) described in `MS-RDPECLIP`

This library includes:
- Clipboard SVC PDUs parsing
- Clipboard SVC processing
- Clipboard backend API types for implementing OS-specific clipboard logic

For concrete native clipboard backend implementations, see `ironrdp-cliprdr-native` crate.

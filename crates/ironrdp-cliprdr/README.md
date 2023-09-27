# IronRDP CLIPRDR

Implementation of cliboard static virtual channel(`CLIPRDR`) described in `MS-RDPECLIP`

This library includes:
- Cliboard SVC PDUs parsing
- Clipboard SVC processing
- Clipboard backend API types for implementing OS-specific clipboard logic

For concrete native clipboard backend implementations, see `ironrdp-cliprdr-native` crate.
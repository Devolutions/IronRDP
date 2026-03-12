# ironrdp-egfx

Graphics Pipeline Extension ([MS-RDPEGFX]) implementation for IronRDP.

Provides PDU types and client/server processors for the Display Pipeline Virtual
Channel Extension, including H.264/AVC420 and AVC444 video streaming support.

## Features

### `openh264`

Enables the built-in `OpenH264Decoder` implementation of the `H264Decoder` trait.
This is the base feature; enable one of the sub-features below to select how
OpenH264 is loaded:

- **`openh264-bundled`** -- Compiles OpenH264 from C source at build time
  (requires a C compiler and NASM). No patent coverage applies to binaries
  built this way.

- **`openh264-libloading`** -- Loads a prebuilt OpenH264 shared library at
  runtime via `dlopen`/`LoadLibrary`. When using Cisco's official prebuilt
  binaries (downloaded separately by the end user), those binaries carry
  patent coverage under Cisco's OpenH264 license.

### Choosing between bundled and libloading

Applications that need H.264 patent coverage (e.g., distributing via
package managers) should use `openh264-libloading` and arrange for the
end user to download the Cisco binary separately. See the
[openh264 crate documentation](https://docs.rs/openh264) for details
on library discovery and hash verification.

Applications in controlled environments (CI, development, testing) can
use `openh264-bundled` for a simpler build with no external dependencies
beyond a C toolchain.

[MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0

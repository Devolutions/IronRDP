# OpenH264 Licensing in ironrdp-egfx

## TL;DR

- Use the `openh264` feature (default: `libloading`) for production builds
- Use the `openh264-source` feature ONLY for development and testing
- Compiling from source provides ZERO patent coverage from Cisco

## Background

H.264/AVC is covered by patents administered by Via Licensing Alliance.
Cisco pays all royalties for their precompiled OpenH264 binaries, making
them free for end users. This patent coverage applies ONLY to binaries
distributed by Cisco — compiling from source gives you the BSD-2-Clause
copyright license but no patent license.

## Features

### `openh264` (production)

Loads Cisco's precompiled OpenH264 binary at runtime via `libloading`.
The binary must be installed separately on the system (e.g., via the
distribution's package manager).

For Cisco's patent coverage to apply, all four conditions must be met:

1. **Separate download**: The binary is downloaded independently
2. **User control**: The user can enable/disable it
3. **Attribution**: Display "OpenH264 Video Codec provided by Cisco Systems, Inc."
4. **License reproduction**: Include the full license text

The `OPENH264_ATTRIBUTION` constant is provided for condition 3.

### `openh264-source` (development/testing only)

Compiles OpenH264 from C source at build time. Convenient for testing
but provides NO patent coverage from Cisco. Never use in shipped products.

## System Library Paths

The decoder searches these paths (in order):

- Debian/Ubuntu: `/usr/lib/x86_64-linux-gnu/libopenh264.so*`
- Fedora/RHEL: `/usr/lib64/libopenh264.so*`
- Arch: `/usr/lib/libopenh264.so*`
- Flatpak: `/usr/lib/extensions/openh264/extra/lib/libopenh264.so`

Use `OpenH264DecoderConfig::library_path` to specify a custom path.

## References

- [Cisco Binary License](https://www.openh264.org/BINARY_LICENSE.txt)
- [OpenH264 FAQ](https://www.openh264.org/faq.html)
- [Via LA AVC Patent Portfolio](https://via-la.com/licensing-programs/avc-h-264/)

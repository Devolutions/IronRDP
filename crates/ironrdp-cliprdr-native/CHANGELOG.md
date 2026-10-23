# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.8.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.7.0...ironrdp-cliprdr-native-v0.8.0)] - 2026-10-23

### <!-- 1 -->Features

- Add the Linux `data_control` module, a Wayland clipboard client for compositors offering `ext-data-control-v1` or `wlr-data-control-unstable-v1`, with delayed rendering ([#2055](https://github.com/Devolutions/IronRDP/issues/2055)) ([a1efb0bb85](https://github.com/Devolutions/IronRDP/commit/a1efb0bb85e1654ba91f1407e77050082ee3d7d2))

- Add `LinuxClipboard`, a Linux CLIPRDR backend for text and images over Wayland data-control with an X11/XWayland fallback ([#2016](https://github.com/Devolutions/IronRDP/issues/2016)) ([aed5ce8ba2](https://github.com/Devolutions/IronRDP/commit/aed5ce8ba225c6d2b17ab39b0d12b3fc65e565bd))

- Add `WinClipboard::with_file_transfer` to copy files in both directions through the Windows clipboard; `WinClipboard::new` leaves file transfer off ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e))

### <!-- 4 -->Bug Fixes

- Stop `WinClipboard` advertising handle-based formats, local paths, shell items and OLE storage formats, which it cannot deliver ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e))

- Queue backend events that arrive while `WinClipboard` waits for remote data instead of dropping them ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e))

- Free the `WinClipboard` window subclass state on drop, and no longer use it after `WM_DESTROY` freed it ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e))

- Treat `CLIPBRD_E_CANT_OPEN` as `WinCliprdrError::ClipboardAccessDenied` ([#2179](https://github.com/Devolutions/IronRDP/issues/2179)) ([b9d1400033](https://github.com/Devolutions/IronRDP/commit/b9d140003323f3ae3cd22f91be556e58ed43322e))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-cliprdr` public dependency to 0.8




## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.6.0...ironrdp-cliprdr-native-v0.7.0)] - 2026-07-10

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-cliprdr` public dependency to 0.7



## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.5.0...ironrdp-cliprdr-native-v0.6.0)] - 2026-05-27

### <!-- 1 -->Features

- Implement clipboard file transfer support ([#1166](https://github.com/Devolutions/IronRDP/issues/1166)) ([c98a8fb774](https://github.com/Devolutions/IronRDP/commit/c98a8fb7741986e9afef00cb5615250c963a7fa9)) 

  Add end-to-end clipboard file transfer (upload and download) across the
  CLIPRDR channel per MS-RDPECLIP.

## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.4.0...ironrdp-cliprdr-native-v0.5.0)] - 2025-12-18

### <!-- 4 -->Bug Fixes

- Prevent window class registration error on multiple sessions ([#1047](https://github.com/Devolutions/IronRDP/issues/1047)) ([a2af587e60](https://github.com/Devolutions/IronRDP/commit/a2af587e60e869f0235703e21772d1fc6a7dadcd)) 

  When starting a second clipboard session, `RegisterClassA` would fail
  with `ERROR_CLASS_ALREADY_EXISTS` because window classes are global to
  the process. Now checks if the class is already registered before
  attempting registration, allowing multiple WinClipboard instances to
  coexist.

### <!-- 7 -->Build

- Bump windows from 0.61.3 to 0.62.1 ([#1010](https://github.com/Devolutions/IronRDP/issues/1010)) ([79e71c4f90](https://github.com/Devolutions/IronRDP/commit/79e71c4f90ea68b14fe45241c1cf3953027b22a2)) 

## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.3.0...ironrdp-cliprdr-native-v0.4.0)] - 2025-08-29

### <!-- 4 -->Bug Fixes

- Map `E_ACCESSDENIED` WinAPI error code to `ClipboardAccessDenied` error (#936) ([b0c145d0d9](https://github.com/Devolutions/IronRDP/commit/b0c145d0d9cf2f347e537c08ce9d6c35223823d5)) 

  When the system clipboard updates, we receive an `Updated` event. Then
  we try to open it, but we can get `AccessDenied` error because the
  clipboard may still be locked for another window (like _Notepad_). To
  handle this, we have special logic that attempts to open the clipboard
  in the event of such errors.
  The problem is that so far, the `ClipboardAccessDenied` error was not mapped.

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.3...ironrdp-cliprdr-native-v0.1.4)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.2...ironrdp-cliprdr-native-v0.1.3)] - 2025-02-03

### <!-- 4 -->Bug Fixes

- Handle `WM_ACTIVATEAPP` in `clipboard_subproc` ([#657](https://github.com/Devolutions/IronRDP/issues/657)) ([9b2926ea12](https://github.com/Devolutions/IronRDP/commit/9b2926ea1212d3f9dec9354334d5bdaa1bebd81e)) 

  Previously, the function handled only `WM_ACTIVATE`.

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.1...ironrdp-cliprdr-native-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo ([#631](https://github.com/Devolutions/IronRDP/issues/631)) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-native-v0.1.0...ironrdp-cliprdr-native-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

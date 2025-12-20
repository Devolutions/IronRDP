# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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

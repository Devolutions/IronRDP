# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.2.0...ironrdp-cliprdr-format-v0.3.0)] - 2026-10-23

### <!-- 1 -->Features

- Add `validate_dib` and `validate_dibv5` to check a `CF_DIB` or `CF_DIBV5` payload and return its length ([#1794](https://github.com/Devolutions/IronRDP/issues/1794)) ([c7766c5a56](https://github.com/Devolutions/IronRDP/commit/c7766c5a56ba668a93737f2ca384fc2b63546376))

- Add `validate_cf_html` to check a `CF_HTML` payload and return its length ([#1794](https://github.com/Devolutions/IronRDP/issues/1794)) ([c7766c5a56](https://github.com/Devolutions/IronRDP/commit/c7766c5a56ba668a93737f2ca384fc2b63546376))

### <!-- 4 -->Bug Fixes

- Reject DIBs with an embedded color profile, a mismatched `biSizeImage`, or truncated pixel data in `dib_to_png` and `dibv5_to_png` ([#1794](https://github.com/Devolutions/IronRDP/issues/1794)) ([c7766c5a56](https://github.com/Devolutions/IronRDP/commit/c7766c5a56ba668a93737f2ca384fc2b63546376))

- Validate the headers, offsets and fragment markers of a `CF_HTML` payload in `cf_html_to_plain_html`, which now rejects payloads missing `Version`, `StartHTML` or `EndHTML` ([#1794](https://github.com/Devolutions/IronRDP/issues/1794)) ([c7766c5a56](https://github.com/Devolutions/IronRDP/commit/c7766c5a56ba668a93737f2ca384fc2b63546376))

- Convert 16-bit and grayscale PNG images correctly in `png_to_cf_dib` and `png_to_cf_dibv5` ([#1877](https://github.com/Devolutions/IronRDP/issues/1877)) ([a59b3b687d](https://github.com/Devolutions/IronRDP/commit/a59b3b687d8ec81cfb79bd6e91e7d3341c866b6b))

### <!-- 7 -->Build

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))

- [**breaking**] Update `ironrdp-core` public dependency to 0.3



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.1.4...ironrdp-cliprdr-format-v0.2.0)] - 2026-05-27

### <!-- 7 -->Build

- Update `ironrdp-core` public dependency to 0.2 ([#965](https://github.com/Devolutions/IronRDP/issues/965)) ([630525deae](https://github.com/Devolutions/IronRDP/commit/630525deae92f39bfed53248ab0fec0e71249322)) 


## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.1.3...ironrdp-cliprdr-format-v0.1.4)] - 2025-09-04

### <!-- 7 -->Build

- Bump png from 0.17.16 to 0.18.0 (#961) ([21fa028dff](https://github.com/Devolutions/IronRDP/commit/21fa028dffa5f9bb1498b4d48d063ea42929faf5)) 

## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.1.2...ironrdp-cliprdr-format-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.1.1...ironrdp-cliprdr-format-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 


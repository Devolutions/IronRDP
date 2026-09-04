# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-cliprdr-format-v0.2.0...ironrdp-cliprdr-format-v0.3.0)] - 2026-09-04

### <!-- 1 -->Features

- [**breaking**] Populate decode/encode error offsets from cursor positions ([#1275](https://github.com/Devolutions/IronRDP/issues/1275)) ([8607ac5d1c](https://github.com/Devolutions/IronRDP/commit/8607ac5d1c2ea14efcac02921e54d951ab1045ec)) 

  ## Summary
  
  The workspace sweep that follows #1266. Decode and encode error
  construction sites now pass the cursor, so the reported position is the
  byte the decoder or encoder actually stopped at.
  
  Stacked on #1266 and merges after it.
  
  ## What "no position" means here
  
  #1266 makes `offset` an `Option<usize>` where `None` means the error has
  no position in the input stream at all, rather than a position that
  happened to be unavailable. This PR is the other half of that: it walks
  the workspace and gives a real position to every site that has one, so
  the sites left reporting `None` are the ones that genuinely never had
  one.
  
  Those are constructors validating their arguments, integer conversions,
  cache lookups that missed, accessors on already-decoded structures, and
  the declared-size checks described below. They report nothing rather
  than byte zero, and that is now their permanent answer rather than a gap
  awaiting another sweep.
  
  There are no `at: 0` sites left anywhere in the workspace.
  
  ## The rule
  
  The position is attached where the cursor identifies the bytes being
  complained about. It is omitted where the complaint is about a size the
  peer declared, computed from data already consumed, because there the
  cursor points at a byte that is not the problem.

- Expand OLE clipboard snapshots ([#1794](https://github.com/Devolutions/IronRDP/issues/1794)) ([c7766c5a56](https://github.com/Devolutions/IronRDP/commit/c7766c5a56ba668a93737f2ca384fc2b63546376)) 

  Expose bounded read-only snapshots for text, locale, DIB/DIBV5, and
  Windows HTML formats. Validate FORMATETC and payloads, return
  independent HGLOBAL media, and keep file, write, and advisory paths
  disabled.

- Add clipboard image support ([#1877](https://github.com/Devolutions/IronRDP/issues/1877)) ([a59b3b687d](https://github.com/Devolutions/IronRDP/commit/a59b3b687d8ec81cfb79bd6e91e7d3341c866b6b)) 

  Adds PNG clipboard image support to the agent daemon and CLI using CLIPRDR DIB conversions.



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


# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpel-v0.0.0)] - 2026-09-01

### <!-- 1 -->Features

- Add location redirection ([#1778](https://github.com/Devolutions/IronRDP/issues/1778)) ([1cee7a8613](https://github.com/Devolutions/IronRDP/commit/1cee7a86135a0556c01965d0406233bd7df367a9)) 

  Implement MS-RDPEL v1 codecs and the location DVC state machine, then
  route the ActiveX methods through the bounded client input queue.
  
  Preserve mstsc-compatible validation and altitude caching while
  surfacing inactive sessions, channel readiness, queue pressure, and
  encoding failures. Coordinates are caller-supplied only and are never
  logged or persisted.



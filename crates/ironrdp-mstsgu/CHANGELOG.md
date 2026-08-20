# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- HTTP multi-leg authentication for the WebSocket upgrade: Negotiate SPNEGO (Kerberos with NTLM fallback via KDC discovery / `SSPI_KDC_URL`), pure NTLM when only NTLM is advertised, and Basic fallback.

## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

Initial release.

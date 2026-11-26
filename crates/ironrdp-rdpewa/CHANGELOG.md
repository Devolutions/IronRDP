# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpewa-v0.2.0...ironrdp-rdpewa-v0.3.0)] - 2026-11-26

### <!-- 1 -->Features

- Add MS-RDPEWA WebAuthn RPC processor ([#2444](https://github.com/Devolutions/IronRDP/issues/2444)) ([e6b323657e](https://github.com/Devolutions/IronRDP/commit/e6b323657ef8da22cd74ba051e51da54cd436e82)) 

  Add `RdpewaServerRpc` and `RdpewaWebAuthnRequest` to drive
  server-initiated `[MS-RDPEWA]` (`WebAuthN_Channel`) `IUVPAA`,
  `WEB_AUTHN` (`MakeCredential` and `GetAssertion`), and `CANCEL_CUR_OP`
  RPCs, bounding inbound DVC messages to 256 KiB and aborting unanswered
  ceremonies with `E_ABORT` on channel close, drop, or declined channel
  creation via `DvcServerProcessor::creation_failed`.
  
  Order `CborKey` according to CTAP2 section 6 and RFC 8949 section 4.2.3
  canonical CBOR (by CBOR major type first, then encoded byte length, then
  lexicographically by encoded bytes), align `WebAuthnPara` and
  `DeviceInfo` with `[MS-RDPEWA]` wire field names (`requireResident`,
  `preferResident`, `attestationPreference`, and lowercase
  `manufacturer`/`product` aliases) and user-verification flag constants
  (`CTAPCLT_UV_*`), and read top-level `request` bytes for `CancelCurOp`
  in `RdpewaClient`.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpewa-v0.1.0...ironrdp-rdpewa-v0.2.0)] - 2026-10-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-dvc` public dependency to 0.10

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.11

- [**breaking**] Update `ironrdp-svc` public dependency to 0.10



## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rdpewa-v0.1.0)] - 2026-10-23

### Added

- Initial MS-RDPEWA protocol crate with CBOR PDU codec, client processor, and server skeleton.

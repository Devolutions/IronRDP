# IronRDP RDPDR native backends

Native backend building blocks for the IronRDP RDPDR static channel.

- On macOS and Linux, the crate exports the existing `nix::backend` filesystem
  backend.
- On Windows, the crate contains the native, handle-relative filesystem
  foundation used for drive redirection. It validates every protocol path
  before resolving it below an opened volume root, and it rejects DOS device
  aliases and reparse-point traversal. Its static filesystem support includes
  create/open with initial allocation and validated delete-on-close, close,
  flush, bounded synchronous offset I/O, basic file-information queries, actual
  selected-volume information, basic metadata, EOF/allocation, delete-on-close
  and confined rename changes, handle-bound security descriptor queries and
  validated descriptor updates, alternate-data-stream information, plus
  one-entry-at-a-time directory enumeration. Waiting byte-range locks and
  directory-change notifications complete from bounded, cancellable worker
  operations; close, reset, and device removal cancel outstanding work. Device
  Control is deny-by-default: supported filesystem controls are the
  handle-bound `FSCTL_CREATE_OR_GET_OBJECT_ID` with an empty input and fixed
  64-byte output, plus the read-only `FSCTL_GET_COMPRESSION`,
  `FSCTL_GET_INTEGRITY_INFORMATION`, and `FSCTL_QUERY_ALLOCATED_RANGES`, all
  with validated, bounded buffers.
  - On Windows, optional smartcard redirection is pluggable via
    `WindowsRdpdrBackendFactory::with_smartcard(true)`. Smartcard-only products
    (no drives) are valid. The current backend is a compile-ready stub that
    completes decoded MS-RDPESC calls with `SCARD_E_UNSUPPORTED_FEATURE`; a full
    WinSCard implementation is a follow-up.

The Windows implementation is intentionally layered so protocol code remains
platform independent in `ironrdp-rdpdr`.

`WindowsRdpdrBackendFactory` configures zero or more `RedirectedDrive`s and an
optional smartcard flag. Multi-drive registry management remains outside this
native backend.

# IronRDP RDPDR native backends

Native backend building blocks for the IronRDP RDPDR static channel.

- On macOS and Linux, the crate exports the existing `nix::backend` filesystem
  backend.
- On Windows, the crate contains the native, handle-relative filesystem foundation used for drive redirection.
  It validates every protocol path before resolving it below an opened volume root, and it rejects DOS device aliases and reparse-point traversal.
  Its static filesystem support includes create/open, close, flush, bounded offset I/O, file and volume information, metadata changes, security descriptors, alternate data streams, directory enumeration, locks, notifications, and deny-by-default device controls.
  Smartcard-only products are valid when the channel is configured with `ironrdp_rdpdr::Rdpdr::with_smartcard(0)`.
  The WinSCard path implements core and extended MS-RDPESC calls, preserves ANSI wire variants, and follows Windows buffer-probe rules.
  `WindowsRdpdrBackendFactory::with_default_printer(true)` discovers the current user's default queue, announces its local driver, and spools bounded `RAW` jobs on a worker thread.
  Default-printer redirection requires a matching remote driver and does not implement Easy Print/XPS, multiple printers, cache PDUs, or hotplug.

The Windows implementation is intentionally layered so protocol code remains
platform independent in `ironrdp-rdpdr`.

`WindowsRdpdrBackendFactory` configures zero or more `RedirectedDrive`s and optional default-printer discovery.
Multi-drive registry management remains outside this native backend.

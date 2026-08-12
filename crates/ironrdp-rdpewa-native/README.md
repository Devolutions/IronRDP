# ironrdp-rdpewa-native

Windows backend for the IronRDP [MS-RDPEWA] client.

## Paths

1. **webauthn.dll oneshot (preferred for ceremonies)**
   Forwards the original RDPEWA CBOR through `webauthn.dll`'s IWTS remote-RPC path
   (same private stack MSTSC uses). Required for **hash-only** hosts that omit
   `clientDataJSON` — public `WebAuthN*` APIs cannot complete those ceremonies.

2. **Public WebAuthN* APIs (fallback)**
   Used when oneshot is unavailable and the host supplied full `clientDataJSON`.

Session channel registration may still prefer hosting `webauthn.dll` as a DVC COM
listener (`ironrdp-dvc-com-plugin`) for full MSTSC parity. Set
`IRONRDP_WEBAUTHN_FORCE_NATIVE=1` to exercise this backend via `RdpewaClientListener`
instead.

[MS-RDPEWA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpewa/

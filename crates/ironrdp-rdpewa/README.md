# ironrdp-rdpewa

MS-RDPEWA (WebAuthn Virtual Channel) protocol types and DVC processors for IronRDP.

## Channel

- DVC name: `WebAuthN_Channel`
- Spec: [MS-RDPEWA](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpewa/)

## Layout

- `pdu` — CBOR request/response codec and MVP command types
- `client` — `RdpewaClient`, recreatable `RdpewaClientListener`, and `RdpewaClientHandler`
- `server` — minimal `RdpewaServer` skeleton for tests

Platform backends (Windows WebAuthn APIs) live in `ironrdp-rdpewa-native`.

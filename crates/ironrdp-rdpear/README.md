# ironrdp-rdpear

Opt-in client framing, inner-packet codecs, and portable package-call NDR codecs for the [MS-RDPEAR] authentication-redirection dynamic virtual channel used by Remote Credential Guard.
The crate validates the outer envelope and dispatches its encrypted payload to a caller-supplied `AuthRedirectionBackend`.
It uses `no_std` with `alloc` when default features are disabled and performs no authentication or I/O itself.
Freestanding-target support remains limited by the existing DRDYNVC/SVC dependency chain.

The outer `Length` field counts encrypted payload bytes, excluding the 24-byte header.
This matches all 328 independently reassembled, TLS-authenticated Windows envelopes in `wireshark-rdp`'s `rdp-credential-guard-accepted1.pcapng`, but differs from MS-RDPEAR revision 10.0 (2024-04-23) section 2.2's "overall length" wording.
The decoder rejects total-length framing rather than guessing between formats, which would accept a native envelope truncated by exactly 24 bytes.
This verifies outer framing only; the capture's inner Kerberos protection has not been decrypted.

Register `AuthRedirectionClient::new(backend, max_message_size)` using `DrdynvcClient::with_dynamic_channel` only after establishing an authenticated, secure session and obtaining its CredSSP security context.
The size limit includes the 24-byte outer header and is enforced during inbound DVC reassembly, before fragments are retained.
Responses are checked against the same limit before they are returned for transmission.
Also configure `DrdynvcClient::with_max_transport_message_size` before constructing the static channel, because SVC carrier reassembly precedes the per-DVC limit.
The carrier bound applies to all dynamic channels on that transport and must accommodate DRDYNVC control messages and fragment headers.
The client sends no unsolicited messages, rejects malformed frames before calling the backend, and disables processing after any active-channel request failure.
Backend errors propagate to the session caller, which must terminate the security exchange rather than retry a potentially advanced cryptographic sequence.
Closing the channel notifies the backend to release its context.
Each client is single-use; a new security context requires a new client.

The backend must authenticate and decrypt each payload with the negotiated CredSSP security context, validate the DER `TSRemoteGuardInnerPacket` and security-package buffer header, dispatch only authorized package operations, and encrypt the response.
It must enforce replay/sequence checks, identity binding, package version negotiation, and limits on decrypted data and native allocations.
Neither encrypted payloads nor credential material should be logged.

## Decrypted inner packets

`RemoteGuardInnerPacket::decode_with_limit` parses one complete DER packet without copying its package name or call buffer.
It checks the plaintext size before parsing, requires the default version zero to be omitted as DER specifies, and validates the 16-byte package-call header.
Encoding adds that header and emits canonical DER lengths.
Currently unused extensions are rejected explicitly rather than silently ignored.
Package names and call bodies remain opaque at the DER layer; decoding does not authorize a package.
The backend must authenticate and bound decryption allocations before using this codec.

## Portable package-call NDR

`ndr::{KerberosRequest, KerberosResponse, NtlmRequest, NtlmResponse}` cover the request and response arms of all 18 Kerberos and five NTLM calls from appendices 6.1–6.3.
They preserve unique-pointer nullability, counted buffers, UTF-16 code units, signed scalars, 32-bit BOOL values, and opaque wrapped protocol values without executing credential operations.
`Debug` omits credential material.
Both codec surfaces take independent wire-size, allocation, and cumulative-element limits.
Decoding checks conformant counts against correlations and remaining bytes before reserving memory.

Use `NdrProfile::WindowsNative` explicitly for the inspected Windows layouts; published defaults do not match their roots.
Existing methods and `Encode`/`Decode` retain `NdrProfile::Published`; `encode_with_profile` and `decode_with_profile` accept `NdrProfile::WindowsNative`.
The native profile follows the inspected Windows 11 26100 layouts, not a negotiated or automatically detected dialect.

| Layout | Published profile | Windows-native profile |
| --- | --- | --- |
| Root | Structure by value | Non-null unique pointer, with Kerberos eight-byte or NTLM four-byte referent alignment |
| Key `reserved1` | NULL unique-pointer slot; non-null referents rejected | Four-byte ignored slot, zeroed on encoding and ignored on decoding; no referent |
| NTLM operations after negotiation | IDs `0x201`–`0x204` | IDs `0x202`–`0x205`; private `ProtectCredential` at `0x201` is rejected |

`call_id()` retains the logical published ID; `wire_call_id(profile)` returns the selected wire discriminant.
The native profile also rejects reserved/private Kerberos operations.
Neither profile executes credential operations or copies host pointers into messages.

Use serialization v1 for RDPEAR package calls, as required by MS-RDPEAR section 2.2.2:

- `encode_with_limits` and `decode_with_limits` retain the complete version-one serialization object defined by [MS-RPCE] section 2.2.6, with both integer byte orders.
  They include common/private headers and eight-byte end padding; decoding requires complete consumption and ignores reserved filler/padding contents.
  `Encode` and `Decode` remain v1 codecs with default limits; `Encode` produces little-endian data.
- `encode_body_with_limits` and `decode_body_with_limits` expose raw NDR32 for lower-level integration and tests, not an alternative RDPEAR package-call wire format.
  They use caller-selected byte order and body-relative alignment, omit serialization headers and end padding, and require exact body consumption.

The corresponding `encode_body_with_profile` and `decode_body_with_profile` methods select the raw body's layout explicitly.

`RemoteGuardInnerPacket.package_call` excludes MS-RDPEAR's 16-byte security-package header.
Supply the complete v1 object for RDPEAR; DER encoding adds only the security-package header, and DER decoding returns the supplied call bytes unchanged.
No codec detects envelopes or falls back between them.
The specification defines this package framing, but it remains unverified against native Windows plaintext because the captures' inner Kerberos protection has not been decrypted.
NDR64, version-two serialization, concatenated objects, and authentication-package execution are not supported.

The published `void* reserved1` has no referent type and the literal IDL fails SDK MIDL compilation with MIDL2139.
Native metadata instead marks it `[ignore]`; an independently compiled fixture confirms that no pointer value or referent is transmitted, although a four-byte ignored wire slot remains.
`ReservedPointer` is a marker, not a native address or credential handle.
`reserved2` and `reserved3` remain opaque protocol data; the codec does not interpret or operate on their key contents.

Tests exercise both envelopes for every arm and byte order, malformed inputs, caller limits, and independent SDK-generated vectors for enum16 unions, nested deferral, pointer-to-pointer buffers, conformant credential arrays, Unicode capacities, and NTLM fixed-key widths.
Raw fixture assertions strip the two eight-byte RPC headers and serialization-only end padding, not the security-package header.
The reproducible public-IDL fixture and the compiler-error reproducer are in `tests/ndr_oracle`.
The published fixture's replacement of `void*` with `byte*` proves only its NULL pointer layout; separate native-profile fixtures use `[ignore]`.
The fuzz crate includes bounded decode and value-preserving round-trip checks for all four package surfaces.

## Remaining Remote Credential Guard work

This crate alone is not end-to-end Remote Credential Guard support.
The connector's opt-in [`remote-guard` integration][rcg-integration] supplies a backend using `sspi::rcg` and wires its live context through the blocking and async drivers.
It supports bounded Kerberos and NTLMv2 operation subsets; see that integration's live-logon evidence and remaining native-operation limits.
It remains experimental and does not provide a native Windows credential backend.
There is no plaintext fallback or default client registration.
Outbound credential-operation interoperability and native failure-path tests remain necessary.
Initial RCG authentication must use Kerberos without NTLM fallback.
That requirement is distinct from the NTLM package operations defined by MS-RDPEAR.

The framing follows MS-RDPEAR sections 2.1 (channel name) and 2.2 (outer header and encrypted payload).
Package operations and negotiation are defined in sections 2.2.1, 2.2.2, and 3.1.5.

[MS-RDPEAR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpear/
[MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/
[rcg-integration]: https://docs.rs/ironrdp-connector#experimental-remote-credential-guard

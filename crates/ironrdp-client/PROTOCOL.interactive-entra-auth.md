# Interactive Entra authentication channel

No Microsoft Open Specification documents `Microsoft::Windows::RDS::InteractiveEntraAuth`.
This note describes the exchange `src/interactive_entra.rs` implements, following the Windows App 2.0.1485.0 client, so the `oracle: reference` markers in that module's tests have one place to point to.

## Acceptance

Windows App opens the channel only when interactive Entra authentication is enabled for it, which it is not by default.
This client leaves the channel off unless the embedder enables it.

## Request

The first message on the channel is plain JSON:

- `version` must be 1; any other value fails with `E_NOTIMPL`.
- `scenario_id` is a GUID string, required, and must be `75cc6d06-3a48-43a1-bcd0-cb46eb8f9aae` or `b419e9ce-25cd-44bc-8da5-6ff75edd1e28`. Any other GUID fails with `E_NOTIMPL`. The client does not otherwise use the ID.
- `username_hint` is optional.

A request that is not a JSON object or lacks `scenario_id` fails; this client reports both as `E_INVALIDARG`.

## TLS

After the request, the client runs a TLS handshake inside the channel as the TLS client.
Every later message on the channel goes through that TLS layer.

## Server certificate

The common name (OID `2.5.4.3`) of the server's TLS certificate is the session host's Entra device ID.

Windows App requires the certificate to chain to one of the tenant's Entra P2P root certificates, and fails with `E_ACCESSDENIED` when it does not or when the device ID or the roots are missing.
This client checks once the token is granted, because the embedder supplies the roots with the token.

## Token and RDS AAD exchange

The client asks for an Entra token for the session host's device, with `username_hint`, when present, as the user name, bound to the session's proof-of-possession key.
The exchange inside TLS is then the RDS AAD Auth exchange of [MS-RDPBCGR] section 5.4.5.4.1: the server sends `ts_nonce`, the client sends `rdp_assertion`, and the server answers with `authentication_result`.

## Errors

On any failure, the client sends `{"error_code": <HRESULT>}` and closes the channel.
The message goes through TLS once the handshake has completed and is plain before it.
Whether the HRESULT is written signed was not determined; this client writes it as a signed 32-bit integer.

## Not verified

- No exchange has been run against a live session host.
- Where Windows App obtains the tenant's P2P root certificates was not determined.
- What the two scenario IDs mean is not documented.

[MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

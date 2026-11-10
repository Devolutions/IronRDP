# IronRDP RDCleanPath

RDCleanPath PDU structure used by IronRDP and Devolutions Gateway.

VERSION_1 has two request shapes without changing its DER fields:

| Shape | `preconnection_blob` | X.224 | Proxy front |
| --- | --- | --- | --- |
| Ordinary | Optional legacy complete PCB | Present | optional PCB → X.224 → TLS |
| VMConnect | PCB V2 Unicode payload | Absent | encode PCB → TLS |

The typed [`RDCleanPathMessage`](crate::RDCleanPathMessage) model exposes these as separate variants so callers do
not infer VMConnect ordering from the presence of a generic PCB. A VMConnect response omits X.224;
the client then performs CredSSP followed by X.224.

## Error codes

An error PDU carries an `error_code`, and optionally an HTTP status code, a WSA error, or a TLS alert.

| Codes | Meaning |
| --- | --- |
| 0 | Reserved |
| 1 | General error |
| 2 | Negotiation error: the X.224 field holds the server's negotiation response |
| 3 to 99 | Reserved |
| 100 to 16383 | Standard reasons |
| 16384 to 32767 | Implementation-specific reasons, defined by each proxy |
| 32768 to 65535 | Reserved, so that implementations storing the code as a signed 16-bit integer can represent every code in use |

IronRDP assigns every code outside the implementation-specific range.

### Standard reasons

A standard reason describes a condition any client can explain to its user without knowing which proxy it talks to.
The credential is the `proxy_auth` value the client sent.

| Code | Constant | Meaning |
| --- | --- | --- |
| 100 | `EXPIRED_ERROR_CODE` | The credential has expired |
| 101 | `NOT_YET_VALID_ERROR_CODE` | The credential is not valid yet |
| 102 | `REVOKED_ERROR_CODE` | The credential has been revoked |
| 103 | `TARGET_UNREACHABLE_ERROR_CODE` | The proxy could not connect to the target |
| 104 | `DNS_RESOLUTION_FAILED_ERROR_CODE` | The proxy could not resolve the target host name |
| 105 | `TLS_HANDSHAKE_FAILED_ERROR_CODE` | The TLS handshake between the proxy and the target failed |

The web client's `@devolutions/iron-remote-desktop-rdp` package exports the same codes as `RDCleanPathErrorCode`.

To propose a new standard reason, open a pull request against IronRDP.
It must make sense for any proxy, regardless of its credential format or network topology,
and carry information the HTTP status, WSA error and TLS alert fields do not.
New reasons take the next free code; never renumber or reuse a code.

### Implementation-specific reasons

A proxy assigns codes from 16384 to 32767 to reasons that only make sense for that proxy,
such as checks specific to its credential format or the components it routes through.
These codes have no registry: two proxies may give the same code different meanings.
A client interprets them only when it knows which proxy it talks to, and otherwise treats them as general errors.
Prefer a standard reason when one fits.

### Compatibility

A receiver treats any code it does not know as a general error.
Clients that predate a code still decode the PDU, and see it as an unknown error.
To keep those clients informative, a proxy sending a reason code still fills in the detail fields that describe the failure:
the HTTP status code for authorization failures (for example, 401 with `expired`),
and the WSA error or TLS alert for connection failures.

Do not add fields to the PDU or to the error structure to carry new information.
IronRDP-based peers decode them with der's derived decoder, which rejects an unknown field that follows the last field it knows, despite the context-specific tags.
der 0.8 also rejects an unknown field between known ones.
A new error field would follow the last one, so a peer that predates it fails to decode any error PDU carrying it.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

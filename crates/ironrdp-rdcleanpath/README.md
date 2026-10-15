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
| 100 to 16383 | Standard reasons, assigned by IronRDP |
| 16384 to 32767 | Implementation-specific reasons, defined by each proxy |
| 32768 to 65535 | Reserved, so that implementations storing the code as a signed 16-bit integer can represent every code in use |

### Standard reasons

A standard reason describes a condition any client can explain to its user without knowing which proxy it talks to.
Each one has a stable snake_case name:
[`RDCleanPathErr::reason`](crate::RDCleanPathErr::reason) returns the name for a code, and
[`RDCleanPathErr::error_code_for_reason`](crate::RDCleanPathErr::error_code_for_reason) returns the code for a name.
The credential is the `proxy_auth` value the client sent.

| Code | Reason | Meaning |
| --- | --- | --- |
| 100 | `expired` | The credential has expired |
| 101 | `not_yet_valid` | The credential is not valid yet |
| 102 | `revoked` | The credential has been revoked |
| 103 | `target_unreachable` | The proxy could not connect to the target |
| 104 | `dns_resolution_failed` | The proxy could not resolve the target host name |
| 105 | `tls_handshake_failed` | The TLS handshake between the proxy and the target failed |

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
Deployed decoders reject unknown trailing fields, so a peer that predates a new field fails to decode any PDU carrying it.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

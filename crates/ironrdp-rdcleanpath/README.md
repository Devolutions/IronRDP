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

| Code | Meaning |
| --- | --- |
| 1 | General error |
| 2 | Negotiation error: the X.224 field holds the server's negotiation response |

Codes from 100 tell why the proxy rejected the connection.
Each one maps to a stable snake_case reason name, shared with Devolutions Gateway.
[`RDCleanPathErr::reason`](crate::RDCleanPathErr::reason) returns the name for a code, and
[`RDCleanPathErr::error_code_for_reason`](crate::RDCleanPathErr::error_code_for_reason) returns the code for a name.

| Code | Reason | Meaning |
| --- | --- | --- |
| 100 | `bad_signature` | The token signature does not verify |
| 101 | `expired` | The token has expired |
| 102 | `not_yet_valid` | The token is not valid yet |
| 103 | `gateway_id_mismatch` | The token was issued for a different gateway |
| 104 | `revoked` | The token has been revoked |
| 105 | `unexpected_replay` | The token was reused when reuse is not allowed |
| 106 | `wrong_token_type` | The token type does not match the request |
| 107 | `target_unreachable` | The proxy could not connect to the target |
| 108 | `dns_resolution_failed` | The proxy could not resolve the target host name |
| 109 | `tls_handshake_failed` | The TLS handshake with the target failed |
| 110 | `agent_unavailable` | The agent that routes to the target is unavailable |

Clients that predate a code still decode the PDU and display it as an unknown error.
To keep those clients informative, a proxy sending a reason code still fills in the detail fields it sent before:
the HTTP status code for token failures (for example, 401 with `expired`), and the WSA error or TLS alert for connection failures.

A receiver treats any code it does not know as a general error.
New reasons take the next free code; never renumber or reuse a code.
Codes 3 to 99 are reserved.

Do not add fields to the PDU or to the error structure to carry new information.
Deployed decoders reject unknown trailing fields, so a peer that predates a new field fails to decode any PDU carrying it.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

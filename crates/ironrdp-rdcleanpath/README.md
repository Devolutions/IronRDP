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

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

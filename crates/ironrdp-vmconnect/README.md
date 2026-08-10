# IronRDP VMConnect

Hyper-V console front-end: **PCB → TLS → CredSSP → X.224**.

Enhanced mode is the default. Its PCB payload is `{vm_id};EnhancedMode=1`; basic mode sends
`{vm_id}` unchanged.
After pre-X.224 CredSSP, the X.224 request advertises only `HYBRID`. `HYBRID_EX` is intentionally
excluded because VMConnect does not need the Early User Authorization Result extension.

Callers must finish the PCB write within [`PCB_TRANSMIT_DEADLINE`](crate::PCB_TRANSMIT_DEADLINE)
(10s, MS-RDPEPS) after TCP connect. This crate does not own a runtime timer.

[`connect_front`](crate::connect_front) always runs CredSSP and **rejects** a connector with
`enable_tls` or `enable_credssp` cleared, so every embedder shares one security choke point.
Transport choice (direct vs gateway) stays with the application.

The configured credentials authenticate the Hyper-V host during this pre-X.224 CredSSP exchange.
After authentication succeeds, the connector omits them from the normal RDP tail, including the
X.224 username cookie, licensing identity, and guest-facing Client Info PDU. The tested Enhanced
Session endpoint did not honor a separate Client Info password for guest autologon; guest sign-in
remains interactive.

| API | Role |
| --- | --- |
| `PORT` | 2179 |
| `PCB_TRANSMIT_DEADLINE` | 10s bound for PCB after TCP connect |
| `Mode` | Enhanced or basic console routing |
| `preconnection_blob_payload` | Unicode payload for RDCleanPath VMConnect requests |
| `encode_preconnection_blob` | PCB V2 bytes |
| `send_preconnection_blob` | write PCB → `PcbSent` |
| `pcb_sent_via_proxy` | receipt when an RDCleanPath proxy wrote the PCB |
| `prepare_connector` | shared TLS + CredSSP prerequisite check |
| `ensure_selected_credssp` | require HYBRID after post-CredSSP X.224 |
| `connect_front` | CredSSP + X.224 after TLS; takes `PcbSent` → `Upgraded` |

```rust,ignore
let pcb_sent =
    ironrdp_vmconnect::send_preconnection_blob(&mut framed, vm_id, ironrdp_vmconnect::Mode::Enhanced).await?;
// caller: TLS
let upgraded = ironrdp_vmconnect::connect_front(
    pcb_sent, &mut framed, &mut connector, &mut network, server_name, &pubkey, kerberos,
).await?;
let result = ironrdp_async::connect_finalize(upgraded, connector, /* ... */).await?;
```

For RDCleanPath, send [`preconnection_blob_payload`](crate::preconnection_blob_payload) in an
explicit VMConnect request. The proxy encodes and writes the PCB before TLS, then the client passes
[`pcb_sent_via_proxy`](crate::pcb_sent_via_proxy) to `connect_front`.

[IronRDP](https://github.com/Devolutions/IronRDP)
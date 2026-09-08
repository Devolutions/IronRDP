# IronRDP VMConnect

Hyper-V console front-end: **PCB → TLS → CredSSP → X.224**.

Enhanced mode is the default. Its PCB payload is `{vm_id};EnhancedMode=1`; basic mode sends
`{vm_id}` unchanged.
After pre-X.224 CredSSP, the X.224 request advertises only `HYBRID`. `HYBRID_EX` is intentionally
excluded because VMConnect does not need the Early User Authorization Result extension.

Callers must finish the PCB write within [`PCB_TRANSMIT_DEADLINE`](crate::PCB_TRANSMIT_DEADLINE)
(10s, MS-RDPEPS) after TCP connect. This crate does not own a runtime timer.

[`connect_front`](crate::connect_front) runs the native and Web CredSSP + X.224 front and rejects a
connector with `enable_tls` or `enable_credssp` cleared. FFI embedders that drive these steps
themselves reuse [`prepare_connector`](crate::prepare_connector) and
[`ensure_selected_credssp`](crate::ensure_selected_credssp), so the prerequisites and selected
protocol checks remain shared. Transport choice (direct vs gateway) stays with the application.

The configured credentials authenticate the Hyper-V host during this pre-X.224 CredSSP exchange.
After authentication succeeds, the connector omits them from the normal RDP tail, including the
X.224 username cookie, licensing identity, and guest-facing Client Info PDU. The tested Enhanced
Session endpoint did not honor a separate Client Info password for guest autologon; guest sign-in
remains interactive.

On Windows, `connect_front_with_current_user` uses native SSPI to authenticate the Hyper-V host with the caller's logon token.
This path does not require or expose a reusable host password.
It requires CredSSP version 5 or later for nonce-bound public-key verification.

## Local frame-buffer redirection

`FrameBufferClient` implements the private dynamic channel for local Windows VMConnect sessions.
The Hyper-V host offers `Microsoft::Windows::RDS::Frame_Buffer::Control::v08.01` only when it recognizes a same-machine connection.
The channel exchanges the caller's logon SID, opens three host-created `Global\Microsoft::Windows::RDS::FBR-*` objects, and reads the 32-bpp top-down DIB from shared memory.
The mapping begins with a dirty `RECT` and `BITMAPINFOHEADER`; pixel data starts at the next Windows allocation-granularity boundary.
The server serializes updates with a mutex and signals an auto-reset event after writing a dirty region.

`local_instance_id` reads the Terminal Server instance ID used by the Hyper-V host's same-machine check.
Applications supply it as the RDP digital product ID when establishing a local VMConnect session.

| API | Role |
| --- | --- |
| `PORT` | 2179 |
| `PCB_TRANSMIT_DEADLINE` | 10s bound for PCB after TCP connect |
| `Mode` | Enhanced or basic console routing |
| `FrameBufferClient` | Windows-only FBR DVC and shared-memory reader |
| `local_instance_id` | Same-machine Terminal Server identity |
| `preconnection_blob_payload` | Unicode payload for RDCleanPath VMConnect requests |
| `encode_preconnection_blob` | PCB V2 bytes |
| `send_preconnection_blob` | write PCB → `PcbSent` |
| `pcb_sent_via_proxy` | receipt when an RDCleanPath proxy wrote the PCB |
| `prepare_connector` | shared TLS + CredSSP prerequisite check |
| `ensure_selected_credssp` | require HYBRID after post-CredSSP X.224 |
| `connect_front` | CredSSP + X.224 after TLS; takes `PcbSent` → `Upgraded` |
| `connect_front_with_current_user` | Native Windows CredSSP + X.224 using the caller's token |

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
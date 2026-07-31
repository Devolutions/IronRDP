# IronRDP VMConnect

Hyper-V console front-end: **PCB → TLS → CredSSP → X.224**.

PCB payload is always `{vm_id};EnhancedMode=1`.
After pre-X.224 CredSSP, the host may select either `HYBRID` or `HYBRID_EX`.

| API | Role |
| --- | --- |
| `PORT` | 2179 |
| `encode_preconnection_blob` | PCB V2 bytes |
| `send_preconnection_blob` | write PCB → `PcbSent` |
| `connect_front` | CredSSP + X.224 after TLS; takes `PcbSent` → `Upgraded` |

```rust,ignore
let pcb_sent = ironrdp_vmconnect::send_preconnection_blob(&mut framed, vm_id).await?;
// caller: TLS
let upgraded = ironrdp_vmconnect::connect_front(
    pcb_sent, &mut framed, &mut connector, &mut network, server_name, &pubkey, kerberos,
).await?;
let result = ironrdp_async::connect_finalize(upgraded, connector, /* ... */).await?;
```

[IronRDP](https://github.com/Devolutions/IronRDP)
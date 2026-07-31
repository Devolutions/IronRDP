# IronRDP VMConnect

Hyper-V console front-end: **PCB → TLS → CredSSP → X.224**.

| API | Role |
| --- | --- |
| `PORT` | 2179 |
| `send_preconnection_blob` | pre-TLS routing blob → `PcbSent` receipt |
| `connect_front` | post-TLS CredSSP + X.224; **requires** `PcbSent` by value → `Upgraded` |
| `PcbSent` | opaque receipt; only produced by `send_preconnection_blob` |

Caller owns TLS between the two steps. Hand `Upgraded` to `ironrdp_async::connect_finalize`.

```rust,ignore
let pcb_sent = ironrdp_vmconnect::send_preconnection_blob(&mut framed, vm_id).await?;
// caller: TLS upgrade on the same stream
let upgraded = ironrdp_vmconnect::connect_front(
    pcb_sent, &mut framed, &mut connector, &mut network, server_name, &pubkey, kerberos,
).await?;
let result = ironrdp_async::connect_finalize(upgraded, connector, /* ... */).await?;
```

[IronRDP](https://github.com/Devolutions/IronRDP)

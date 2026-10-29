# IronRDP Server

Extendable skeleton for implementing custom RDP servers.

For now, it requires the [Tokio runtime](https://tokio.rs/).

---

The server currently supports:

**Security**
 - Enhanced RDP Security with TLS External Security Protocols (TLS 1.2 and TLS 1.3)

**Input**
 - FastPath input events
 - x224 input events and disconnect

**Codecs**
 - bitmap display updates with RDP 6.0 compression

---

Custom logic for your RDP server can be added by implementing these traits:
 - `RdpServerInputHandler` - callbacks used when the server receives input events from a client
 - `RdpServerDisplay`      - notifies the server of display updates

This crate is part of the [IronRDP] project.

## Echo RTT probes (feature `echo`)

Enable the `echo` feature to use the ECHO dynamic virtual channel (`MS-RDPEECO`) and measure round-trip time.

```rust
use ironrdp_server::{RdpServer, ServerError, ServerErrorExt as _, ServerResult};

# async fn demo(mut server: RdpServer) -> ServerResult<()> {
// Grab and clone the shared handle before moving the server into a task.
let echo = server.echo_handle().clone();

let local = tokio::task::LocalSet::new();
local
	.run_until(async move {
		let server_task = tokio::task::spawn_local(async move { server.run().await });

		{
			echo.send_request(b"ping".to_vec())?;

			for measurement in echo.take_measurements() {
				println!(
					"echo payload size={} rtt={:?}",
					measurement.payload.len(),
					measurement.round_trip_time
				);
			}
		}

		server_task
			.await
			.map_err(|e| ServerError::custom("server task", e))??;
		Ok::<(), ServerError>(())
	})
	.await?;
# Ok(()) }
```

`send_request` queues a probe via the server event loop. If no client has opened the ECHO channel yet, the request is dropped.

## Static-channel writes

Register a per-connection processor through `StaticChannelFactory`, then send `ServerEvent::StaticChannelJoined` through `RdpServer::event_sender()` to check whether the active client joined its channel.
Send a complete, unframed channel PDU with `ServerEvent::StaticChannelWithCompletion`; the server adds headers and sends its chunks in order.
Fragmented writes set `SHOW_PROTOCOL` on every chunk, as required by MS-RDPBCGR section 3.1.5.2.1, "Sending of Virtual Channel PDU"; unfragmented writes do not set it.
This API does not expose a flag override and does not apply the shared SVC encoder's Windows RDPDR compatibility exception, so do not use it for an endpoint that requires `SHOW_PROTOCOL` to be omitted.
The completion receiver resolves only after the entire transport write succeeds, not when the peer processes the PDU.
A closed receiver means failure or discard, but does not prove that no bytes were sent, so it is not permission to retry.

The event queue is unbounded.
Acquire the event's owned permit from a shared, bounded Tokio semaphore before allocating the payload, cap each payload's size, and await completion before submitting another PDU.
The event retains its permit until written or discarded, even when the producer drops its completion receiver.
These limits belong to the producer; the server does not impose a byte budget.
The entire PDU holds the shared writer lock, so a large payload or slow transport stalls all other session output, including display and heartbeat traffic, until its write finishes or fails.

Dropping the completion receiver or making `is_current` return `false` cancels a write that has not started.
Keep `is_current` nonblocking; it is checked again after acquiring the shared writer lock.
Once transmission starts, producer cancellation does not interrupt it or release its permit early.
Unjoined channels and events left over from a previous connection are discarded.

[IronRDP]: https://github.com/Devolutions/IronRDP

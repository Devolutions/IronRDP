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

## Runtime control handle

Use `RdpServer::handle()` to control a running server from other tasks without using raw events.

```rust
use ironrdp_server::RdpServer;

# async fn demo(server: RdpServer) -> anyhow::Result<()> {
let handle = server.handle().clone();

// Request listener address once run loop starts.
let _bound_addr = handle.local_addr().await?;

// Update credentials for subsequent connections.
handle.set_credentials(ironrdp_server::Credentials {
	username: "alice".to_owned(),
	password: "secret".to_owned(),
	domain: Some("example".to_owned()),
})?;

// Stop the server loop.
handle.quit("shutdown requested")?;
# Ok(()) }
```

---

Custom logic for your RDP server can be added by implementing these traits:
 - `RdpServerInputHandler` - callbacks used when the server receives input events from a client
 - `RdpServerDisplay`      - notifies the server of display updates

## Runtime control

Clone `RdpServer::handle()` before running the server to control it from another task.
Use it for quit requests, credential updates, and asynchronous listener-address queries.
See [`RdpServerHandle`] for request timing, lifecycle behavior, and error semantics.

[`RdpServerHandle`]: https://docs.rs/ironrdp-server/latest/ironrdp_server/struct.RdpServerHandle.html

This crate is part of the [IronRDP] project.

## RemoteApp window orders

Opt in with `RdpServerBuilder::with_remote_app(Some(capabilities))` and attach a `rail` static-channel processor with `with_static_channel_factory`.
The server offers Remote Programs and Window List capabilities only to clients requesting RAIL.
`ConnectionHandler::on_remote_app_capabilities` reports the effective client capabilities after each activation, including reactivation.
Desktop sessions are unchanged when RemoteApp is disabled.

Create a `WindowingOrders` batch, send it through `ServerEvent::WindowingOrders`, and await the returned completion receiver.
`Written` means the transport accepted every fragment, not that the client processed the orders.
`NotStarted` means the batch was discarded before writing and can be retried on the same connection.
An error after writing starts must not be retried because some orders may already have reached the client.
Dropping the completion receiver cancels a batch only if writing has not started.

Window orders and display updates share graphics serialization, so fragmented updates stay contiguous and resize waits for an in-progress batch.
Batches exceeding the negotiated reassembly limit are rejected before writing; without the client's Multifragment Update capability, only single-fragment batches are accepted.
Split larger batches at order boundaries before submitting them.
Fields requiring extended Window List support are rejected when the client negotiated only basic support.
The embedder remains responsible for RAIL channel messages and for respecting the reported icon-cache limits.

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
Submit only for an active session: `run` and `serve` discard channel events while waiting for a connection.
There is no dispatch deadline; if the event loop is not being driven, queued payloads and permits remain held until the events are drained, discarded, or the server is dropped.
Acquire the event's owned permit from a shared, bounded Tokio semaphore before allocating the payload, cap each payload's size, and await completion before submitting another PDU.
The event retains its permit until written or discarded, even when the producer drops its completion receiver.
These limits belong to the producer; the server does not impose a byte budget.
The entire PDU holds the shared writer lock while dispatch also holds server state, so a large payload or slow transport stalls other session output, inbound PDU handling (including frame acknowledgments), and auto-reconnect cookie updates until its write finishes or fails.

Dropping the completion receiver or making `is_current` return `false` cancels a write that has not started.
Shared generation state in `is_current` lets a different owner invalidate data without owning the completion receiver.
Keep `is_current` nonblocking; it is checked again with the server-state and shared-writer locks held, so waiting on work that needs either lock can deadlock the session.
Cancellation does not remove a queued event or immediately release its permit.
Once transmission starts, producer cancellation does not interrupt it or release its permit early.
Unjoined channels and events left over from a previous connection are discarded.

[IronRDP]: https://github.com/Devolutions/IronRDP

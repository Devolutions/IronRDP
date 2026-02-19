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
 - Advanced Input DVC (`FreeRDP::Advanced::Input`) mouse events

**Dynamic channels**
 - Display Control DVC (`Microsoft::Windows::RDS::DisplayControl`) layout requests
 - ECHO DVC (`ECHO`) RTT probes
 - EGFX DVC (`Microsoft::Windows::RDS::Graphics`) when built with feature `egfx`

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

## Input and graphics hooks

- Keyboard and mouse are always hooked to `RdpServerInputHandler` from both FastPath and classic Input PDUs.
- Mouse side buttons from extended mouse events are forwarded as `Button4` / `Button5`.
- Display layout changes from Display Control DVC are forwarded to `RdpServerDisplay::request_layout`.
- With feature `egfx`, the server always attaches the Graphics DVC hook; with no custom gfx factory it uses a default no-op EGFX handler.

This crate is part of the [IronRDP] project.

## Echo RTT probes (feature `echo`)

Enable the `echo` feature to use the ECHO dynamic virtual channel (`MS-RDPEECO`) and measure round-trip time.

```rust
use ironrdp_server::RdpServer;

# async fn demo(mut server: RdpServer) -> anyhow::Result<()> {
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

		server_task.await??;
		Ok::<(), anyhow::Error>(())
	})
	.await?;
# Ok(()) }
```

`send_request` queues a probe via the server event loop. If no client has opened the ECHO channel yet, the request is dropped.

[IronRDP]: https://github.com/Devolutions/IronRDP

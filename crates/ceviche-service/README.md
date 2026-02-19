# Ceviche Service

Companion control-plane service for `ironrdp-wtsprotocol-provider`.

Current scope:

- Hosts a Windows named-pipe server for provider control commands.
- Accepts `StartListen`, `StopListen`, `WaitForIncoming`, `AcceptConnection`, and `CloseConnection` commands.
- Binds a TCP listener for `StartListen` and queues accepted sockets as incoming connection IDs.
- Returns control responses (`ListenerStarted`, `ListenerStopped`, `IncomingConnection`/`NoIncoming`, `ConnectionReady`, `Ack`).

The full IronRDP transport/data-plane integration is still in progress.

## Configuration

- Environment variable: `IRONRDP_WTS_CONTROL_PIPE`
  - Named pipe name (with or without `\\.\pipe\` prefix).
  - Default when unset: `IronRdpWtsControl`.

- Environment variable: `IRONRDP_WTS_LISTEN_ADDR`
  - TCP bind address for side-by-side incoming sockets.
  - Default when unset: `0.0.0.0:4489`.

## Build

```powershell
cargo build -p ceviche-service --release
```

## Run (debug)

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControl"
cargo run -p ceviche-service
```

## Local control-plane smoke test (no TermDD required)

You can validate the control-pipe command flow without `TermService`/`mstsc` by running
`ceviche-service` locally and driving commands directly through named-pipe IPC.

Example:

```powershell
$env:IRONRDP_WTS_CONTROL_PIPE = "IronRdpWtsControlLocal"
$env:IRONRDP_WTS_LISTEN_ADDR = "127.0.0.1:4496"
cargo run -p ceviche-service
```

In a second shell:

```powershell
.\crates\ironrdp-wtsprotocol-provider\scripts\smoke-test-control-pipe-local.ps1 \
  -PipeName IronRdpWtsControlLocal \
  -ListenerName IRDP-Tcp \
  -PortNumber 4496
```

Expected output includes:

- `listener_started`
- `incoming_connection`
- `connection_ready`
- `listener_stopped`

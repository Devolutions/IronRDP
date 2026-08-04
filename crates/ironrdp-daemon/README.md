# IronRDP Daemon

Reusable persistent RDP-session support for local RPC hosts. It owns the daemon lifecycle,
retained framebuffer, input and screenshot handling, session log buffer, NOW endpoint, and durable
NOW operation manager. The local RPC schema and transport are provided directly by
[`ironrdp-rpc`](../ironrdp-rpc).

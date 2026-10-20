# ironrdp-capture

`ironrdp-capture` decodes Ethernet pcapng captures into reassembled RDP TCP flows and decrypts supported TLS sessions from NSS-compatible key logs.
It also unwraps supported RD Gateway WebSocket and RPC-over-HTTP tunnels into their inner RDP streams.

The crate is an internal analysis building block for offline tools such as `ironrdp-capture-replay`.
It retains TLS key material only in memory and does not write decrypted payloads or secrets.

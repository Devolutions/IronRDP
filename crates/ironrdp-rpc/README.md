# IronRDP RPC

Shared local RPC protocol and transport used by `ironrdp-agent` and
`ironrdp-viewer`.

The crate provides the typed request/response schema, binary codecs, framed
messages, Unix-domain-socket transport, and Windows named-pipe transport.
It is an internal workspace crate; both endpoints must use compatible
versions of the protocol.

# IronRDP RPC

Shared local RPC protocol and transport used by IronRDP local clients.

The crate provides the typed request/response schema, binary codecs, framed messages, Unix-domain-socket
transport, and Windows named-pipe transport. It is shared by the IronRDP local RPC hosts and clients;
both endpoints must use compatible versions of the protocol.

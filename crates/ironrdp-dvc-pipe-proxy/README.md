# IronRDP DVC pipe proxy

This crate provides a Device Virtual Channel (DVC) handler for IronRDP, enabling proxying of RDP DVC
traffic over a named pipe.

It was originally designed to simplify custom DVC integration within Devolutions Remote Desktop
Manager (RDM). By implementing a thin pipe proxy for target RDP clients (such as IronRDP, FreeRDP,
mstsc, etc.), the main client logic can be centralized and reused across all supported clients via a
named pipe.

This approach allows you to implement your DVC logic in one place, making it easier to support
multiple RDP clients without duplicating code.

Additionally, this crate can be used for other scenarios, such as testing your own custom DVC
channel client, without needing to patch or rebuild IronRDP itself.

The proxy keeps the DVC open when its local pipe client disconnects and waits for a replacement client.
Dropping the proxy cancels connection attempts and retry delays.
Use `DvcNamedPipeProxy::new_async` when the write callback must wait for transport capacity.
A failed write callback stops forwarding rather than discarding bytes and continuing a corrupted stream.
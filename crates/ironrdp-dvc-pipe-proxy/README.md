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
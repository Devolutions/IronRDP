# IronRDP Viewer

A portable, lightweight RDP viewer (GUI binary) without GPU acceleration.

This crate is the desktop counterpart to the [`ironrdp-client`](../ironrdp-client/) library:
it wires the library into a [`winit`]-driven event loop and provides the CLI front-end
(`.rdp` file parsing, interactive credential prompts, etc.).

The crate was previously published as `ironrdp-client` (the binary target of the same name).

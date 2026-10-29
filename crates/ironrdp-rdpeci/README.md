# ironrdp-rdpeci

`ironrdp-rdpeci` implements the RDP Core Input Virtual Channel Extension defined by [MS-RDPECI].
Windows 11 and Windows Server 2022 and later accept keyboard and mouse input on the `Microsoft::Windows::RDS::CoreInput` dynamic channel instead of fast-path input, so the input can follow the channel onto a multitransport tunnel.

The crate provides the PDU codecs, a client processor that performs the version exchange and encodes fast-path input events for the channel, and a server processor that answers the exchange and hands decoded events to a caller-supplied handler.
Input events reuse the `ironrdp-pdu` fast-path event type, whose wire format the channel shares.
The crate performs no I/O; an integrating client registers the client processor and sends the returned channel messages through its session transport.

[MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/

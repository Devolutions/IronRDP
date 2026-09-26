# ironrdp-rdpel

`ironrdp-rdpel` implements the client side of the RDP Location Virtual Channel Extension defined by [MS-RDPEL].
It provides the location PDU codecs and the dynamic-channel state machine needed to forward caller-supplied latitude, longitude, and altitude.

The crate performs no I/O, queries no host location service, and stores coordinates only in memory for protocol delta encoding.
An integrating client must register the channel before connection, supply location updates explicitly, and deliver the returned channel messages through its session transport.

[MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/

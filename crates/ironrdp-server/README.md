# IronRDP Server

Library for implementing custom async RDP servers on the tokio runtime.

---
The server currently supports:

**Security**
 - Security-Enhanced connection with SSL Security

**Input**
 - FastPath input events
 - x224 input events and disconnect

**Codecs**
 - bitmap display updates with RDP 6.0 compression

---
Custom logic for your RDP server can be added by implementing these traits:
 - `RdpServerInputHandler` - callbacks used when the server receives input events from a client
 - `RdpServerDisplay`      - notifies the server of display updates

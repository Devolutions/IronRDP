# IronRDP Server

Extendable skeleton for implementing custom RDP servers.

For now, it requires the [Tokio runtime](https://tokio.rs/).

---

The server currently supports:

**Security**
 - Enhanced RDP Security with TLS External Security Protocols (TLS 1.2 and TLS 1.3)

**Input**
 - FastPath input events
 - x224 input events and disconnect

**Codecs**
 - bitmap display updates with RDP 6.0 compression

---

Custom logic for your RDP server can be added by implementing these traits:
 - `RdpServerInputHandler` - callbacks used when the server receives input events from a client
 - `RdpServerDisplay`      - notifies the server of display updates

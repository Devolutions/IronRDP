/// Marker trait indicating that the implementation of [`Encode`] and [`Decode`] will produce,
/// respectively consume, a fully encoded PDU frame which may be sent to or received from the peer,
/// as-is.
///
/// This is opposed to sub-structures which are merely intended to be _part_ of a PDU, but
/// which would be improper to send on the wire directly.
pub trait Pdu {}

use crate::time::MonotonicInstant;

/// What a single [`Sequence::step_input`] call has to work with.
///
/// A sequence either consumes one PDU or advances on its own. Those are
/// different things, and the difference matters: a PDU was read off the wire at
/// a moment the I/O driver observed, while a transition with no PDU has no
/// arrival to speak of. Keeping them in one type means the absence of a reading
/// is only expressible where there is nothing to have read, so an implementor
/// handed a PDU always has the instant that goes with it.
///
/// [`Sequence::step_input`]: crate::Sequence::step_input
#[derive(Clone, Copy, Debug)]
pub enum StepInput<'a> {
    /// A PDU read from the wire.
    Pdu {
        /// The PDU bytes, as delimited by the hint the sequence asked for.
        pdu: &'a [u8],
        /// When the read that carried `pdu` completed, as observed by the I/O
        /// driver that performed it.
        ///
        /// This is the driver's clock, not the sequence's: a sequence reading a
        /// clock itself would measure how quickly it drained an already-filled
        /// buffer rather than when the bytes arrived. Instants from different
        /// drivers have different epochs and must not be compared; see
        /// [`MonotonicInstant`].
        received_at: MonotonicInstant,
    },
    /// No PDU: the sequence advances on its own.
    NoPdu,
}

impl<'a> StepInput<'a> {
    /// The PDU bytes, empty when there is no PDU.
    #[must_use]
    pub fn pdu(self) -> &'a [u8] {
        match self {
            Self::Pdu { pdu, .. } => pdu,
            Self::NoPdu => &[],
        }
    }

    /// When the PDU arrived, or `None` when there is no PDU to have arrived.
    #[must_use]
    pub fn received_at(self) -> Option<MonotonicInstant> {
        match self {
            Self::Pdu { received_at, .. } => Some(received_at),
            Self::NoPdu => None,
        }
    }
}

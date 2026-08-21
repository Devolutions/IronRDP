use ironrdp_core::WriteBuf;
use ironrdp_pdu::PduHint;

use crate::sequence_error::SequenceResult;
use crate::state::State;
use crate::time::MonotonicInstant;
use crate::written::Written;

/// A single step of a sans-I/O PDU state machine.
///
/// Implementors drive one phase of an RDP connect or accept sequence (e.g.
/// negotiation, channel connection, license exchange, activation,
/// finalization). Each `step` call consumes at most one input PDU and
/// produces at most one output PDU, leaving all I/O to the caller.
pub trait Sequence: Send {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint>;

    fn state(&self) -> &dyn State;

    /// Advances the sequence.
    ///
    /// `received_at` is when `input` arrived on the wire, as observed by the I/O
    /// driver, or `None` from a driver that does not observe arrival times. The
    /// absence of a reading is deliberately not expressible as an instant: a
    /// driver that cannot measure has taken no measurement, which is a different
    /// thing from one that measured no elapsed time, and only the sequence
    /// knows which of the two its reply may be derived from.
    ///
    /// A driver that always passes `None` never opens a connect-time bandwidth
    /// window, so the Bandwidth Measure Results it sends report only the Stop's
    /// own payload against the untimed floor. See `ironrdp-connector`'s
    /// `connection::counted_len` doc for why the byte count is
    /// measurement-gated rather than reported in full.
    fn step(
        &mut self,
        input: &[u8],
        received_at: Option<MonotonicInstant>,
        output: &mut WriteBuf,
    ) -> SequenceResult<Written>;

    fn step_no_input(&mut self, output: &mut WriteBuf) -> SequenceResult<Written> {
        self.step(&[], None, output)
    }
}

ironrdp_core::assert_obj_safe!(Sequence);

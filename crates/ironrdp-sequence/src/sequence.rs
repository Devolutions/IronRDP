use ironrdp_core::WriteBuf;
use ironrdp_pdu::PduHint;

use crate::sequence_error::SequenceResult;
use crate::state::State;
use crate::step_input::StepInput;
use crate::time::MonotonicInstant;
use crate::written::Written;

/// A single step of a sans-I/O PDU state machine.
///
/// Implementors drive one phase of an RDP connect or accept sequence (e.g.
/// negotiation, channel connection, license exchange, activation,
/// finalization). Each step consumes at most one input PDU and produces at most
/// one output PDU, leaving all I/O to the caller.
///
/// [`step_input`](Self::step_input) is the one transition to implement.
/// [`step`](Self::step) and [`step_no_input`](Self::step_no_input) are the two
/// ways to call it, and exist so that a caller cannot hand over a PDU without
/// saying when it arrived. Do not override them.
pub trait Sequence: Send {
    fn next_pdu_hint(&self) -> Option<&dyn PduHint>;

    fn state(&self) -> &dyn State;

    /// Advances the sequence, consuming `input`.
    fn step_input(&mut self, input: StepInput<'_>, output: &mut WriteBuf) -> SequenceResult<Written>;

    /// Advances the sequence with the PDU the last
    /// [`next_pdu_hint`](Self::next_pdu_hint) asked for.
    ///
    /// `received_at` is when the driver's read of `pdu` completed. A driver that
    /// serves several PDUs out of a single read reports that read's instant for
    /// all of them, including the ones it hands over later: they arrived
    /// together, whenever the caller got round to draining them.
    fn step(&mut self, pdu: &[u8], received_at: MonotonicInstant, output: &mut WriteBuf) -> SequenceResult<Written> {
        self.step_input(StepInput::Pdu { pdu, received_at }, output)
    }

    /// Advances the sequence without a PDU, for the states that move on their
    /// own.
    fn step_no_input(&mut self, output: &mut WriteBuf) -> SequenceResult<Written> {
        self.step_input(StepInput::NoPdu, output)
    }
}

ironrdp_core::assert_obj_safe!(Sequence);

#[cfg(test)]
mod tests {
    use super::*;

    /// Records the input of the last transition, so a test can check what the two calling
    /// methods actually hand to the one method implementors write.
    #[derive(Default)]
    struct Recorder {
        last: Option<(Vec<u8>, Option<MonotonicInstant>)>,
    }

    impl Sequence for Recorder {
        fn next_pdu_hint(&self) -> Option<&dyn PduHint> {
            None
        }

        fn state(&self) -> &dyn State {
            &()
        }

        fn step_input(&mut self, input: StepInput<'_>, _: &mut WriteBuf) -> SequenceResult<Written> {
            self.last = Some((input.pdu().to_vec(), input.received_at()));
            Ok(Written::Nothing)
        }
    }

    #[test]
    fn step_hands_the_pdu_and_its_arrival_time_through() {
        let mut sequence = Recorder::default();
        let received_at = MonotonicInstant::from_millis(1_234);

        sequence
            .step(&[0xAA, 0xBB], received_at, &mut WriteBuf::new())
            .expect("step");

        assert_eq!(sequence.last, Some((vec![0xAA, 0xBB], Some(received_at))));
    }

    /// A transition with no PDU has no arrival to report, and must not invent one. This is the
    /// only way an implementor ever sees no instant.
    #[test]
    fn step_no_input_reports_no_pdu_and_no_arrival_time() {
        let mut sequence = Recorder::default();

        sequence.step_no_input(&mut WriteBuf::new()).expect("step_no_input");

        assert_eq!(sequence.last, Some((Vec::new(), None)));
    }
}

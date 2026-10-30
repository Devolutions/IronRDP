//! Pieces shared by the client and server channel processors.

use std::sync::{Mutex, MutexGuard, PoisonError};

use ironrdp_core::{DecodeResult, ReadCursor};
use ironrdp_pdu::{PduResult, pdu_other_err};
use tracing::warn;

use crate::QueueFull;
use crate::pdu::{PduLayout, read_frame};

/// Locks state shared by two channel processors.
///
/// A panic while one processor held the lock leaves the state as consistent as between two PDUs,
/// so the other processor keeps using it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Decodes each PDU of `payload` under the layout `state` negotiated so far and applies it.
///
/// A PDU that does not decode is skipped, a malformed header ends the message because the next
/// PDU cannot be located, and a reply `apply` cannot queue fails the message.
pub(crate) fn process_frames<S, P>(
    payload: &[u8],
    state: &mut S,
    layout: fn(&S) -> PduLayout,
    decode_body: fn(u16, &[u8], PduLayout) -> DecodeResult<P>,
    apply: fn(&mut S, P) -> Result<(), QueueFull>,
) -> PduResult<()> {
    let mut src = ReadCursor::new(payload);
    while !src.is_empty() {
        let (pdu_id, body) = match read_frame(&mut src) {
            Ok(frame) => frame,
            Err(error) => {
                warn!(%error, "Ignoring the rest of a malformed text input message");
                break;
            }
        };
        // The version PDU comes first, so the layout is known for the PDUs that depend on it.
        match decode_body(pdu_id, body, layout(state)) {
            Ok(pdu) => apply(state, pdu).map_err(|error| pdu_other_err!("text input reply", source: error))?,
            Err(error) => warn!(pdu_id, %error, "Ignoring an undecodable text input PDU"),
        }
    }
    Ok(())
}

//! Pieces shared by the client and server channel processors.

use std::sync::{Mutex, MutexGuard, PoisonError};

use ironrdp_core::{DecodeResult, ReadCursor};
use ironrdp_dvc::{DvcEncode, DvcMessage};
use tracing::warn;

use crate::QueueFull;
use crate::pdu::{PduLayout, VersionedPdu, read_frame};

/// Locks state shared by two channel processors.
///
/// A panic while one processor held the lock leaves the state as consistent as between two PDUs,
/// so the other processor keeps using it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Boxes `pdus` for a dynamic channel, encoded under `layout`.
pub(crate) fn versioned_messages<P>(pdus: Vec<P>, layout: PduLayout) -> Vec<DvcMessage>
where
    VersionedPdu<P>: DvcEncode + Send + 'static,
{
    pdus.into_iter()
        .map(|pdu| -> DvcMessage { Box::new(VersionedPdu { pdu, layout }) })
        .collect()
}

/// Decodes each PDU of `payload` under the layout `state` negotiated so far and applies it.
///
/// A PDU that does not decode is skipped, and a malformed header ends the message because the
/// next PDU cannot be located.
/// When a reply does not fit the queue, the rest of the message is left unprocessed, so the
/// state never runs ahead of the replies; the connection itself carries on.
pub(crate) fn process_frames<S, P>(
    state: &mut S,
    payload: &[u8],
    layout: fn(&S) -> PduLayout,
    decode_body: fn(u16, &[u8], PduLayout) -> DecodeResult<P>,
    apply: fn(&mut S, P) -> Result<(), QueueFull>,
) {
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
            Ok(pdu) => {
                if let Err(error) = apply(state, pdu) {
                    warn!(pdu_id, %error, "Ignoring the rest of a text input message");
                    break;
                }
            }
            Err(error) => warn!(pdu_id, %error, "Ignoring an undecodable text input PDU"),
        }
    }
}

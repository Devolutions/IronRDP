//! Text input ([MS-RDPETXT]) wiring.
//!
//! The server sends on one channel and the client answers on the other, so the replies queued
//! while the server-to-client channel processes a message are sent after the session iteration.
//!
//! [MS-RDPETXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/

use std::sync::{Arc, PoisonError};

use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
use ironrdp_rdpetxt::client::{ClientToServerChannel, ServerToClientChannel, TextInputClient};
use ironrdp_session::ActiveStage;
use tracing::{debug, warn};

/// Registers both text input channels around one shared client.
pub(crate) fn attach(drdynvc: DrdynvcClient) -> DrdynvcClient {
    let client = TextInputClient::default().into_shared();
    let server_to_client = Arc::clone(&client);
    // The server may close and reopen the channels during a session, so each opening gets a new
    // processor sharing the client.
    drdynvc
        .with_dynamic_channel_factory(ironrdp_rdpetxt::SERVER_TO_CLIENT_CHANNEL_NAME, move || {
            ServerToClientChannel::new(Arc::clone(&server_to_client))
        })
        .with_dynamic_channel_factory(ironrdp_rdpetxt::CLIENT_TO_SERVER_CHANNEL_NAME, move || {
            ClientToServerChannel::new(Arc::clone(&client))
        })
}

/// Moves the queued replies into `dvc_batch` when the iteration has no other dynamic channel
/// batch.
///
/// Returns whether `dvc_batch` now holds them, in which case they must be routed for the
/// client-to-server channel rather than for the request that produced the iteration.
pub(crate) fn flush(active_stage: &mut ActiveStage, dvc_batch: &mut Option<DvcMessageBatch>) -> bool {
    // UNTESTED: replies wait in the shared client while another batch occupies the iteration
    // and leave with the next iteration that has none.
    // The server waits for some of them, such as the version reply and text change
    // acknowledgements, so a change here must not drop or reorder them.
    if dvc_batch.as_ref().is_some_and(|batch| !batch.messages().is_empty()) {
        return false;
    }
    match pending_batch(active_stage) {
        Some(batch) => {
            *dvc_batch = Some(batch);
            true
        }
        None => false,
    }
}

/// Inserts `text` into the focused remote edit control.
pub(crate) fn commit_text(active_stage: &mut ActiveStage, text: &str) -> Option<DvcMessageBatch> {
    let Some(channel) = active_stage.get_dvc::<ClientToServerChannel>() else {
        debug!("Dropping committed text because the text input channel is not open");
        return None;
    };
    let committed = channel
        .processor()
        .client()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .commit_text(text);
    if committed.is_none() {
        debug!("Dropping committed text because no remote edit control has focus");
        return None;
    }
    pending_batch(active_stage)
}

fn pending_batch(active_stage: &mut ActiveStage) -> Option<DvcMessageBatch> {
    let mut channel = active_stage.get_dvc_mut::<ClientToServerChannel>()?;
    let channel_id = channel.channel_id();
    match channel
        .processor_mut()
        .pending_messages()
        .and_then(|messages| DvcMessageBatch::try_new(channel_id, messages))
    {
        Ok(batch) if !batch.messages().is_empty() => Some(batch),
        Ok(_) => None,
        Err(error) => {
            warn!(%error, "Unable to send text input PDUs");
            None
        }
    }
}

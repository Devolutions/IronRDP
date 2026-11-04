use core::pin::Pin;
use core::time::Duration;
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use ironrdp_dvc::encode_dvc_messages;
use ironrdp_pdu::PduResult;
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tokio::sync::Notify;
use tracing::{debug, error};

use crate::error::DvcPipeProxyError;
use crate::message::RawDataDvcMessage;
use crate::os_pipe::OsPipe;

const IO_BUFFER_SIZE: usize = 1024 * 64; // 64K
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_millis(100);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub(crate) type WriteDvcFuture = Pin<Box<dyn Future<Output = PduResult<()>> + Send>>;
/// Shared by every worker the proxy starts; the lock stands in for a `Sync` bound on the callback.
pub(crate) type OnWriteDvcMessage = Arc<Mutex<dyn Fn(u32, Vec<SvcMessage>) -> WriteDvcFuture + Send>>;

pub(crate) struct WorkerCtx {
    pub(crate) on_write_dvc: OnWriteDvcMessage,
    pub(crate) to_pipe_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    pub(crate) abort_event: Arc<Notify>,
    pub(crate) pipe_name: String,
    pub(crate) channel_name: String,
    pub(crate) channel_id: u32,
}

pub(crate) fn run_worker<P: OsPipe>(ctx: WorkerCtx) -> std::io::Result<std::thread::JoinHandle<()>> {
    let thread_name = format!("ironrdp-dvc-pipe-{}", ctx.channel_id);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);

    let thread = std::thread::Builder::new().name(thread_name).spawn(move || {
        let channel_name = ctx.channel_name.clone();
        let pipe_name = ctx.pipe_name.clone();
        debug!(%channel_name, %pipe_name, "Starting DVC pipe proxy worker thread");

        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(
                    %channel_name,
                    %pipe_name,
                    %error,
                    "Failed to initialize DVC pipe proxy worker thread"
                );
                let _ = startup_tx.send(Err(error));
                return;
            }
        };

        if startup_tx.send(Ok(())).is_err() {
            return;
        }

        debug!(
            channel_name = %ctx.channel_name,
            pipe_name = %ctx.pipe_name,
            "Started DVC pipe proxy worker thread"
        );
        if let Err(error) = runtime.block_on(worker::<P>(ctx)) {
            error!(?error, "DVC pipe proxy worker thread has failed");
        }
    })?;

    startup_rx.recv().unwrap_or_else(|_| {
        Err(std::io::Error::other(
            "dvc pipe proxy worker stopped before startup completed",
        ))
    })?;
    Ok(thread)
}

enum NextWorkerState {
    Abort,
    Reconnect,
}

async fn process_client<P: OsPipe>(ctx: &mut WorkerCtx) -> Result<NextWorkerState, DvcPipeProxyError> {
    let pipe_name = &ctx.pipe_name;
    let channel_name = &ctx.channel_name;

    let mut pipe = tokio::select! {
        pipe = P::connect(pipe_name) => {
            debug!(%channel_name, %pipe_name, "DVC proxy worker thread has started");
            pipe?
        }
        _ = ctx.abort_event.notified() => {
            debug!(%channel_name, %pipe_name, "DVC proxy worker thread has been aborted");
            return Ok(NextWorkerState::Abort);
        }
    };

    let mut from_pipe_buffer = [0u8; IO_BUFFER_SIZE];

    loop {
        let abort = ctx.abort_event.notified();
        let read_pipe = pipe.read(&mut from_pipe_buffer);
        let read_dvc = ctx.to_pipe_rx.recv();

        tokio::select! {
            () = abort => {
                debug!(%channel_name, %pipe_name, "Received abort signal for DVC proxy worker thread");
                return Ok(NextWorkerState::Abort);
            }
            read_bytes_result = read_pipe => {
                let read_bytes = read_bytes_result?;

                if read_bytes == 0 {
                    debug!(%channel_name, %pipe_name, "DVC proxy pipe returned EOF");

                    // If client unexpectedly closed the connection, we should
                    // still be able to reconnect to same session.
                    return Ok(NextWorkerState::Reconnect);
                }

                let messages = encode_dvc_messages(
                    ctx.channel_id,
                    vec![Box::new(RawDataDvcMessage(from_pipe_buffer[..read_bytes].to_vec()))],
                    ChannelFlags::empty(),
                )
                .map_err(DvcPipeProxyError::EncodeDvcMessage)?;

                let write = {
                    let on_write_dvc = ctx.on_write_dvc.lock().unwrap_or_else(PoisonError::into_inner);
                    on_write_dvc(ctx.channel_id, messages)
                };
                let result = tokio::select! {
                    () = ctx.abort_event.notified() => return Ok(NextWorkerState::Abort),
                    result = write => result,
                };
                if let Err(error) = result {
                    error!(%channel_name, %pipe_name, ?error, "DVC pipe proxy write callback failed");
                    return Ok(NextWorkerState::Abort);
                }
            }
            dvc_input = read_dvc => {
                let data = match dvc_input {
                    Some(data) => data,
                    None => {
                        debug!(%channel_name, %pipe_name, "DVC mpsc channel returned EOF");
                        // Server DVC has been closed, there is no point in
                        // trying to reconnect.
                        return Ok(NextWorkerState::Abort);
                    }
                };

                let result = tokio::select! {
                    () = ctx.abort_event.notified() => return Ok(NextWorkerState::Abort),
                    result = pipe.write_all(&data) => result,
                };
                if let Err(error) = result {
                    error!(%channel_name, %pipe_name, ?error, "Failed to write to DVC pipe");
                    return Ok(NextWorkerState::Reconnect);
                }
            }
        };
    }
}

async fn worker<P: OsPipe>(mut bridged_ctx: WorkerCtx) -> Result<(), DvcPipeProxyError> {
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;

    loop {
        match process_client::<P>(&mut bridged_ctx).await {
            Err(error) => {
                error!(
                    channel_name = %bridged_ctx.channel_name,
                    pipe_name = %bridged_ctx.pipe_name,
                    ?error,
                    retry_delay_ms = reconnect_delay.as_millis(),
                    "DVC pipe proxy connection failed; retrying"
                );
                // Dropped Windows pipes need the I/O driver to finish canceled operations.
                tokio::select! {
                    () = bridged_ctx.abort_event.notified() => break,
                    () = tokio::time::sleep(reconnect_delay) => {}
                }
                reconnect_delay = reconnect_delay.saturating_mul(2).min(MAX_RECONNECT_DELAY);
            }
            Ok(NextWorkerState::Abort) => {
                debug!(
                    channel_name = %bridged_ctx.channel_name,
                    pipe_name = %bridged_ctx.pipe_name,
                    "Abort DVC proxy worker thread"
                );
                break;
            }
            Ok(NextWorkerState::Reconnect) => {
                reconnect_delay = INITIAL_RECONNECT_DELAY;
                debug!(
                    channel_name = %bridged_ctx.channel_name,
                    pipe_name = %bridged_ctx.pipe_name,
                    "Reconnect to DVC pipe"
                );
                continue;
            }
        };
    }

    Ok(())
}

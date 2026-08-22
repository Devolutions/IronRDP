use core::time::Duration;
use std::sync::{Arc, mpsc};

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

pub(crate) type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send>;

pub(crate) struct WorkerCtx {
    pub(crate) on_write_dvc: OnWriteDvcMessage,
    pub(crate) to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    pub(crate) abort_event: Arc<Notify>,
    pub(crate) pipe_name: String,
    pub(crate) channel_name: String,
    pub(crate) channel_id: u32,
}

pub(crate) fn run_worker<P: OsPipe>(ctx: WorkerCtx) -> std::io::Result<()> {
    let thread_name = format!("ironrdp-dvc-pipe-{}", ctx.channel_id);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);

    std::thread::Builder::new().name(thread_name).spawn(move || {
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

        let (async_tx, async_rx) = tokio::sync::mpsc::unbounded_channel();
        let WorkerCtx {
            on_write_dvc,
            to_pipe_rx: std_rx,
            abort_event,
            pipe_name,
            channel_name,
            channel_id,
        } = ctx;

        let bridge_thread_name = format!("ironrdp-dvc-pipe-{channel_id}-bridge");
        if let Err(error) = std::thread::Builder::new().name(bridge_thread_name).spawn(move || {
            while let Ok(data) = std_rx.recv() {
                if async_tx.send(data).is_err() {
                    break; // Receiver dropped
                }
            }
        }) {
            error!(
                %channel_name,
                %pipe_name,
                %error,
                "Failed to start DVC pipe proxy bridge thread"
            );
            let _ = startup_tx.send(Err(error));
            return;
        }

        let ctx = BridgedWorkerCtx {
            on_write_dvc,
            to_pipe_rx: async_rx,
            abort_event,
            pipe_name,
            channel_name,
            channel_id,
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
    })
}

enum NextWorkerState {
    Abort,
    Reconnect,
}

struct BridgedWorkerCtx {
    on_write_dvc: OnWriteDvcMessage,
    to_pipe_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    abort_event: Arc<Notify>,
    pipe_name: String,
    channel_name: String,
    channel_id: u32,
}

async fn process_client<P: OsPipe>(ctx: &mut BridgedWorkerCtx) -> Result<NextWorkerState, DvcPipeProxyError> {
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

                if let Err(error) = (ctx.on_write_dvc)(0, messages) {
                    error!(%channel_name, %pipe_name, ?error, "DVC pipe proxy write callback failed");
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

                if let Err(error) = pipe.write_all(&data).await
                {
                    error!(%channel_name, %pipe_name, ?error, "Failed to write to DVC pipe");
                    return Ok(NextWorkerState::Reconnect);
                }
            }
        };
    }
}

async fn worker<P: OsPipe>(mut bridged_ctx: BridgedWorkerCtx) -> Result<(), DvcPipeProxyError> {
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
                std::thread::sleep(reconnect_delay);
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

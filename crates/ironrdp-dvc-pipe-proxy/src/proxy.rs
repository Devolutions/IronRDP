use core::future::ready;
use std::sync::{Arc, Mutex};

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_svc::SvcMessage;
use tokio::sync::mpsc;
use tracing::{debug, error};

use crate::worker::{OnWriteDvcMessage, WorkerCtx, WriteDvcFuture, run_worker};

const IO_MPSC_CHANNEL_SIZE: usize = 100;

struct WorkerControlCtx {
    to_pipe_tx: mpsc::Sender<Vec<u8>>,
    abort_event: Arc<tokio::sync::Notify>,
    thread: std::thread::JoinHandle<()>,
}

/// A proxy DVC pipe client that forwards DVC messages to/from a named pipe server.
pub struct DvcNamedPipeProxy {
    channel_name: String,
    named_pipe_name: String,
    dvc_write_callback: OnWriteDvcMessage,
    worker: Option<WorkerControlCtx>,
}

impl DvcNamedPipeProxy {
    /// Creates a new DVC named pipe proxy.
    /// `dvc_write_callback` is called when the proxy receives a DVC message from the
    /// named pipe server and the SVC message is ready to be sent to the DVC channel in the main
    /// IronRDP active session loop.
    pub fn new<F>(channel_name: &str, named_pipe_name: &str, dvc_write_callback: F) -> Self
    where
        F: Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send + 'static,
    {
        Self::new_async(channel_name, named_pipe_name, move |channel_id, messages| {
            ready(dvc_write_callback(channel_id, messages))
        })
    }

    /// Creates a proxy whose write callback can await transport capacity without losing stream bytes.
    ///
    /// Callback failure stops the worker; dropping the proxy cancels a pending callback.
    pub fn new_async<F, Fut>(channel_name: &str, named_pipe_name: &str, dvc_write_callback: F) -> Self
    where
        F: Fn(u32, Vec<SvcMessage>) -> Fut + Send + 'static,
        Fut: Future<Output = PduResult<()>> + Send + 'static,
    {
        Self {
            channel_name: channel_name.to_owned(),
            named_pipe_name: named_pipe_name.to_owned(),
            dvc_write_callback: Arc::new(Mutex::new(move |channel_id, messages| -> WriteDvcFuture {
                Box::pin(dvc_write_callback(channel_id, messages))
            })),
            worker: None,
        }
    }

    fn stop_worker(&mut self) {
        if let Some(ctx) = self.worker.take() {
            debug!(
                channel_name = %self.channel_name,
                pipe_name = %self.named_pipe_name,
                "Stopping DVC named pipe proxy worker"
            );
            ctx.abort_event.notify_one();
            // Runtime teardown waits for filesystem operations before the pipe path is reused.
            if let Err(error) = ctx.thread.join() {
                error!(?error, "DVC pipe proxy worker thread panicked");
            }
        }
    }
}

impl_as_any!(DvcNamedPipeProxy);

impl DvcProcessor for DvcNamedPipeProxy {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(%self.channel_name, %self.named_pipe_name, "Starting DVC named pipe proxy");

        // A server that closed the channel can open it again with the same proxy.
        self.stop_worker();
        let on_write_dvc = Arc::clone(&self.dvc_write_callback);

        let (to_pipe_tx, to_pipe_rx) = mpsc::channel(IO_MPSC_CHANNEL_SIZE);

        let abort_event = Arc::new(tokio::sync::Notify::new());

        let ctx = WorkerCtx {
            on_write_dvc,
            to_pipe_rx,
            abort_event: Arc::clone(&abort_event),
            pipe_name: self.named_pipe_name.clone(),
            channel_name: self.channel_name.clone(),
            channel_id,
        };

        #[cfg(not(target_os = "windows"))]
        let worker = run_worker::<crate::platform::unix::UnixPipe>(ctx);

        #[cfg(target_os = "windows")]
        let worker = run_worker::<crate::platform::windows::WindowsPipe>(ctx);

        let thread = worker.map_err(|error| {
            error!(
                channel_name = %self.channel_name,
                pipe_name = %self.named_pipe_name,
                %error,
                "Failed to start DVC pipe proxy worker thread"
            );
            pdu_other_err!("start DVC pipe proxy worker: {error}")
        })?;

        self.worker = Some(WorkerControlCtx {
            to_pipe_tx,
            abort_event,
            thread,
        });

        Ok(vec![])
    }

    fn close(&mut self, _channel_id: u32) {
        self.stop_worker();
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if let Some(worker) = &self.worker {
            match worker.to_pipe_tx.try_send(payload.to_vec()) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    return Err(pdu_other_err!("DVC pipe proxy outbound queue is full"));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(pdu_other_err!("DVC pipe proxy channel is closed"));
                }
            }
        } else {
            debug!("Attempt to process DVC packet on non-initialized DVC pipe proxy.");
        }

        Ok(vec![])
    }
}

impl DvcClientProcessor for DvcNamedPipeProxy {}

impl Drop for DvcNamedPipeProxy {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

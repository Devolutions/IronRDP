use std::sync::Arc;

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{pdu_other_err, PduResult};
use ironrdp_svc::SvcMessage;

use crate::worker::{run_worker, OnWriteDvcMessage, WorkerCtx};

const IO_MPSC_CHANNEL_SIZE: usize = 100;

struct WorkerControlCtx {
    to_pipe_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    abort_event: Arc<tokio::sync::Notify>,
}

/// A proxy DVC pipe client that forwards DVC messages to/from a named pipe server.
pub struct DvcNamedPipeProxy {
    channel_name: String,
    named_pipe_name: String,
    dvc_write_callback: Option<OnWriteDvcMessage>,
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
        Self {
            channel_name: channel_name.to_owned(),
            named_pipe_name: named_pipe_name.to_owned(),
            dvc_write_callback: Some(Box::new(dvc_write_callback)),
            worker: None,
        }
    }
}

impl_as_any!(DvcNamedPipeProxy);

impl DvcProcessor for DvcNamedPipeProxy {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        info!(%self.channel_name, %self.named_pipe_name, "Starting DVC named pipe proxy");

        let on_write_dvc = self
            .dvc_write_callback
            .take()
            .expect("DvcProcessor::start called multiple times");

        let (to_pipe_tx, to_pipe_rx) = tokio::sync::mpsc::channel(IO_MPSC_CHANNEL_SIZE);

        let abort_event = Arc::new(tokio::sync::Notify::new());

        let ctx = WorkerCtx {
            on_write_dvc,
            to_pipe_rx,
            abort_event: Arc::clone(&abort_event),
            pipe_name: self.named_pipe_name.clone(),
            channel_name: self.channel_name.clone(),
            channel_id,
        };

        self.worker = Some(WorkerControlCtx {
            to_pipe_tx,
            abort_event,
        });

        #[cfg(not(target_os = "windows"))]
        run_worker::<crate::platform::unix::UnixPipe>(ctx);

        #[cfg(target_os = "windows")]
        run_worker::<crate::platform::windows::WindowsPipe>(ctx);

        Ok(vec![])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if let Some(worker) = &self.worker {
            if let Err(error) = worker.to_pipe_tx.try_send(payload.to_vec()) {
                match error {
                    tokio::sync::mpsc::error::TrySendError::Full(_) => {
                        return Err(pdu_other_err!("DVC pipe proxy channel is full"));
                    }
                    tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                        return Err(pdu_other_err!("DVC pipe proxy channel is closed"));
                    }
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
        if let Some(ctx) = &self.worker {
            // Signal the worker thread to abort.
            ctx.abort_event.notify_one();
        }
        self.worker = None;
    }
}

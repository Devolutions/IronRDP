mod error;
mod worker;

use std::sync::mpsc;

use error::DvcPipeProxyError;
use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::{pdu_other_err, PduResult};
use ironrdp_svc::SvcMessage;
use worker::{worker_thread_func, OnWriteDvcMessage, WorkerCtx};

use crate::windows::{Event, MessagePipeServer, Semaphore};

const IO_MPSC_CHANNEL_SIZE: usize = 100;

struct WorkerControlCtx {
    to_pipe_tx: mpsc::SyncSender<Vec<u8>>,
    to_pipe_semaphore: Semaphore,
    abort_event: Event,
}

/// A proxy DVC pipe client that forwards DVC messages to/from a named pipe server.
pub struct DvcNamedPipeProxy {
    channel_name: String,
    named_pipe_name: String,
    dvc_write_callback: Option<OnWriteDvcMessage>,
    worker_control_ctx: Option<WorkerControlCtx>,
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
        let named_pipe_name = format!("\\\\.\\pipe\\{named_pipe_name}");

        Self {
            channel_name: channel_name.to_owned(),
            named_pipe_name,
            dvc_write_callback: Some(Box::new(dvc_write_callback)),
            worker_control_ctx: None,
        }
    }
}

impl_as_any!(DvcNamedPipeProxy);

impl Drop for DvcNamedPipeProxy {
    fn drop(&mut self) {
        if let Some(ctx) = &self.worker_control_ctx {
            // Signal the worker thread to abort.
            ctx.abort_event.set().ok();
        }
    }
}

impl DvcNamedPipeProxy {
    fn start_impl(&mut self, channel_id: u32) -> Result<(), DvcPipeProxyError> {
        // PIPE -> DVC channel - handled via callback passed to the constructor
        // DVC -> PIPE channel - handled via mpsc internally in the worker thread
        let (to_pipe_tx, to_pipe_rx) = mpsc::sync_channel(IO_MPSC_CHANNEL_SIZE);

        let semaphore_max_count = IO_MPSC_CHANNEL_SIZE
            .try_into()
            .expect("Channel size is too large for underlying WinAPI semaphore");

        let to_pipe_semaphore = Semaphore::new_unnamed(0, semaphore_max_count)?;

        let abort_event = Event::new_unnamed()?;

        let worker_control_ctx = WorkerControlCtx {
            to_pipe_tx,
            to_pipe_semaphore: to_pipe_semaphore.clone(),
            abort_event: abort_event.clone(),
        };

        let pipe = MessagePipeServer::new(&self.named_pipe_name)?;

        let dvc_write_callback = self
            .dvc_write_callback
            .take()
            .expect("DVC write callback already taken");

        let worker_ctx = WorkerCtx {
            pipe,
            to_pipe_rx,
            to_pipe_semaphore,
            abort_event,
            dvc_write_callback,
            pipe_name: self.named_pipe_name.clone(),
            channel_name: self.channel_name.clone(),
            channel_id,
        };

        let pipe_name = self.named_pipe_name.clone();
        let channel_name = self.channel_name.clone();

        self.worker_control_ctx = Some(worker_control_ctx);

        std::thread::spawn(move || {
            if let Err(error) = worker_thread_func(worker_ctx) {
                error!(%error, %pipe_name, %channel_name, "DVC pipe proxy worker thread failed");
            }
        });

        Ok(())
    }
}

impl DvcProcessor for DvcNamedPipeProxy {
    fn channel_name(&self) -> &str {
        &self.channel_name
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.start_impl(channel_id)
            .map_err(|e| pdu_other_err!("dvc named pipe proxy failed", source: e))?;

        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // Send the payload to the worker thread via the mpsc channel.

        let ctx = match &self.worker_control_ctx {
            Some(ctx) => ctx,
            None => {
                return Err(pdu_other_err!("DVC pipe proxy not started"));
            }
        };

        ctx.to_pipe_tx
            .send(payload.to_vec())
            .map_err(|_| pdu_other_err!("DVC pipe proxy send failed"))?;

        // Signal WinAPI-based worker IO loop.
        ctx.to_pipe_semaphore
            .release(1)
            .map_err(|_| pdu_other_err!("DVC pipe proxy semaphore release failed"))?;

        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DvcNamedPipeProxy {}

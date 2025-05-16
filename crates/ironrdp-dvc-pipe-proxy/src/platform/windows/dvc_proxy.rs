use ironrdp_core::{ensure_size, impl_as_any, Encode, EncodeResult};
use ironrdp_dvc::{encode_dvc_messages, DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_pdu::{pdu_other_err, PduResult};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use std::sync::{mpsc};
use crate::windows::{wait_any, wait_any_with_timeout, Event, MessagePipeServer, Semaphore, WindowsError};

const PIPE_CONNECT_TIMEOUT: u32 = 10_000; // 10 seconds
const PIPE_WRITE_TIMEOUT: u32 = 3_000; // 3 seconds
const IO_MPSC_CHANNEL_SIZE: usize = 100;
const MESSAGE_BUFFER_SIZE: usize = 64 * 1024; // 64 KiB


#[derive(Debug)]
pub enum DvcPipeProxyError {
    Windows(WindowsError),
    MpscIo,
    DvcIncompleteWrite,
    EncodeDvcMessage,
    ConnectTimeout,
}

impl From<WindowsError> for DvcPipeProxyError {
    fn from(err: WindowsError) -> Self {
        DvcPipeProxyError::Windows(err)
    }
}

impl std::fmt::Display for DvcPipeProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DvcPipeProxyError::Windows(err) => return err.fmt(f),
            DvcPipeProxyError::MpscIo => write!(f, "MPSC IO error"),
            DvcPipeProxyError::DvcIncompleteWrite => write!(f, "DVC incomplete write"),
            DvcPipeProxyError::EncodeDvcMessage => write!(f, "DVC message encoding error"),
            DvcPipeProxyError::ConnectTimeout => write!(f, "DVC connect timeout"),
        }
    }
}

impl std::error::Error for DvcPipeProxyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DvcPipeProxyError::Windows(err) => Some(err),
            DvcPipeProxyError::MpscIo => None,
            DvcPipeProxyError::DvcIncompleteWrite => None,
            DvcPipeProxyError::EncodeDvcMessage => None,
            DvcPipeProxyError::ConnectTimeout => None,
        }
    }
}


struct WorkerControlCtx {
    to_pipe_tx: mpsc::SyncSender<Vec<u8>>,
    to_pipe_semaphore: Semaphore,
    abort_event: Event,
}

struct WorkerCtx {
    pipe: MessagePipeServer,
    to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    to_pipe_semaphore: Semaphore,
    abort_event: Event,
    dvc_write_callback: OnWriteDvcMessage,
    pipe_name: String,
    channel_name: String,
    channel_id: u32,
}

/// A client for the Display Control Virtual Channel.
pub struct DvcNamedPipeProxy {
    channel_name: String,
    named_pipe_name: String,
    dvc_write_callback: Option<OnWriteDvcMessage>,

    worker_control_ctx: Option<WorkerControlCtx>,

    // DVC -> process -> MPSC(on_pipe_write) -> worker -> pipe
    // pipe -> worker -> callback -> session write -> DVC
}

impl DvcNamedPipeProxy {
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

struct GenericDvcMessage(Vec<u8>);

impl GenericDvcMessage {
    fn new(data: Vec<u8>) -> Self {
        Self(data)
    }
}

impl Encode for GenericDvcMessage {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "GenericDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for GenericDvcMessage {}

fn worker_thread_func(
    worker_ctx: WorkerCtx,
) -> Result<(), DvcPipeProxyError> {
    let WorkerCtx {
        mut pipe,
        to_pipe_rx,
        to_pipe_semaphore,
        abort_event,
        dvc_write_callback,
        pipe_name,
        channel_name,
        channel_id,
    } = worker_ctx;

    info!(%channel_name, %pipe_name, "Connecting DVC pipe proxy");

    {
        let mut connect_ctx = pipe.prepare_connect_overlapped()?;

        if !connect_ctx.overlapped_connect()? {
            const EVENT_ID_ABORT: usize = 0;
            const EVENT_ID_CONNECT: usize = 1;
            let events = &[abort_event.raw(), connect_ctx.event().raw()];

            let wait_result = match wait_any_with_timeout(
                events,
                PIPE_CONNECT_TIMEOUT
            ) {
                Ok(idx) => idx,
                Err(WindowsError::WaitForMultipleObjectsTimeout) => {
                    warn!(%channel_name, %pipe_name, "DVC pipe proxy connection timed out");
                    return Ok(());
                }
                Err(err) => {
                    return Err(DvcPipeProxyError::Windows(err));
                }
            };

            if wait_result == EVENT_ID_ABORT {
                info!(%channel_name, %pipe_name, "DVC pipe proxy connection has been aborted");
                return Ok(());
            }

            connect_ctx.get_result()?;
        }
    }

    info!(%channel_name, %pipe_name, "DVC pipe proxy connected");

    let mut read_ctx = pipe.prepare_read_overlapped(MESSAGE_BUFFER_SIZE)?;

    const EVENT_ID_ABORT: usize = 0;
    const EVENT_ID_READ: usize = 1;
    const EVENT_ID_WRITE_MPSC: usize = 2;
    let events = &[abort_event.raw(), read_ctx.event().raw(), to_pipe_semaphore.raw()];

    read_ctx.overlapped_read()?;

    info!(%channel_name, %pipe_name, "DVC pipe proxy IO loop started");

    loop {
        let wait_result = wait_any(events)?;

        // abort event
        if wait_result == EVENT_ID_ABORT {
            info!(%channel_name, %pipe_name, "DVC pipe proxy connection has been aborted");
            return Ok(());
        }

        // read from pipe
        if wait_result == EVENT_ID_READ {
            let read_result = read_ctx.get_result()?.to_vec();

            trace!(%channel_name, %pipe_name, "DVC proxy read {} bytes from pipe", read_result.len());

            if read_result.len() != 0 {
                let messages = encode_dvc_messages(
                    channel_id,
                    vec![Box::new(GenericDvcMessage(read_result))],
                    ChannelFlags::empty()
                ).map_err(|_| {
                    DvcPipeProxyError::EncodeDvcMessage
                })?;

                if let Err(err) = dvc_write_callback(0, messages) {
                    error!(%err, %channel_name, %pipe_name, "DVC pipe proxy write callback failed");
                }
            }

            // Queue the read operation again
            read_ctx.overlapped_read()?;
            continue;
        }

        // read from mpsc and write to pipe
        if wait_result == EVENT_ID_WRITE_MPSC {
            let payload = to_pipe_rx.recv().map_err(|_| {
                DvcPipeProxyError::MpscIo
            })?;

            let payload_len = payload.len();

            if payload_len == 0 {
                warn!(%channel_name, %pipe_name, "Rejected empty DVC data (not sent to pipe)");
                continue;
            }

            trace!(%channel_name, %pipe_name, "DVC proxy write {} bytes to pipe,", payload_len);

            // write to pipe
            let mut overlapped_write = pipe.prepare_write_overlapped(payload)?;

            const EVENT_ID_WRITE_PIPE: usize = 1;
            let events = &[abort_event.raw(), overlapped_write.event().raw()];

            overlapped_write.overlapped_write()?;
            let wait_result = wait_any_with_timeout(events, PIPE_WRITE_TIMEOUT)?;

            // abort event
            if wait_result == EVENT_ID_ABORT {
                info!(%channel_name, %pipe_name, "DVC pipe proxy write aborted");
                return Ok(());
            }

            // write to pipe
            let bytes_written = overlapped_write.get_result()?;

            if bytes_written != payload_len as u32 {
                // Message-based pipe write failed
                return Err(DvcPipeProxyError::DvcIncompleteWrite);
            }

            continue;
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
        self
            .start_impl(channel_id)
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

pub type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send>;

use std::sync::mpsc;

use ironrdp_core::{ensure_size, Encode, EncodeResult};
use ironrdp_dvc::{encode_dvc_messages, DvcEncode};
use ironrdp_pdu::PduResult;
use ironrdp_svc::{ChannelFlags, SvcMessage};

use crate::platform::windows::error::DvcPipeProxyError;
use crate::windows::{wait_any, wait_any_with_timeout, Event, MessagePipeServer, Semaphore, WindowsError};

const PIPE_CONNECT_TIMEOUT: u32 = 10_000; // 10 seconds
const PIPE_WRITE_TIMEOUT: u32 = 3_000; // 3 seconds
const MESSAGE_BUFFER_SIZE: usize = 64 * 1024; // 64 KiB

pub(crate) type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send>;

pub(crate) struct WorkerCtx {
    pub pipe: MessagePipeServer,
    pub to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    pub to_pipe_semaphore: Semaphore,
    pub abort_event: Event,
    pub dvc_write_callback: OnWriteDvcMessage,
    pub pipe_name: String,
    pub channel_name: String,
    pub channel_id: u32,
}

pub(crate) fn worker_thread_func(worker_ctx: WorkerCtx) -> Result<(), DvcPipeProxyError> {
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
            let events = &[abort_event.raw(), connect_ctx.event().raw()];

            let wait_result = match wait_any_with_timeout(events, PIPE_CONNECT_TIMEOUT) {
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

            if !read_result.is_empty() {
                let messages = encode_dvc_messages(
                    channel_id,
                    vec![Box::new(RawDataDvcMessage(read_result))],
                    ChannelFlags::empty(),
                )
                .map_err(|_| DvcPipeProxyError::EncodeDvcMessage)?;

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
            let payload = to_pipe_rx.recv().map_err(|_| DvcPipeProxyError::MpscIo)?;

            let payload_len = payload.len();

            if payload_len == 0 {
                warn!(%channel_name, %pipe_name, "Rejected empty DVC data (not sent to pipe)");
                continue;
            }

            trace!(%channel_name, %pipe_name, "DVC proxy write {} bytes to pipe,", payload_len);

            // write to pipe
            let mut overlapped_write = pipe.prepare_write_overlapped(payload)?;

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

            if bytes_written as usize != payload_len {
                // Message-based pipe write failed
                return Err(DvcPipeProxyError::DvcIncompleteWrite);
            }

            continue;
        }
    }
}

struct RawDataDvcMessage(Vec<u8>);

impl Encode for RawDataDvcMessage {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RawDataDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDataDvcMessage {}

use std::sync::Arc;

use ironrdp_dvc::encode_dvc_messages;
use ironrdp_pdu::PduResult;
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tokio::sync::{mpsc, Notify};

use crate::error::DvcPipeProxyError;
use crate::message::RawDataDvcMessage;
use crate::os_pipe::OsPipe;

const IO_BUFFER_SIZE: usize = 1024 * 64; // 64K

pub(crate) type OnWriteDvcMessage = Box<dyn Fn(u32, Vec<SvcMessage>) -> PduResult<()> + Send>;

pub(crate) struct WorkerCtx {
    pub(crate) on_write_dvc: OnWriteDvcMessage,
    pub(crate) to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    pub(crate) abort_event: Arc<Notify>,
    pub(crate) pipe_name: String,
    pub(crate) channel_name: String,
    pub(crate) channel_id: u32,
}

pub(crate) fn run_worker<P: OsPipe>(ctx: WorkerCtx) {
    let _ = std::thread::spawn(move || {
        let channel_name = ctx.channel_name.clone();
        let pipe_name = ctx.pipe_name.clone();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(DvcPipeProxyError::Io);

        let runtime = match runtime {
            Ok(runtime) => runtime,
            Err(error) => {
                error!(
                    %channel_name,
                    %pipe_name,
                    ?error,
                    "DVC pipe proxy worker thread initialization failed."
                );
                return;
            }
        };

        if let Err(error) = runtime.block_on(worker::<P>(ctx)) {
            error!(
                %channel_name,
                %pipe_name,
                ?error,
                "DVC pipe proxy worker thread has failed."
            );
        }
    });
}

enum NextWorkerState {
    Abort,
    Reconnect,
}

async fn process_client<P: OsPipe>(ctx: &mut WorkerCtx) -> Result<NextWorkerState, DvcPipeProxyError> {
    let pipe_name = &ctx.pipe_name;
    let channel_name = &ctx.channel_name;

    /*
     match fs::metadata(&ctx.pipe_name).await
     {
         Ok(metadata) => {
             use std::os::unix::fs::FileTypeExt;

             info!(
                 %channel_name,
                 %pipe_name,
                 "DVC pipe already exists, removing stale file."
             );

             // Just to be sure, check if it's indeed a socket -
             // throw an error if calling code accidentally passed a regular file.
             if !metadata.file_type().is_socket() {
                 return Err(DvcPipeProxyError::Io(std::io::Error::new(
                     std::io::ErrorKind::InvalidInput,
                     format!("Path {} is not a socket", ctx.pipe_name),
                 )));
             }

             fs::remove_file(&ctx.pipe_name).await.map_err(DvcPipeProxyError::Io)?;
         }
         Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
             trace!(
                 %pipe_name,
                 %channel_name,
                 "DVC pipe does not exist, creating it."
             );
         }
         Err(e) => {
             return Err(DvcPipeProxyError::Io(e));
         }
     }

    let listener = tokio::net::UnixListener::bind(&ctx.pipe_name)
         .map_err(DvcPipeProxyError::Io)?;

     info!(%pipe_name, %pipe_name, "Waiting for DVC pipe connection...");

     let mut pipe = tokio::select! {
         stream = listener.accept() => {
             let (pipe, _) = stream.map_err(DvcPipeProxyError::Io)?;
             info!(%channel_name, %pipe_name,"DVC proxy worker thread has started.");
             pipe
         }
         _ = ctx.abort_event.notified() => {
             info!(%channel_name, %pipe_name, "DVC proxy worker thread has been aborted.");
             return Ok(NextWorkerState::Abort);
         }
     };
     */

    let mut pipe = tokio::select! {
        pipe = P::connect(pipe_name) => {
            info!(%channel_name, %pipe_name,"DVC proxy worker thread has started.");
            pipe?
        }
        _ = ctx.abort_event.notified() => {
            info!(%channel_name, %pipe_name, "DVC proxy worker thread has been aborted.");
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
                info!(%channel_name, %pipe_name, "Received abort signal for DVC proxy worker thread.");
                return Ok(NextWorkerState::Abort);
            }
            read_bytes_result = read_pipe => {
                let read_bytes = read_bytes_result?;

                if read_bytes == 0 {
                    info!(%channel_name, %pipe_name, "DVC proxy pipe returned EOF");

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
                        info!(%channel_name, %pipe_name, "DVC mpsc channel returned EOF.");
                        // Server DVC has been closed, there is no point in
                        // trying to reconnect.
                        return Ok(NextWorkerState::Abort);
                    }
                };

                if let Err(error) = pipe.write_all(&data).await
                {
                    error!(%channel_name, %pipe_name, ?error, "Failed to write to DVC pipe");
                    continue;
                }
            }
        };
    }
}

async fn worker<P: OsPipe>(mut ctx: WorkerCtx) -> Result<(), DvcPipeProxyError> {
    loop {
        match process_client::<P>(&mut ctx).await? {
            NextWorkerState::Abort => {
                info!(
                    channel_name = %ctx.channel_name,
                    pipe_name = %ctx.pipe_name,
                    "Aborting DVC proxy worker thread."
                );
                break;
            }
            NextWorkerState::Reconnect => {
                info!(
                    channel_name = %ctx.channel_name,
                    pipe_name = %ctx.pipe_name,
                    "Reconnecting to DVC pipe..."
                );
                continue;
            }
        };
    }

    Ok(())
}

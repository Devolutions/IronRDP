use async_trait::async_trait;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe;
use tracing::debug;

use crate::error::DvcPipeProxyError;
use crate::os_pipe::OsPipe;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
// ConnectNamedPipe reports this when the client wins the create/accept race.
const ERROR_PIPE_CONNECTED: i32 = 535;

/// Windows-specific implementation of the OS pipe trait.
pub(crate) struct WindowsPipe {
    pipe_server: named_pipe::NamedPipeServer,
}

#[async_trait]
impl OsPipe for WindowsPipe {
    async fn connect(pipe_name: &str) -> Result<Self, DvcPipeProxyError> {
        let pipe_path = format!("\\\\.\\pipe\\{pipe_name}");
        debug!(%pipe_name, %pipe_path, "Creating DVC proxy Windows named pipe");

        let pipe_server = named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .access_inbound(true)
            .access_outbound(true)
            .max_instances(2)
            .in_buffer_size(PIPE_BUFFER_SIZE)
            .out_buffer_size(PIPE_BUFFER_SIZE)
            .pipe_mode(named_pipe::PipeMode::Byte)
            .create(&pipe_path)
            .map_err(|error| {
                debug!(%pipe_name, %pipe_path, %error, "Failed to create DVC proxy Windows named pipe");
                DvcPipeProxyError::Io(error)
            })?;

        debug!(%pipe_name, %pipe_path, "Waiting for DVC proxy Windows named-pipe client");
        match pipe_server.connect().await {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_CONNECTED) => {
                debug!(
                    %pipe_name,
                    %pipe_path,
                    "DVC proxy Windows named-pipe client connected before accept"
                );
            }
            Err(error) => {
                debug!(%pipe_name, %pipe_path, %error, "Failed to accept DVC proxy Windows named-pipe client");
                return Err(DvcPipeProxyError::Io(error));
            }
        }
        debug!(%pipe_name, %pipe_path, "Connected DVC proxy Windows named-pipe client");

        Ok(Self { pipe_server })
    }

    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DvcPipeProxyError> {
        self.pipe_server.read(buffer).await.map_err(DvcPipeProxyError::Io)
    }

    async fn write_all(&mut self, buffer: &[u8]) -> Result<(), DvcPipeProxyError> {
        self.pipe_server
            .write_all(buffer)
            .await
            .map_err(DvcPipeProxyError::Io)
            .map(|_| ())
    }
}

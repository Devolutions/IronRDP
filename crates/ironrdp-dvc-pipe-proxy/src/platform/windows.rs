use async_trait::async_trait;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe;

use crate::error::DvcPipeProxyError;
use crate::os_pipe::OsPipe;

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;

/// Unix-specific implementation of the OS pipe trait.
pub(crate) struct WindowsPipe {
    pipe_server: named_pipe::NamedPipeServer,
}

#[async_trait]
impl OsPipe for WindowsPipe {
    async fn connect(pipe_name: &str) -> Result<Self, DvcPipeProxyError> {
        let pipe_name = format!("\\\\.\\pipe\\{pipe_name}");

        let pipe_server = named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .access_inbound(true)
            .access_outbound(true)
            .max_instances(2)
            .in_buffer_size(PIPE_BUFFER_SIZE)
            .out_buffer_size(PIPE_BUFFER_SIZE)
            .pipe_mode(named_pipe::PipeMode::Message)
            .create(pipe_name)
            .map_err(DvcPipeProxyError::Io)?;

        pipe_server.connect().await.map_err(DvcPipeProxyError::Io)?;

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

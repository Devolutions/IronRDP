use async_trait::async_trait;
use tokio::fs;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::error::DvcPipeProxyError;
use crate::os_pipe::OsPipe;

/// Unix-specific implementation of the OS pipe trait.
pub(crate) struct UnixPipe {
    socket: tokio::net::UnixStream,
}

#[async_trait]
impl OsPipe for UnixPipe {
    async fn connect(pipe_name: &str) -> Result<Self, DvcPipeProxyError> {
        // Domain socket file could already exist from a previous run.
        match fs::metadata(&pipe_name).await {
            Ok(metadata) => {
                use std::os::unix::fs::FileTypeExt as _;

                info!(
                    %pipe_name,
                    "DVC pipe already exists, removing stale file."
                );

                // Just to be sure, check if it's indeed a socket -
                // throw an error if calling code accidentally passed a regular file.
                if !metadata.file_type().is_socket() {
                    return Err(DvcPipeProxyError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("Path {pipe_name} is not a socket"),
                    )));
                }

                fs::remove_file(pipe_name).await.map_err(DvcPipeProxyError::Io)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                trace!(
                    %pipe_name,
                    "DVC pipe does not exist, creating it."
                );
            }
            Err(e) => {
                return Err(DvcPipeProxyError::Io(e));
            }
        }

        let listener = tokio::net::UnixListener::bind(pipe_name).map_err(DvcPipeProxyError::Io)?;

        let (socket, _) = listener.accept().await.map_err(DvcPipeProxyError::Io)?;

        Ok(Self { socket })
    }

    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DvcPipeProxyError> {
        self.socket.read(buffer).await.map_err(DvcPipeProxyError::Io)
    }

    async fn write_all(&mut self, buffer: &[u8]) -> Result<(), DvcPipeProxyError> {
        self.socket
            .write_all(buffer)
            .await
            .map_err(DvcPipeProxyError::Io)
            .map(|_| ())
    }
}

use async_trait::async_trait;

use crate::error::DvcPipeProxyError;

#[async_trait]
pub(crate) trait OsPipe: Send + Sync {
    /// Creates a new OS pipe and waits for the connection.
    async fn connect(pipe_name: &str) -> Result<Self, DvcPipeProxyError>
    where
        Self: Sized;

    /// Reads data from the pipe and returns the number of bytes read.
    ///
    /// Returned future should be stateless and can be polled multiple times.
    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, DvcPipeProxyError>;

    /// Writes data to the pipe and returns the number of bytes written.
    async fn write_all(&mut self, buffer: &[u8]) -> Result<(), DvcPipeProxyError>;
}

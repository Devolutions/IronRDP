#[derive(Debug)]
pub(crate) enum WindowsError {
    CreateNamedPipe(windows::core::Error),
    CreateEvent(windows::core::Error),
    SetEvent(windows::core::Error),
    ReleaseSemaphore(windows::core::Error),
    InvalidSemaphoreParams(&'static str),
    WaitForMultipleObjectsFailed(windows::core::Error),
    WaitForMultipleObjectsTimeout,
    WaitForMultipleObjectsAbandoned(u32),
    OverlappedConnect(windows::core::Error),
    OverlappedRead(windows::core::Error),
    OverlappedWrite(windows::core::Error),
    CreateSemaphore(windows::core::Error),
    InvalidPipeName(String),
}

impl core::fmt::Display for WindowsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WindowsError::CreateNamedPipe(_) => write!(f, "failed to create named pipe"),
            WindowsError::CreateEvent(_) => write!(f, "failed to create event object"),
            WindowsError::SetEvent(_) => write!(f, "failed to set event to signaled state"),
            WindowsError::InvalidSemaphoreParams(cause) => write!(f, "invalid semaphore parameters: {cause}"),
            WindowsError::ReleaseSemaphore(_) => write!(f, "failed to release semaphore"),
            WindowsError::WaitForMultipleObjectsFailed(_) => write!(f, "failed to wait for multiple objects"),
            WindowsError::WaitForMultipleObjectsTimeout => write!(f, "timed out waiting for multiple objects"),
            WindowsError::WaitForMultipleObjectsAbandoned(idx) => {
                write!(f, "wait for multiple objects failed, handle #{idx} was abandoned")
            }
            WindowsError::OverlappedConnect(_) => write!(f, "overlapped connect failed"),
            WindowsError::OverlappedRead(_) => write!(f, "overlapped read failed"),
            WindowsError::OverlappedWrite(_) => write!(f, "overlapped write failed"),
            WindowsError::CreateSemaphore(_) => write!(f, "failed to create semaphore object"),
            WindowsError::InvalidPipeName(cause) => write!(f, "invalid pipe name: `{cause}`"),
        }
    }
}

impl core::error::Error for WindowsError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            WindowsError::CreateNamedPipe(err)
            | WindowsError::SetEvent(err)
            | WindowsError::ReleaseSemaphore(err)
            | WindowsError::WaitForMultipleObjectsFailed(err)
            | WindowsError::OverlappedConnect(err)
            | WindowsError::OverlappedRead(err)
            | WindowsError::OverlappedWrite(err)
            | WindowsError::CreateSemaphore(err) => Some(err),
            WindowsError::CreateEvent(err) => Some(err),
            WindowsError::InvalidSemaphoreParams(_)
            | WindowsError::WaitForMultipleObjectsTimeout
            | WindowsError::InvalidPipeName(_) => None,
            WindowsError::WaitForMultipleObjectsAbandoned(_) => None,
        }
    }
}

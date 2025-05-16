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
}

impl core::fmt::Display for WindowsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WindowsError::CreateNamedPipe(_) => write!(f, "CreateNamedPipe failed"),
            WindowsError::CreateEvent(_) => write!(f, "CreateEvent failed"),
            WindowsError::SetEvent(_) => write!(f, "SetEvent failed"),
            WindowsError::InvalidSemaphoreParams(cause) => {
                write!(f, "Invalid semaphore parameters: {}", cause)
            }
            WindowsError::ReleaseSemaphore(_) => {
                write!(f, "ReleaseSemaphore failed")
            }
            WindowsError::WaitForMultipleObjectsFailed(_) => {
                write!(f, "WaitForMultipleObjects failed")
            }
            WindowsError::WaitForMultipleObjectsTimeout => {
                write!(f, "WaitForMultipleObjects timed out")
            }
            WindowsError::WaitForMultipleObjectsAbandoned(idx) => {
                write!(f, "WaitForMultipleObjects handle #{idx} was abandoned")
            }
            WindowsError::OverlappedConnect(_) => {
                write!(f, "Overlapped connect failed")
            }
            WindowsError::OverlappedRead(_) => {
                write!(f, "Overlapped read failed")
            }
            WindowsError::OverlappedWrite(_) => {
                write!(f, "Overlapped write failed")
            }
            WindowsError::CreateSemaphore(_) => {
                write!(f, "CreateSemaphore failed")
            }
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
            | WindowsError::WaitForMultipleObjectsAbandoned(_) => None,
        }
    }
}

use crate::windows::WindowsError;

#[derive(Debug)]
pub(crate) enum DvcPipeProxyError {
    Windows(WindowsError),
    MpscIo,
    DvcIncompleteWrite,
    EncodeDvcMessage,
}

impl From<WindowsError> for DvcPipeProxyError {
    fn from(err: WindowsError) -> Self {
        DvcPipeProxyError::Windows(err)
    }
}

impl core::fmt::Display for DvcPipeProxyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DvcPipeProxyError::Windows(err) => err.fmt(f),
            DvcPipeProxyError::MpscIo => write!(f, "MPSC IO error"),
            DvcPipeProxyError::DvcIncompleteWrite => write!(f, "DVC incomplete write"),
            DvcPipeProxyError::EncodeDvcMessage => write!(f, "DVC message encoding error"),
        }
    }
}

impl core::error::Error for DvcPipeProxyError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            DvcPipeProxyError::Windows(err) => Some(err),
            DvcPipeProxyError::MpscIo => None,
            DvcPipeProxyError::DvcIncompleteWrite => None,
            DvcPipeProxyError::EncodeDvcMessage => None,
        }
    }
}

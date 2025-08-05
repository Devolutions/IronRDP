#[derive(Debug)]
pub(crate) enum DvcPipeProxyError {
    Io(std::io::Error),
    EncodeDvcMessage(ironrdp_core::EncodeError),
}

impl core::fmt::Display for DvcPipeProxyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DvcPipeProxyError::Io(_) => write!(f, "IO error"),
            DvcPipeProxyError::EncodeDvcMessage(_) => write!(f, "DVC message encoding error"),
        }
    }
}

impl core::error::Error for DvcPipeProxyError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            DvcPipeProxyError::Io(err) => Some(err),
            DvcPipeProxyError::EncodeDvcMessage(src) => Some(src),
        }
    }
}

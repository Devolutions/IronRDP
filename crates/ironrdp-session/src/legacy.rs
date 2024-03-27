use crate::SessionError;

// FIXME: code should be fixed so that we never need this conversion
// For that, some code from this ironrdp_session::legacy and ironrdp_connector::legacy modules should be moved to ironrdp_pdu itself
impl From<ironrdp_connector::ConnectorErrorKind> for crate::SessionErrorKind {
    fn from(value: ironrdp_connector::ConnectorErrorKind) -> Self {
        match value {
            ironrdp_connector::ConnectorErrorKind::Pdu(e) => crate::SessionErrorKind::Pdu(e),
            ironrdp_connector::ConnectorErrorKind::Credssp(_) => panic!("unexpected"),
            ironrdp_connector::ConnectorErrorKind::AccessDenied => panic!("unexpected"),
            ironrdp_connector::ConnectorErrorKind::General => crate::SessionErrorKind::General,
            ironrdp_connector::ConnectorErrorKind::Custom => crate::SessionErrorKind::Custom,
            _ => crate::SessionErrorKind::General,
        }
    }
}

pub(crate) fn map_error(error: ironrdp_connector::ConnectorError) -> SessionError {
    error.into_other_kind()
}

impl ironrdp_error::legacy::CatchAllKind for crate::SessionErrorKind {
    const CATCH_ALL_VALUE: Self = crate::SessionErrorKind::General;
}

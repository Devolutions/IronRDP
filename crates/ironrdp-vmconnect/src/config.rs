use ironrdp_connector::general_err;
use ironrdp_pdu::nego::NegoRequestData;
use sspi::Username;

#[derive(Debug, Clone)]
pub struct Credentials {
    pub(crate) username: String,
    pub(crate) password: String,
}

impl Credentials {
    pub(crate) fn to_sspi_auth_identity(&self, domain: Option<&str>) -> Result<sspi::AuthIdentity, sspi::Error> {
        Ok(sspi::AuthIdentity {
            username: Username::new(&self.username, domain)?,
            password: self.password.clone().into(),
        })
    }
}

impl TryFrom<&ironrdp_connector::Credentials> for Credentials {
    type Error = ironrdp_connector::ConnectorError;

    fn try_from(value: &ironrdp_connector::Credentials) -> Result<Self, Self::Error> {
        let ironrdp_connector::Credentials::UsernamePassword { username, password } = value else {
            return Err(general_err!("Invalid credentials type for VM connection",));
        };

        Ok(Credentials {
            username: username.to_owned(),
            password: password.to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct VmConnectorConfig {
    pub request_data: Option<NegoRequestData>,
    pub credentials: Credentials,
}

impl TryFrom<&ironrdp_connector::Config> for VmConnectorConfig {
    type Error = ironrdp_connector::ConnectorError;

    fn try_from(value: &ironrdp_connector::Config) -> Result<Self, Self::Error> {
        let request_data = value.request_data.clone();
        let ironrdp_connector::Credentials::UsernamePassword { username, password } = &value.credentials else {
            return Err(general_err!("Invalid credentials type for VM connection",));
        };

        let credentials = Credentials {
            username: username.to_owned(),
            password: password.to_owned(),
        };

        Ok(VmConnectorConfig {
            request_data,
            credentials,
        })
    }
}

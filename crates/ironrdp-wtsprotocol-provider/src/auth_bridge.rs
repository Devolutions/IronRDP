use ironrdp_acceptor::credssp::CredsspSequence;
use ironrdp_connector::sspi;
use ironrdp_pdu::nego;
use windows::Win32::Foundation::E_ACCESSDENIED;

#[derive(Debug, Clone, Copy)]
pub struct CredsspPolicy {
    pub require_hybrid_security: bool,
}

impl Default for CredsspPolicy {
    fn default() -> Self {
        Self {
            require_hybrid_security: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct CredsspServerBridge {
    kerberos_config: Option<sspi::KerberosServerConfig>,
}

fn credssp_sequence_type_marker() -> core::any::TypeId {
    core::any::TypeId::of::<CredsspSequence<'static>>()
}

impl CredsspServerBridge {
    #[must_use]
    pub fn with_kerberos_config(mut self, kerberos_config: sspi::KerberosServerConfig) -> Self {
        self.kerberos_config = Some(kerberos_config);
        self
    }

    pub fn kerberos_config(&self) -> Option<&sspi::KerberosServerConfig> {
        self.kerberos_config.as_ref()
    }

    pub fn validate_security_protocol(
        &self,
        policy: CredsspPolicy,
        security_protocol: nego::SecurityProtocol,
    ) -> windows_core::Result<()> {
        let _ = credssp_sequence_type_marker();

        if policy.require_hybrid_security
            && !security_protocol.intersects(nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX)
        {
            return Err(windows_core::Error::new(
                E_ACCESSDENIED,
                "client does not support required CredSSP security protocols",
            ));
        }

        Ok(())
    }
}

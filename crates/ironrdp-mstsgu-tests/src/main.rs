use anyhow::{Context as _, bail};
use core::time::Duration;
use ironrdp_mstsgu::GwConnectTarget;
use ironrdp_mstsgu::test_support::GatewayTransport;
use ironrdp_tls::CertificateValidation;

mod mock_rpch;
mod rpch;
mod smartcard;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [scenario, endpoint] if scenario == "proxy-connect" => {
            let target = GwConnectTarget {
                gw_endpoint: endpoint.clone(),
                gw_user: "user".to_owned(),
                gw_pass: "pass".to_owned(),
                smart_card: None,
                server: "server.test".to_owned(),
            };
            let mut transport = GatewayTransport::connect_tls(
                &target,
                CertificateValidation::DangerouslyAcceptInvalidCertificate,
                None,
            )
            .await
            .context("connect through the configured proxy")?;
            transport.close().await.map_err(anyhow::Error::msg)?;
            Ok(())
        }
        #[cfg(feature = "smartcard")]
        [scenario] if scenario == "smartcard-auth" => smartcard::check_authentication_errors(),
        [scenario] if scenario == "smartcard-disabled" => smartcard::check_disabled_feature(),
        [scenario] if scenario == "rpch-gateway" => rpch::check_gateway().await,
        [scenario] if scenario == "rpch-malformed" => rpch::check_malformed_replies().await,
        [scenario] if scenario == "rpch-in-recycle" => {
            tokio::time::timeout(Duration::from_secs(600), rpch::check_in_recycle())
                .await
                .context("IN rollover transfer exceeded ten minutes")?
        }
        _ => bail!("unknown Mstsgu check or arguments"),
    }
}

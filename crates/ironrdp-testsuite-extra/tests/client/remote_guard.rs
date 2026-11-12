#[cfg(feature = "remote-guard")]
use core::future::Future as _;
#[cfg(feature = "remote-guard")]
use core::task::{Context, Poll, Waker};

use ironrdp_cfg::PropertySetExt as _;
#[cfg(feature = "remote-guard")]
use ironrdp_client::__test_apply_server_redirection;
use ironrdp_client::__test_build_connector;
#[cfg(feature = "remote-guard")]
use ironrdp_client::config::TransportKind;
use ironrdp_client::config::{Config, ConfigBuilder, Destination, RemoteGuardMode};
#[cfg(feature = "remote-guard")]
use ironrdp_client::output_channel::output_channel;
#[cfg(feature = "remote-guard")]
use ironrdp_client::rdp::{RdpClient, RdpOutputEvent};
#[cfg(feature = "remote-guard")]
use ironrdp_connector::ClientConnectorState;
use ironrdp_connector::Sequence as _;
use ironrdp_core::{WriteBuf, decode};
use ironrdp_dvc::DrdynvcClient;
#[cfg(feature = "remote-guard")]
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::{ConnectionRequest, RequestFlags, SecurityProtocol};
#[cfg(feature = "remote-guard")]
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
#[cfg(feature = "remote-guard")]
use ironrdp_pdu::rdp::server_redirection::ServerRedirectionPdu;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_pdu::x224::X224;
use ironrdp_propertyset::PropertySet;
use ironrdp_svc::SvcProcessor as _;

fn complete_builder() -> ConfigBuilder {
    ConfigBuilder::new()
        .with_destination(Destination::from_parts("remote-guard.invalid", 3389))
        .with_username("remote_guard_user")
        .with_password(String::new())
        .with_client_build(1)
        .with_client_dir(r"C:\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
}

fn remote_guard_test_config(mode: RemoteGuardMode) -> Config {
    complete_builder()
        .with_remote_guard_mode(mode)
        .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
        .build()
        .expect("supported remote guard test configuration")
}

fn remote_guard_assert_negotiation_and_carrier(connector: &mut ironrdp_connector::ClientConnector, enabled: bool) {
    assert_eq!(connector.remote_guard_enabled(), enabled);
    assert_eq!(connector.__test_static_channel_count(DrdynvcClient::NAME), 1);
    let carrier = connector
        .get_static_channel_processor::<DrdynvcClient>()
        .expect("DRDYNVC processor");
    assert_eq!(carrier.max_message_size(), enabled.then_some(1_048_576));
    for channel_name in [
        ironrdp::displaycontrol::CHANNEL_NAME,
        ironrdp::echo::CHANNEL_NAME,
        "Microsoft::Windows::RDS::Input",
        ironrdp_egfx::CHANNEL_NAME,
    ] {
        assert!(carrier.has_channel_name(channel_name), "{channel_name}");
    }

    let mut buffer = WriteBuf::default();
    connector
        .step_no_input(&mut buffer)
        .expect("encode only the initial request");
    let request = decode::<X224<ConnectionRequest>>(buffer.filled()).expect("decode initial negotiation");
    assert_eq!(
        request.0.flags,
        if enabled {
            RequestFlags::REDIRECTED_AUTHENTICATION_MODE_REQUIRED
        } else {
            RequestFlags::empty()
        }
    );
    assert_eq!(
        request.0.protocol,
        if enabled {
            SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX
        } else {
            SecurityProtocol::SSL | SecurityProtocol::HYBRID | SecurityProtocol::HYBRID_EX
        }
    );
}

#[test]
fn remote_guard_default_and_enabled_modes() {
    assert_eq!(RemoteGuardMode::default(), RemoteGuardMode::Disabled);
    for (mode, enabled) in [
        (RemoteGuardMode::Disabled, false),
        (RemoteGuardMode::Negotiate, true),
        (RemoteGuardMode::Ntlm, true),
        (RemoteGuardMode::Kerberos, true),
    ] {
        assert_eq!(mode.is_enabled(), enabled);
    }
}

#[test]
fn remote_guard_property_round_trip_and_clear() {
    let mut properties = PropertySet::new();
    properties.insert("remote_guard_sentinel", "retained");
    assert_eq!(properties.remote_guard_mode(), Ok(None));
    properties.insert("ironrdp_remote_guard", "kerberos");
    for (mode, value) in [
        (RemoteGuardMode::Disabled, 0i64),
        (RemoteGuardMode::Negotiate, 1),
        (RemoteGuardMode::Ntlm, 2),
        (RemoteGuardMode::Kerberos, 3),
    ] {
        properties.set_remote_guard_mode(mode);
        assert_eq!(properties.remote_guard_mode(), Ok(Some(mode)));
        assert_eq!(properties.get::<i64>("ironrdp_remote_guard"), Some(value));
        assert_eq!(properties.get::<&str>("ironrdp_remote_guard"), None);
        assert_eq!(properties.get::<&str>("remote_guard_sentinel"), Some("retained"));
    }
    for (name, mode) in [
        ("disabled", RemoteGuardMode::Disabled),
        ("Negotiate", RemoteGuardMode::Negotiate),
        ("NTLM", RemoteGuardMode::Ntlm),
        ("kerberos", RemoteGuardMode::Kerberos),
    ] {
        properties.insert("ironrdp_remote_guard", name);
        assert_eq!(properties.remote_guard_mode(), Ok(Some(mode)));
        assert_eq!(properties.get::<i64>("ironrdp_remote_guard"), None);
    }
    for _ in 0..2 {
        properties.clear_remote_guard_mode();
        assert_eq!(properties.remote_guard_mode(), Ok(None));
        assert_eq!(properties.get::<i64>("ironrdp_remote_guard"), None);
        assert_eq!(properties.get::<&str>("ironrdp_remote_guard"), None);
        assert_eq!(properties.get::<&str>("remote_guard_sentinel"), Some("retained"));
    }
}

#[test]
fn remote_guard_property_rejects_invalid_values() {
    let mut properties = PropertySet::new();
    for value in [i64::MIN, -1, 4, i64::MAX] {
        properties.insert("ironrdp_remote_guard", value);
        assert_eq!(properties.remote_guard_mode(), Err(ironrdp_cfg::InvalidRemoteGuardMode));
    }
    for value in ["", "0", "1", "kerberos-ntlm", "krb5"] {
        properties.insert("ironrdp_remote_guard", value);
        assert_eq!(properties.remote_guard_mode(), Err(ironrdp_cfg::InvalidRemoteGuardMode));
    }
}

#[test]
fn remote_guard_builder_mirrors_mode_property() {
    for (mode, value) in [
        (RemoteGuardMode::Disabled, 0i64),
        (RemoteGuardMode::Negotiate, 1),
        (RemoteGuardMode::Ntlm, 2),
        (RemoteGuardMode::Kerberos, 3),
    ] {
        let builder = complete_builder().with_remote_guard_mode(mode);
        let (builder_mode, builder_properties) = builder.__test_remote_guard_mode_and_properties();
        assert_eq!(builder_mode, mode);
        assert_eq!(builder_properties.get::<i64>("ironrdp_remote_guard"), Some(value));
        assert_eq!(builder_properties.remote_guard_mode(), Ok(Some(mode)));
    }
}

#[test]
fn remote_guard_property_overlay_last_writer_wins() {
    use RemoteGuardMode::{Disabled, Kerberos, Negotiate};

    for (initial, overlay, final_setter, expected, value) in [
        (Kerberos, Some(Negotiate), None, Negotiate, 1i64),
        (Disabled, Some(Negotiate), Some(Kerberos), Kerberos, 3),
        (Kerberos, Some(Disabled), None, Disabled, 0),
        (Kerberos, None, None, Kerberos, 3),
    ] {
        let mut properties = PropertySet::new();
        properties.insert("remote_guard_sentinel", "retained");
        if let Some(mode) = overlay {
            properties.set_remote_guard_mode(mode);
        }
        let mut builder = complete_builder()
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .with_remote_guard_mode(initial)
            .with_property_set(&properties)
            .expect("valid policy overlay");
        if let Some(mode) = final_setter {
            builder = builder.with_remote_guard_mode(mode);
        }
        let (builder_mode, builder_properties) = builder.__test_remote_guard_mode_and_properties();
        assert_eq!(builder_mode, expected);
        assert_eq!(builder_properties.remote_guard_mode(), Ok(Some(expected)));
        assert_eq!(builder_properties.get::<i64>("ironrdp_remote_guard"), Some(value));
        assert_eq!(
            builder_properties.get::<&str>("remote_guard_sentinel"),
            Some("retained")
        );
        if !expected.is_enabled() || cfg!(feature = "remote-guard") {
            let config = builder.build().expect("supported policy");
            assert_eq!(config.remote_guard_mode(), expected);
            assert_eq!(config.properties().get::<i64>("ironrdp_remote_guard"), Some(value));
            assert_eq!(
                config.properties().get::<&str>("remote_guard_sentinel"),
                Some("retained")
            );
        }
    }

    for (mode, value) in [(Disabled, 0i64), (Negotiate, 1), (Kerberos, 3)] {
        let seed = complete_builder().with_remote_guard_mode(mode);
        let (_, seed_properties) = seed.__test_remote_guard_mode_and_properties();
        let builder = ConfigBuilder::from_property_set(seed_properties)
            .expect("valid policy properties")
            .with_password(String::new())
            .with_client_build(1)
            .with_client_dir(r"C:\")
            .with_platform(MajorPlatformType::WINDOWS)
            .with_client_name("client")
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict);
        let (builder_mode, builder_properties) = builder.__test_remote_guard_mode_and_properties();
        assert_eq!(builder_mode, mode);
        assert_eq!(builder_properties.remote_guard_mode(), Ok(Some(mode)));
        assert_eq!(builder_properties.get::<i64>("ironrdp_remote_guard"), Some(value));
        if !mode.is_enabled() || cfg!(feature = "remote-guard") {
            let config = builder.build().expect("supported property-origin policy");
            assert_eq!(config.remote_guard_mode(), mode);
            assert_eq!(config.properties().get::<i64>("ironrdp_remote_guard"), Some(value));
        }
    }
}

#[test]
fn remote_guard_property_overlay_rejects_invalid_mode() {
    for value in [
        ironrdp_propertyset::Value::Int(4),
        ironrdp_propertyset::Value::Str("kerberos-ntlm".to_owned()),
    ] {
        let mut properties = PropertySet::new();
        properties.insert("ironrdp_remote_guard", value);
        let error = complete_builder()
            .with_property_set(&properties)
            .err()
            .expect("invalid policy must reject the overlay");
        assert_eq!(
            error.downcast_ref::<ironrdp_cfg::InvalidRemoteGuardMode>(),
            Some(&ironrdp_cfg::InvalidRemoteGuardMode)
        );
    }
}

#[test]
fn remote_guard_disabled_baseline_is_unchanged() {
    for explicit in [false, true] {
        let mut builder = complete_builder();
        if explicit {
            builder = builder.with_remote_guard_mode(RemoteGuardMode::Disabled);
        }
        let config = builder.build().expect("ordinary configuration");
        assert_eq!(config.remote_guard_mode(), RemoteGuardMode::Disabled);
        assert!(config.connector().enable_tls);
        assert!(config.connector().enable_credssp);
        assert!(!config.connector().enable_standard_rdp_security);
        assert!(config.kerberos_config().is_none());
        assert_eq!(
            config.certificate_validation(),
            ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate
        );
        assert_eq!(
            config.properties().get::<i64>("ironrdp_remote_guard"),
            explicit.then_some(0)
        );
        assert_eq!(
            config.properties().remote_guard_mode(),
            Ok(explicit.then_some(RemoteGuardMode::Disabled))
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_build_stores_enabled_modes() {
    for (mode, value) in [
        (RemoteGuardMode::Negotiate, 1i64),
        (RemoteGuardMode::Ntlm, 2),
        (RemoteGuardMode::Kerberos, 3),
    ] {
        for from_properties in [false, true] {
            let builder = complete_builder().with_certificate_validation(ironrdp_tls::CertificateValidation::Strict);
            let builder = if from_properties {
                let mut properties = PropertySet::new();
                properties.set_remote_guard_mode(mode);
                builder.with_property_set(&properties).expect("valid mode")
            } else {
                builder.with_remote_guard_mode(mode)
            };
            let config = builder.build().expect("valid enabled policy");
            assert_eq!(config.remote_guard_mode(), mode);
            assert_eq!(config.properties().remote_guard_mode(), Ok(Some(mode)));
            assert_eq!(config.properties().get::<i64>("ironrdp_remote_guard"), Some(value));
            assert!(config.connector().enable_tls);
            assert!(config.connector().enable_credssp);
            assert!(!config.connector().enable_standard_rdp_security);
            assert_eq!(
                config.certificate_validation(),
                ironrdp_tls::CertificateValidation::Strict
            );
        }
    }
}

#[cfg(not(feature = "remote-guard"))]
#[test]
fn remote_guard_feature_off_rejects_enabled_modes() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for from_properties in [false, true] {
            let builder = complete_builder().with_certificate_validation(ironrdp_tls::CertificateValidation::Strict);
            let builder = if from_properties {
                let mut properties = PropertySet::new();
                properties.set_remote_guard_mode(mode);
                builder.with_property_set(&properties).expect("valid mode")
            } else {
                builder.with_remote_guard_mode(mode)
            };
            assert_eq!(
                builder.build().expect_err("feature is required").to_string(),
                "remote credential guard requires the experimental remote-guard build feature"
            );
        }
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_requires_tls_and_credssp() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for (tls, credssp) in [(false, true), (true, false), (false, false)] {
            let error = complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_tls(tls)
                .with_credssp(credssp)
                .build()
                .expect_err("enhanced security is required");
            assert_eq!(error.to_string(), "remote credential guard requires TLS and CredSSP");
        }
    }
}

#[cfg(all(windows, feature = "remote-guard"))]
#[test]
fn remote_guard_rejects_standard_rdp_security() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let error = complete_builder()
            .with_remote_guard_mode(mode)
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .with_tls(true)
            .with_credssp(true)
            .with_transport(TransportKind::NamedPipe {
                path: r"\\.\pipe\remote_guard_inert".to_owned(),
            })
            .build()
            .expect_err("standard security cannot carry remote guard");
        assert_eq!(error.to_string(), "remote credential guard requires TLS and CredSSP");
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_rejects_rdcleanpath() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let error = complete_builder()
            .with_remote_guard_mode(mode)
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .with_transport(TransportKind::RDCleanPath {
                url: url::Url::parse("wss://proxy.example.test/rdcleanpath").expect("valid URL"),
            })
            .with_rdcleanpath_token("test-token")
            .build()
            .expect_err("RDCleanPath does not validate target server TLS");
        assert_eq!(
            error.to_string(),
            "remote credential guard requires a validated TLS connection to the RDP server and is not supported with RDCleanPath"
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_requires_strict_certificate_policy() {
    use ironrdp_tls::CertificateValidation::{DangerouslyAcceptInvalidCertificate, Strict};

    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for policy in [None, Some(DangerouslyAcceptInvalidCertificate)] {
            let mut builder = complete_builder().with_remote_guard_mode(mode);
            if let Some(policy) = policy {
                builder = builder.with_certificate_validation(policy);
            }
            assert_eq!(
                builder.build().expect_err("strict policy is required").to_string(),
                "remote credential guard requires strict TLS certificate validation"
            );
        }
        let callback: ironrdp_tls::CertificateValidationCallback = std::sync::Arc::new(|_, _, _| false);
        let config = complete_builder()
            .with_remote_guard_mode(mode)
            .with_certificate_validation_callback(std::sync::Arc::clone(&callback))
            .build()
            .expect("callback implies strict validation");
        assert_eq!(config.certificate_validation(), Strict);
        assert!(std::sync::Arc::ptr_eq(
            config.certificate_validation_callback().expect("callback retained"),
            &callback
        ));
        assert_eq!(
            complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(DangerouslyAcceptInvalidCertificate)
                .with_certificate_validation_callback(callback)
                .build()
                .expect_err("contradictory policy must fail")
                .to_string(),
            "cannot combine a dangerous certificate policy with a validation callback"
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_requires_nonempty_client_name() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        assert_eq!(
            complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_client_name("")
                .build()
                .expect_err("present but empty name must fail")
                .to_string(),
            "remote credential guard requires a client computer name"
        );
        let config = complete_builder()
            .with_remote_guard_mode(mode)
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .with_client_name("x")
            .build()
            .expect("one-character name is sufficient");
        assert_eq!(config.connector().client_name, "x");
        assert_eq!(
            config.kerberos_config().expect("synthesized Kerberos config").hostname,
            "x"
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_auto_creates_kerberos_config() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let config = complete_builder()
            .with_remote_guard_mode(mode)
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .build()
            .expect("valid enabled policy");
        let kerberos = config.kerberos_config().expect("Kerberos is synthesized");
        assert!(matches!(
            kerberos.kdc_resolution,
            ironrdp_connector::credssp::KdcResolution::KdcUrl(None)
        ));
        assert_eq!(kerberos.hostname, "client");
        let sspi: ironrdp_connector::sspi::KerberosConfig = kerberos.clone().into();
        assert!(matches!(
            sspi.kdc_resolution,
            ironrdp_connector::sspi::KdcResolution::KdcUrl(None)
        ));
        assert_eq!(sspi.client_computer_name, "client");
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_rejects_malformed_kdc_proxy() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for (url, name) in [
            (Some(""), None),
            (Some("not a URL"), None),
            (None, Some("http://[")),
            (Some(""), Some("remote-guard.invalid")),
            (Some("not a URL"), Some("remote-guard.invalid")),
        ] {
            let mut properties = PropertySet::new();
            if let Some(url) = url {
                properties.set_kdc_proxy_url(url);
            }
            if let Some(name) = name {
                properties.set_kdc_proxy_name(name);
            }
            let error = complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_property_set(&properties)
                .expect("proxy syntax is checked at build")
                .build()
                .expect_err("invalid proxy cannot silently fall back");
            assert_eq!(
                error.to_string(),
                "invalid KDC proxy configuration for remote credential guard"
            );
        }
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_preserves_explicit_kerberos_config() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for proxy in [None, Some("https://remote-guard.invalid/KdcProxy")] {
            let expected_url = proxy.map(|url| url::Url::parse(url).expect("reserved parse-only URL"));
            let kerberos = ironrdp_connector::credssp::KerberosConfig::new_with_kdc_url(
                proxy.map(str::to_owned),
                "remote_guard_explicit_client".to_owned(),
            )
            .expect("valid typed Kerberos config");
            let config = complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_kerberos_config(kerberos)
                .build()
                .expect("typed Kerberos config is retained");
            let kerberos = config.kerberos_config().expect("explicit Kerberos config");
            assert!(matches!(
                &kerberos.kdc_resolution,
                ironrdp_connector::credssp::KdcResolution::KdcUrl(url) if url == &expected_url
            ));
            assert_eq!(kerberos.hostname, "remote_guard_explicit_client");
            assert_eq!(config.properties().kdc_proxy_url(), proxy);
            let sspi: ironrdp_connector::sspi::KerberosConfig = kerberos.clone().into();
            assert!(matches!(
                &sspi.kdc_resolution,
                ironrdp_connector::sspi::KdcResolution::KdcUrl(url) if url == &expected_url
            ));
            assert_eq!(sspi.client_computer_name, "remote_guard_explicit_client");
        }
        assert_eq!(
            complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_kerberos_config(ironrdp_connector::credssp::KerberosConfig::new_with_iakerb(
                    "remote_guard_explicit_client".to_owned()
                ))
                .build()
                .expect_err("RCG does not support IAKerb")
                .to_string(),
            "remote credential guard does not support IAKerb"
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_preserves_property_kdc_proxy() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        for (key, value, expected) in [
            (
                "kdcproxyurl",
                "https://remote-guard.invalid/KdcProxy",
                "https://remote-guard.invalid/KdcProxy",
            ),
            (
                "kdcproxyname",
                "remote-guard.invalid",
                "https://remote-guard.invalid/KdcProxy",
            ),
            (
                "kdcproxyname",
                "https://remote-guard.invalid/custom",
                "https://remote-guard.invalid/custom",
            ),
            (
                "kdcproxyname",
                "http://remote-guard.invalid/custom",
                "http://remote-guard.invalid/custom",
            ),
        ] {
            let mut properties = PropertySet::new();
            properties.insert(key, value);
            if key == "kdcproxyurl" {
                properties.set_kdc_proxy_name("lower-precedence.invalid");
            }
            let config = complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_property_set(&properties)
                .expect("valid proxy property")
                .build()
                .expect("valid property-derived Kerberos config");
            let kerberos = config.kerberos_config().expect("proxy creates Kerberos config");
            let expected = url::Url::parse(expected).expect("reserved parse-only URL");
            assert!(matches!(
                &kerberos.kdc_resolution,
                ironrdp_connector::credssp::KdcResolution::KdcUrl(Some(url)) if url == &expected
            ));
            assert_eq!(kerberos.hostname, "client");
            assert_eq!(config.properties().get::<&str>(key), Some(value));
            let sspi: ironrdp_connector::sspi::KerberosConfig = kerberos.clone().into();
            assert!(matches!(
                &sspi.kdc_resolution,
                ironrdp_connector::sspi::KdcResolution::KdcUrl(Some(url)) if url == &expected
            ));
            assert_eq!(sspi.client_computer_name, "client");
        }
    }
}

#[test]
fn remote_guard_validation_precedence() {
    use ironrdp_tls::CertificateValidation::DangerouslyAcceptInvalidCertificate;

    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let builder = |tls, credssp, policy, name: &str, proxy: &str| {
            let mut properties = PropertySet::new();
            properties.set_kdc_proxy_url(proxy);
            complete_builder()
                .with_remote_guard_mode(mode)
                .with_tls(tls)
                .with_credssp(credssp)
                .with_certificate_validation(policy)
                .with_client_name(name)
                .with_property_set(&properties)
                .expect("proxy syntax is checked at build")
        };
        #[cfg(not(feature = "remote-guard"))]
        assert_eq!(
            builder(false, false, DangerouslyAcceptInvalidCertificate, "", "not a URL")
                .build()
                .expect_err("feature gate takes precedence")
                .to_string(),
            "remote credential guard requires the experimental remote-guard build feature"
        );
        #[cfg(feature = "remote-guard")]
        {
            use ironrdp_tls::CertificateValidation::Strict;

            assert_eq!(
                builder(false, false, DangerouslyAcceptInvalidCertificate, "", "not a URL")
                    .build()
                    .expect_err("security takes precedence")
                    .to_string(),
                "remote credential guard requires TLS and CredSSP"
            );
            assert_eq!(
                builder(true, true, DangerouslyAcceptInvalidCertificate, "", "not a URL")
                    .with_vmconnect("remote_guard_vm")
                    .build()
                    .expect_err("VMConnect precedes certificate validation")
                    .to_string(),
                "remote credential guard is not supported for VMConnect"
            );
            for (policy, name, expected) in [
                (
                    DangerouslyAcceptInvalidCertificate,
                    "",
                    "remote credential guard requires strict TLS certificate validation",
                ),
                (Strict, "", "remote credential guard requires a client computer name"),
                (
                    Strict,
                    "client",
                    "invalid KDC proxy configuration for remote credential guard",
                ),
            ] {
                assert_eq!(
                    builder(true, true, policy, name, "not a URL")
                        .build()
                        .expect_err("next invalid policy partition")
                        .to_string(),
                    expected
                );
            }
            let config = builder(true, true, Strict, "client", "https://remote-guard.invalid/KdcProxy")
                .build()
                .expect("repairing the final invalid input succeeds");
            assert_eq!(config.remote_guard_mode(), mode);
            let expected = url::Url::parse("https://remote-guard.invalid/KdcProxy").unwrap();
            assert!(matches!(
                &config.kerberos_config().expect("repaired proxy").kdc_resolution,
                ironrdp_connector::credssp::KdcResolution::KdcUrl(Some(url)) if url == &expected
            ));
        }
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_rejects_vmconnect() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        assert_eq!(
            complete_builder()
                .with_remote_guard_mode(mode)
                .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
                .with_vmconnect("remote_guard_vm")
                .build()
                .expect_err("remote VMConnect cannot use remote guard")
                .to_string(),
            "remote credential guard is not supported for VMConnect"
        );
    }
}

#[test]
fn remote_guard_disabled_connector_keeps_one_drdynvc() {
    let config = remote_guard_test_config(RemoteGuardMode::Disabled);
    let mut connector = __test_build_connector(&config, None).expect("ordinary connector");
    assert!(connector.__test_auto_reconnect_cookie().is_none());
    remote_guard_assert_negotiation_and_carrier(&mut connector, false);
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_enabled_connector_negotiation_and_carrier() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let config = remote_guard_test_config(mode);
        let mut connector = __test_build_connector(&config, None).expect("enabled connector");
        assert!(connector.__test_auto_reconnect_cookie().is_none());
        remote_guard_assert_negotiation_and_carrier(&mut connector, true);
    }
}

#[cfg(not(feature = "remote-guard"))]
#[test]
fn remote_guard_connector_feature_off_is_explicit() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let mut config = remote_guard_test_config(RemoteGuardMode::Disabled);
        config.__test_set_remote_guard_mode(mode);
        let error = match __test_build_connector(&config, None) {
            Ok(_) => panic!("connector must reject enabled mode without its feature"),
            Err(error) => error,
        };
        assert!(matches!(error.kind(), ironrdp_connector::ConnectorErrorKind::General));
        assert_eq!(
            error.to_string(),
            "[remote credential guard requires the experimental remote-guard build feature] general error"
        );
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_connector_rejects_arc_cookies() {
    let cookie = ServerAutoReconnect {
        logon_id: 1,
        random_bits: [0; 16],
    };
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let config = remote_guard_test_config(mode);
        let error = match __test_build_connector(&config, Some(&cookie)) {
            Ok(_) => panic!("enabled connector must reject ARC cookies"),
            Err(error) => error,
        };
        assert!(matches!(error.kind(), ironrdp_connector::ConnectorErrorKind::General));
        assert_eq!(
            error.to_string(),
            "[remote credential guard does not support auto-reconnect cookies] general error"
        );
    }
}

#[test]
fn remote_guard_disabled_connector_preserves_arc_cookie() {
    let config = remote_guard_test_config(RemoteGuardMode::Disabled);
    let cookie = ServerAutoReconnect {
        logon_id: 1,
        random_bits: [0; 16],
    };
    let mut connector = __test_build_connector(&config, Some(&cookie)).expect("ordinary ARC connector");
    assert_eq!(connector.__test_auto_reconnect_cookie(), Some(&cookie));
    remote_guard_assert_negotiation_and_carrier(&mut connector, false);
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_redirection_omits_password_cookie() {
    let cookie = b"redirect-cookie-sentinel".to_vec();
    let redirection = ServerRedirectionPdu {
        session_id: 7,
        target_net_address: Some("127.0.0.1".to_owned()),
        password: Some(cookie.clone()),
        ..ServerRedirectionPdu::default()
    };

    let mut ordinary = remote_guard_test_config(RemoteGuardMode::Disabled);
    __test_apply_server_redirection(&mut ordinary, &redirection).expect("apply ordinary redirection");
    let mut ordinary_connector = __test_build_connector(&ordinary, None).expect("build ordinary redirected connector");
    ordinary_connector.state = ClientConnectorState::SecureSettingsExchange {
        io_channel_id: 1003,
        user_channel_id: 1002,
    };
    let mut output = WriteBuf::new();
    ordinary_connector.step_no_input(&mut output).unwrap();
    let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
    assert!(
        data.user_data
            .windows(cookie.len())
            .any(|window| window == cookie.as_slice())
    );

    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let mut config = remote_guard_test_config(mode);
        __test_apply_server_redirection(&mut config, &redirection).expect("apply RCG redirection");
        let mut connector = __test_build_connector(&config, None).expect("build RCG redirected connector");
        connector.state = ClientConnectorState::SecureSettingsExchange {
            io_channel_id: 1003,
            user_channel_id: 1002,
        };
        let mut output = WriteBuf::new();
        connector.step_no_input(&mut output).unwrap();
        let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
        let encoded = data.user_data.as_ref();
        let info = decode::<ClientInfoPdu>(encoded).unwrap();
        assert!(info.client_info.credentials.password.is_empty());
        assert!(info.client_info.password_cookie.is_none());
        assert!(!encoded.windows(cookie.len()).any(|window| window == cookie.as_slice()));
    }
}

#[cfg(feature = "remote-guard")]
#[test]
fn remote_guard_run_rejects_auto_reconnect_before_io() {
    for mode in [
        RemoteGuardMode::Negotiate,
        RemoteGuardMode::Ntlm,
        RemoteGuardMode::Kerberos,
    ] {
        let config = remote_guard_test_config(mode);
        let (output_sender, mut output_receiver) = output_channel(1);
        let client = RdpClient::new(config, output_sender).with_auto_reconnect(1);
        let mut run = Box::pin(client.run());
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(run.as_mut().poll(&mut context), Poll::Ready(())));
        {
            let mut receive = Box::pin(output_receiver.recv());
            let error = match receive.as_mut().poll(&mut context) {
                Poll::Ready(Some(RdpOutputEvent::ConnectionFailure(error))) => error,
                _ => panic!("one poll must yield the queued connection failure"),
            };
            assert!(matches!(error.kind(), ironrdp_connector::ConnectorErrorKind::General));
            assert_eq!(
                error.to_string(),
                "[remote credential guard does not support automatic reconnection] general error"
            );
        }
        drop(run);
        let mut receive = Box::pin(output_receiver.recv());
        assert!(matches!(receive.as_mut().poll(&mut context), Poll::Ready(None)));
    }
}

//! Redirection-warning trust state from `IMsRdpClientNonScriptable3/4`.
//!
//! The Windows client decides before connecting whether to show its redirection warning from
//! `ShowRedirectionWarningDialog`, the publisher classification in `RedirectionWarningType`, the
//! per-resource `WarnAbout*` filters, and `LaunchedViaClientShellInterface`. IronRDP keeps the same
//! inputs and presents one accept/cancel prompt that lists the local resources it will redirect.

use core::ptr::NonNull;

use windows::Win32::Security::Cryptography::{
    CERT_CHAIN_CONTEXT, CERT_CONTEXT, CERT_NAME_SIMPLE_DISPLAY_TYPE, CertDuplicateCertificateChain,
    CertFreeCertificateChain, CertGetNameStringW,
};

/// `__MIDL_IMsRdpClientNonScriptable4_0001` from the MSTSCLib type library.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RedirectionWarningType {
    #[default]
    Default = 0,
    Unsigned = 1,
    Unknown = 2,
    User = 3,
    ThirdPartySigned = 4,
    Trusted = 5,
}

impl RedirectionWarningType {
    pub(crate) fn from_raw(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::Default,
            1 => Self::Unsigned,
            2 => Self::Unknown,
            3 => Self::User,
            4 => Self::ThirdPartySigned,
            5 => Self::Trusted,
            _ => return None,
        })
    }
}

/// An owned reference to a publisher `CERT_CHAIN_CONTEXT`.
#[derive(Debug)]
pub(crate) struct PublisherChain(NonNull<CERT_CHAIN_CONTEXT>);

impl PublisherChain {
    /// Takes ownership of one reference to `chain`.
    ///
    /// # Safety
    ///
    /// `chain` must be a valid chain context whose reference the caller transfers.
    pub(crate) unsafe fn from_owned(chain: *mut CERT_CHAIN_CONTEXT) -> Option<Self> {
        NonNull::new(chain).map(Self)
    }

    /// Validates a host-supplied chain like mstscax (one simple chain whose first element has a
    /// certificate) and takes a new reference to it.
    ///
    /// # Safety
    ///
    /// `chain` must be null or point to a valid chain context.
    pub(crate) unsafe fn duplicate_validated(chain: *const CERT_CHAIN_CONTEXT) -> Option<Self> {
        let context = unsafe { chain.as_ref() }?;
        if context.cChain != 1 || context.rgpChain.is_null() {
            return None;
        }
        let simple = unsafe { (*context.rgpChain).as_ref() }?;
        if simple.cElement == 0 || simple.rgpElement.is_null() {
            return None;
        }
        let element = unsafe { (*simple.rgpElement).as_ref() }?;
        if element.pCertContext.is_null() {
            return None;
        }
        unsafe { Self::from_owned(CertDuplicateCertificateChain(chain)) }
    }

    pub(crate) fn as_ptr(&self) -> *const CERT_CHAIN_CONTEXT {
        self.0.as_ptr()
    }

    /// A new reference that the receiver must release with `CertFreeCertificateChain`.
    pub(crate) fn duplicate_raw(&self) -> *mut CERT_CHAIN_CONTEXT {
        unsafe { CertDuplicateCertificateChain(self.as_ptr()) }
    }

    pub(crate) fn leaf_certificate(&self) -> *const CERT_CONTEXT {
        let context = unsafe { self.0.as_ref() };
        unsafe { (**(**context.rgpChain).rgpElement).pCertContext }
    }

    /// The publisher's display name, bounded for UI.
    pub(crate) fn publisher_name(&self) -> Option<String> {
        let certificate = self.leaf_certificate();
        let mut buffer = [0u16; 256];
        let written =
            unsafe { CertGetNameStringW(certificate, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buffer)) };
        let length = (written as usize).saturating_sub(1).min(buffer.len());
        let name = String::from_utf16_lossy(&buffer[..length]);
        let name = name.trim();
        (!name.is_empty()).then(|| name.chars().filter(|character| !character.is_control()).collect())
    }
}

impl Clone for PublisherChain {
    fn clone(&self) -> Self {
        // CertDuplicateCertificateChain only increments the reference count of a valid context.
        Self(NonNull::new(self.duplicate_raw()).expect("duplicated chain context"))
    }
}

impl Drop for PublisherChain {
    fn drop(&mut self) {
        unsafe { CertFreeCertificateChain(self.as_ptr()) };
    }
}

#[derive(Debug, Default)]
pub(crate) struct TrustSettings {
    pub(crate) show_redirection_warning_dialog: bool,
    pub(crate) redirection_warning_type: RedirectionWarningType,
    pub(crate) warn_about_printers: bool,
    pub(crate) launched_via_client_shell: bool,
    pub(crate) publisher_chain: Option<PublisherChain>,
}

impl TrustSettings {
    /// Applies an `.rdp` file's publisher classification like `ApplyRDPSigningSettings`: the
    /// warning is shown unless the publisher is trusted or the file disables it.
    pub(crate) fn apply_signature(
        &mut self,
        warning_type: RedirectionWarningType,
        chain: Option<PublisherChain>,
        show_publisher_warning: bool,
    ) {
        self.show_redirection_warning_dialog =
            show_publisher_warning && warning_type != RedirectionWarningType::Trusted;
        self.redirection_warning_type = warning_type;
        self.publisher_chain = chain;
    }
}

/// Local resources that a connection redirects, in prompt order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RedirectedResource {
    Drives,
    Clipboard,
    Printers,
    Ports,
    SmartCards,
    WebAuthn,
    AudioCapture,
    Cameras,
}

impl RedirectedResource {
    fn label(self) -> &'static str {
        match self {
            Self::Drives => "Drives",
            Self::Clipboard => "Clipboard",
            Self::Printers => "Printers",
            Self::Ports => "Serial and parallel ports",
            Self::SmartCards => "Smart cards",
            Self::WebAuthn => "Security keys and Windows Hello (WebAuthn)",
            Self::AudioCapture => "Microphone",
            Self::Cameras => "Cameras",
        }
    }
}

/// The resources to list in the redirection warning, or `None` when no prompt is required.
///
/// This mirrors `CRedirectPromptDlg::AllowConnection`: nothing is shown unless
/// `ShowRedirectionWarningDialog` is set; clipboard and printers are listed only when their
/// `WarnAbout*` filter is set; and a session launched through the client-shell interface always
/// prompts unless its publisher is classified as third-party signed.
pub(crate) fn redirection_warning_resources(
    settings: &TrustSettings,
    warn_about_clipboard: bool,
    redirected: &[RedirectedResource],
) -> Option<Vec<RedirectedResource>> {
    if !settings.show_redirection_warning_dialog {
        return None;
    }
    let listed = redirected
        .iter()
        .copied()
        .filter(|resource| match resource {
            RedirectedResource::Clipboard => warn_about_clipboard,
            RedirectedResource::Printers => settings.warn_about_printers,
            _ => true,
        })
        .collect::<Vec<_>>();
    let forced = settings.launched_via_client_shell
        && settings.redirection_warning_type != RedirectionWarningType::ThirdPartySigned;
    (forced || !listed.is_empty()).then_some(listed)
}

/// Title, main instruction, and content of the redirection warning. Only the host-supplied
/// publisher name is shown; the endpoint, account, and remote details never are.
pub(crate) fn redirection_warning_text(
    warning_type: RedirectionWarningType,
    publisher: Option<&str>,
    resources: &[RedirectedResource],
) -> (&'static str, String, String) {
    let instruction = match (warning_type, publisher) {
        (RedirectionWarningType::Trusted, _) => "This remote connection comes from a trusted publisher.".to_owned(),
        (RedirectionWarningType::ThirdPartySigned, Some(_)) => {
            "Make sure you trust the publisher of this remote connection.".to_owned()
        }
        (RedirectionWarningType::Unknown, _) => "The publisher of this remote connection can't be verified.".to_owned(),
        _ => "The publisher of this remote connection can't be identified.".to_owned(),
    };
    let mut content = String::new();
    if let Some(publisher) = publisher {
        content.push_str("Publisher: ");
        content.push_str(publisher);
        content.push_str("\r\n\r\n");
    }
    if resources.is_empty() {
        content.push_str("No local resources will be redirected.");
    } else {
        content.push_str("The remote computer will be able to access these local resources:");
        for resource in resources {
            content.push_str("\r\n\u{2022} ");
            content.push_str(resource.label());
        }
    }
    content.push_str("\r\n\r\nContinue only if you trust this connection.");
    ("IronRDP redirection warning", instruction, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(show: bool) -> TrustSettings {
        TrustSettings {
            show_redirection_warning_dialog: show,
            ..TrustSettings::default()
        }
    }

    #[test]
    fn redirection_warning_requires_the_public_switch() {
        assert_eq!(
            redirection_warning_resources(&settings(false), true, &[RedirectedResource::Drives]),
            None
        );
        assert_eq!(
            redirection_warning_resources(&settings(true), true, &[RedirectedResource::Drives]),
            Some(vec![RedirectedResource::Drives])
        );
    }

    #[test]
    fn warn_about_filters_hide_clipboard_and_printers() {
        let redirected = [RedirectedResource::Clipboard, RedirectedResource::Printers];
        assert_eq!(redirection_warning_resources(&settings(true), false, &redirected), None);

        let mut printers = settings(true);
        printers.warn_about_printers = true;
        assert_eq!(
            redirection_warning_resources(&printers, true, &redirected),
            Some(redirected.to_vec())
        );
    }

    #[test]
    fn client_shell_launch_forces_prompt_unless_third_party_signed() {
        let mut launched = settings(true);
        launched.launched_via_client_shell = true;
        assert_eq!(redirection_warning_resources(&launched, false, &[]), Some(Vec::new()));

        launched.redirection_warning_type = RedirectionWarningType::ThirdPartySigned;
        assert_eq!(redirection_warning_resources(&launched, false, &[]), None);
    }

    #[test]
    fn warning_text_names_only_publisher_and_resources() {
        let (_, instruction, content) = redirection_warning_text(
            RedirectionWarningType::ThirdPartySigned,
            Some("Contoso"),
            &[RedirectedResource::Drives, RedirectedResource::SmartCards],
        );
        assert_eq!(
            instruction,
            "Make sure you trust the publisher of this remote connection."
        );
        assert_eq!(
            content,
            "Publisher: Contoso\r\n\r\nThe remote computer will be able to access these local resources:\
             \r\n\u{2022} Drives\r\n\u{2022} Smart cards\r\n\r\nContinue only if you trust this connection."
        );

        let (_, instruction, content) = redirection_warning_text(RedirectionWarningType::Unsigned, None, &[]);
        assert_eq!(
            instruction,
            "The publisher of this remote connection can't be identified."
        );
        assert!(content.starts_with("No local resources will be redirected."));
    }

    #[test]
    fn warning_type_accepts_only_published_values() {
        assert_eq!(
            RedirectionWarningType::from_raw(5),
            Some(RedirectionWarningType::Trusted)
        );
        assert_eq!(RedirectionWarningType::from_raw(6), None);
        assert_eq!(RedirectionWarningType::from_raw(-1), None);
    }
}

#[cfg(test)]
mod control_tests {
    use core::ptr;

    use windows::Win32::Foundation::{E_ACCESSDENIED, E_INVALIDARG, E_NOTIMPL, E_UNEXPECTED, S_FALSE};
    use windows::Win32::Security::Cryptography::{CERT_CHAIN_PARA, CertGetCertificateChain};
    use windows::Win32::System::Com::IDispatch;
    use windows::Win32::System::Variant::{VARIANT, VT_BYREF, VT_EMPTY, VT_I4};
    use windows_core::{BSTR, Interface as _};

    use super::*;
    use crate::control::{Control, variant_header, variant_header_mut};
    use crate::mstsc::{
        IMsRdpClient5, IMsRdpClientNonScriptable4, IMsRdpClientNonScriptable5, IMsRdpClientShell, IMsTscAx,
    };
    use crate::mstsc::{IRemoteDesktopClient, IRemoteDesktopClientSettings};
    use crate::rdp_signature::tests::{CODE_SIGNING_EKU, TestSigner};

    const TRUE: i16 = -1;
    const FALSE: i16 = 0;

    fn get_bool(read: impl FnOnce(*mut i16) -> windows_core::Result<()>) -> i16 {
        let mut value = 7;
        read(&mut value).expect("read Boolean member");
        value
    }

    #[test]
    fn trust_members_round_trip_or_refuse_unsupported_values() {
        let control: IMsRdpClientNonScriptable5 = Control::new().into();
        unsafe {
            control.put_ShowRedirectionWarningDialog(TRUE).unwrap();
            control.put_RedirectionWarningType(4).unwrap();
            control.put_WarnAboutPrinterRedirection(TRUE).unwrap();
            control.put_LaunchedViaClientShellInterface(TRUE).unwrap();
            assert_eq!(get_bool(|value| control.get_ShowRedirectionWarningDialog(value)), TRUE);
            assert_eq!(get_bool(|value| control.get_WarnAboutPrinterRedirection(value)), TRUE);
            assert_eq!(
                get_bool(|value| control.get_LaunchedViaClientShellInterface(value)),
                TRUE
            );
            let mut warning_type = 0;
            control.get_RedirectionWarningType(&mut warning_type).unwrap();
            assert_eq!(warning_type, 4);
            assert_eq!(control.put_RedirectionWarningType(6).unwrap_err().code(), E_INVALIDARG);

            for (put, get) in [
                (
                    IMsRdpClientNonScriptable4::put_MarkRdpSettingsSecure as unsafe fn(&_, i16) -> _,
                    IMsRdpClientNonScriptable4::get_MarkRdpSettingsSecure as unsafe fn(&_, *mut i16) -> _,
                ),
                (
                    IMsRdpClientNonScriptable4::put_AllowCredentialSaving,
                    IMsRdpClientNonScriptable4::get_AllowCredentialSaving,
                ),
                (
                    IMsRdpClientNonScriptable4::put_TrustedZoneSite,
                    IMsRdpClientNonScriptable4::get_TrustedZoneSite,
                ),
            ] {
                let control = control.cast::<IMsRdpClientNonScriptable4>().unwrap();
                put(&control, FALSE).unwrap();
                assert_eq!(put(&control, TRUE).unwrap_err().code(), E_NOTIMPL);
                assert_eq!(get_bool(|value| get(&control, value)), FALSE);
            }
            control.put_DisableRemoteAppCapsCheck(FALSE).unwrap();
            assert_eq!(
                control.put_DisableRemoteAppCapsCheck(TRUE).unwrap_err().code(),
                E_NOTIMPL
            );
            control.put_WarnAboutDirectXRedirection(FALSE).unwrap();
            assert_eq!(
                control.put_WarnAboutDirectXRedirection(TRUE).unwrap_err().code(),
                E_NOTIMPL
            );
        }
    }

    #[test]
    fn publisher_certificate_chain_is_validated_and_duplicated() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let para = CERT_CHAIN_PARA {
            cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
            ..Default::default()
        };
        let mut raw_chain = ptr::null_mut();
        unsafe { CertGetCertificateChain(None, signer.certificate, None, None, &para, 0, None, &mut raw_chain) }
            .expect("build chain");
        let chain = unsafe { PublisherChain::from_owned(raw_chain) }.expect("chain context");
        assert_eq!(chain.publisher_name().as_deref(), Some("IronRDP test publisher"));

        let control: IMsRdpClientNonScriptable4 = Control::new().into();
        let mut input = VARIANT::default();
        unsafe {
            let header = variant_header_mut(&mut input);
            header.vt = VT_BYREF;
            header.Anonymous.byref = chain.as_ptr().cast_mut().cast();
            control.put_PublisherCertificateChain(&mut input).expect("store chain");
        }
        let mut output = VARIANT::default();
        unsafe { control.get_PublisherCertificateChain(&mut output) }.expect("read chain");
        let header = variant_header(&output);
        assert_eq!(header.vt, VT_BYREF);
        let returned = unsafe { PublisherChain::from_owned(header.Anonymous.byref.cast()) }.expect("owned reference");
        assert_eq!(returned.as_ptr(), chain.as_ptr());

        let mut empty = VARIANT::default();
        unsafe { control.put_PublisherCertificateChain(&mut empty) }.expect("clear chain");
        unsafe { control.get_PublisherCertificateChain(&mut output) }.expect("read cleared chain");
        assert_eq!(variant_header(&output).vt, VT_EMPTY);

        let mut wrong_type = VARIANT::default();
        variant_header_mut(&mut wrong_type).vt = VT_I4;
        assert_eq!(
            unsafe { control.put_PublisherCertificateChain(&mut wrong_type) }
                .unwrap_err()
                .code(),
            E_INVALIDARG
        );
        let mut null_chain = VARIANT::default();
        variant_header_mut(&mut null_chain).vt = VT_BYREF;
        assert_eq!(
            unsafe { control.put_PublisherCertificateChain(&mut null_chain) }
                .unwrap_err()
                .code(),
            E_INVALIDARG
        );
    }

    fn apply_settings(client: &IRemoteDesktopClient, contents: &str) -> windows_core::Result<()> {
        let mut raw = ptr::null_mut();
        unsafe { client.get_Settings(&mut raw) }.expect("modern settings");
        let settings = unsafe { IRemoteDesktopClientSettings::from_raw(raw) };
        let contents = BSTR::from(contents);
        unsafe { settings.ApplySettings(contents.as_ptr()) }
    }

    fn warning_state(client: &IRemoteDesktopClient) -> (i16, i32) {
        let control = client.cast::<IMsRdpClientNonScriptable4>().unwrap();
        let mut warning_type = 0;
        unsafe { control.get_RedirectionWarningType(&mut warning_type) }.unwrap();
        (
            get_bool(|value| unsafe { control.get_ShowRedirectionWarningDialog(value) }),
            warning_type,
        )
    }

    const MODERN_UNSIGNED: &str = "full address:s:host.example\r\ndesktopwidth:i:1280\r\n";
    const MODERN_SCOPE: &str = "Full Address";

    #[test]
    fn modern_apply_settings_classifies_the_publisher() {
        let client: IRemoteDesktopClient = Control::new().into();
        apply_settings(&client, MODERN_UNSIGNED).expect("apply unsigned file");
        assert_eq!(warning_state(&client), (TRUE, RedirectionWarningType::User as i32));

        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        apply_settings(&client, &signer.sign(MODERN_UNSIGNED, MODERN_SCOPE)).expect("apply signed file");
        assert_eq!(warning_state(&client), (TRUE, RedirectionWarningType::Unknown as i32));

        let quiet = format!(
            "{}showpublisherwarningdialog:i:0\r\n",
            signer.sign(MODERN_UNSIGNED, MODERN_SCOPE)
        );
        apply_settings(&client, &quiet).expect("apply file without publisher warning");
        assert_eq!(warning_state(&client), (FALSE, RedirectionWarningType::Unknown as i32));

        let tampered = signer
            .sign(MODERN_UNSIGNED, MODERN_SCOPE)
            .replace("host.example", "attacker.example");
        assert_eq!(apply_settings(&client, &tampered).unwrap_err().code(), E_ACCESSDENIED);
        assert_eq!(warning_state(&client), (FALSE, RedirectionWarningType::Unknown as i32));
    }

    #[test]
    fn client_shell_is_one_object_per_control() {
        let control: IMsRdpClient5 = Control::new().into();
        let mut first = ptr::null_mut();
        let mut second = ptr::null_mut();
        unsafe {
            control.get_MsRdpClientShell(&mut first).unwrap();
            control.get_MsRdpClientShell(&mut second).unwrap();
        }
        let first = unsafe { IMsRdpClientShell::from_raw(first) };
        let second = unsafe { IMsRdpClientShell::from_raw(second) };
        assert_eq!(first.as_raw(), second.as_raw());

        let contents = BSTR::from("full address:s:host\r\n");
        unsafe { first.put_RdpFileContents(contents.as_ptr()) }.unwrap();
        let mut installed = 0;
        unsafe { second.get_IsRemoteProgramClientInstalled(&mut installed) }.unwrap();
        assert_eq!(installed, TRUE);
        assert_eq!(unsafe { second.ShowTrustedSitesManagementDialog() }, S_FALSE);
        unsafe { second.put_PublicMode(TRUE) }.unwrap();
        assert_eq!(get_bool(|value| unsafe { second.get_PublicMode(value) }), TRUE);
        assert!(second.cast::<IDispatch>().is_ok());
    }

    fn client_shell(control: &IMsRdpClient5, contents: &str) -> IMsRdpClientShell {
        let mut raw = ptr::null_mut();
        unsafe { control.get_MsRdpClientShell(&mut raw) }.unwrap();
        let shell = unsafe { IMsRdpClientShell::from_raw(raw) };
        let contents = BSTR::from(contents);
        unsafe { shell.put_RdpFileContents(contents.as_ptr()) }.unwrap();
        shell
    }

    fn server(control: &IMsRdpClient5) -> String {
        let mut server = ptr::null();
        unsafe { control.cast::<IMsTscAx>().unwrap().get_Server(&mut server) }.unwrap();
        unsafe { BSTR::from_raw(server) }.to_string()
    }

    #[test]
    fn launch_applies_the_records_to_the_owning_control() {
        let control: IMsRdpClient5 = Control::new().into();
        let shell = client_shell(&control, MODERN_UNSIGNED);
        // Without a password the control stays idle, as it does for any Connect.
        unsafe { shell.Launch() }.expect("launch in-process");
        assert_eq!(server(&control), "host.example");

        let client = control.cast::<IRemoteDesktopClient>().unwrap();
        assert_eq!(warning_state(&client), (TRUE, RedirectionWarningType::User as i32));
        let trust = control.cast::<IMsRdpClientNonScriptable4>().unwrap();
        assert_eq!(
            get_bool(|value| unsafe { trust.get_LaunchedViaClientShellInterface(value) }),
            TRUE
        );
    }

    #[test]
    fn launch_rejects_tampered_files_without_changing_the_control() {
        let control: IMsRdpClient5 = Control::new().into();
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let tampered = signer
            .sign(MODERN_UNSIGNED, MODERN_SCOPE)
            .replace("host.example", "attacker.example");
        let shell = client_shell(&control, &tampered);
        assert_eq!(unsafe { shell.Launch() }.unwrap_err().code(), E_ACCESSDENIED);
        assert_eq!(server(&control), "");
        let trust = control.cast::<IMsRdpClientNonScriptable4>().unwrap();
        assert_eq!(
            get_bool(|value| unsafe { trust.get_LaunchedViaClientShellInterface(value) }),
            FALSE
        );
    }

    #[test]
    fn launch_after_the_control_is_destroyed_is_unexpected() {
        let control: IMsRdpClient5 = Control::new().into();
        let shell = client_shell(&control, MODERN_UNSIGNED);
        drop(control);
        assert_eq!(unsafe { shell.Launch() }.unwrap_err().code(), E_UNEXPECTED);
    }
}

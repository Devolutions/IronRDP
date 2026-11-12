//! Signed `.rdp` file verification, matching the Windows Remote Desktop client.
//!
//! A signed file carries `signscope:s:` (comma-separated record names) and `signature:s:` (Base64 of
//! a 12-byte header followed by a detached PKCS #7 SignedData). The signed bytes are the UTF-16LE
//! text of each scoped record rendered as `name:type:value\r\n` with a lowercase name, in scope
//! order, then the `signscope` record itself, then a UTF-16 NUL. The signer certificate must carry a
//! server-authentication, code-signing, or RDP-signing EKU, be time-valid, and chain under the
//! Authenticode policy. The outcome classifies the publisher for `RedirectionWarningType`.

use base64::Engine as _;
use windows::Win32::Foundation::{
    CRYPT_E_HASH_VALUE, ERROR_SUCCESS, FILETIME, NTE_BAD_SIGNATURE, STATUS_INVALID_SIGNATURE,
};
use windows::Win32::Security::Cryptography::{
    CERT_CHAIN_CONTEXT, CERT_CHAIN_PARA, CERT_CHAIN_POLICY_AUTHENTICODE, CERT_CHAIN_POLICY_FLAGS,
    CERT_CHAIN_POLICY_PARA, CERT_CHAIN_POLICY_STATUS, CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT, CERT_CONTEXT,
    CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG, CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE, CERT_STORE_PROV_PKCS7,
    CERT_TRUST_IS_NOT_SIGNATURE_VALID, CERT_TRUST_IS_NOT_TIME_VALID, CERT_TRUST_IS_OFFLINE_REVOCATION,
    CERT_TRUST_IS_REVOKED, CERT_TRUST_IS_UNTRUSTED_ROOT, CERT_TRUST_REVOCATION_STATUS_UNKNOWN, CRYPT_INTEGER_BLOB,
    CRYPT_VERIFY_MESSAGE_PARA, CTL_USAGE, CertCloseStore, CertFreeCertificateContext, CertGetCertificateChain,
    CertGetEnhancedKeyUsage, CertOpenStore, CertVerifyCertificateChainPolicy, CertVerifyTimeValidity,
    CryptHashCertificate2, CryptVerifyDetachedMessageSignature, HCERTCHAINENGINE, PKCS_7_ASN_ENCODING,
    X509_ASN_ENCODING,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_core::{HSTRING, PCWSTR};

use crate::client_shell::{RdpRecordValue, RdpRecords};
use crate::trust::{PublisherChain, RedirectionWarningType};

/// Settings that a signature must cover whenever they are present, in the order mstsc writes
/// them to `signscope`. `Full Address` is mandatory.
pub(crate) const SENSITIVE_SETTINGS: [&str; 48] = [
    "Full Address",
    "Alternate Full Address",
    "PCB",
    "Use Redirection Server Name",
    "Server Port",
    "Negotiate Security Layer",
    "EnableCredSspSupport",
    "DisableConnectionSharing",
    "AutoReconnection Enabled",
    "GatewayHostname",
    "GatewayUsageMethod",
    "GatewayProfileUsageMethod",
    "GatewayCredentialsSource",
    "Support URL",
    "PromptCredentialOnce",
    "Require pre-authentication",
    "Pre-authentication server address",
    "Alternate Shell",
    "Shell Working Directory",
    "RemoteApplicationProgram",
    "RemoteApplicationExpandWorkingdir",
    "RemoteApplicationMode",
    "RemoteApplicationGuid",
    "RemoteApplicationName",
    "RemoteApplicationIcon",
    "RemoteApplicationFile",
    "RemoteApplicationFileExtensions",
    "RemoteApplicationCmdLine",
    "RemoteApplicationExpandCmdLine",
    "Prompt For Credentials",
    "Authentication Level",
    "AudioMode",
    "RedirectDrives",
    "RedirectPrinters",
    "RedirectCOMPorts",
    "RedirectSmartCards",
    "RedirectPOSDevices",
    "RedirectClipboard",
    "DevicesToRedirect",
    "DrivesToRedirect",
    "LoadBalanceInfo",
    "RedirectDirectX",
    "RDGIsKDCProxy",
    "KDCProxyName",
    "EnableRdsAadAuth",
    "RedirectWebAuthn",
    "RedirectTextProcessing",
    "allowed security protocols",
];

/// Records that carry the signature itself rather than connection settings.
pub(crate) const SIGNATURE_RECORDS: [&str; 2] = ["signscope", "signature"];

const MAX_SIGN_SCOPE_CHARS: usize = 0x800;
const SIGNATURE_HEADER_LEN: usize = 12;
const SIGNATURE_TYPE_CERTIFICATE: u16 = 1;

// Verification flags, as computed by the Windows client.
const FLAG_WRONG_USAGE: u32 = 0x1001;
const FLAG_NOT_TIME_VALID: u32 = 0x1002;
const FLAG_REVOKED: u32 = 0x1004;
const FLAG_REVOCATION_UNKNOWN: u32 = 0x1008;
const FLAG_BAD_CERTIFICATE_SIGNATURE: u32 = 0x1010;
const FLAG_UNTRUSTED_ROOT: u32 = 0x1_0001;
const FLAG_OTHER: u32 = 0x1_0000;
const FLAG_CERTIFICATE_CLASS: u32 = 0x1000;
const FLAG_MISSING_SCOPE: u32 = 0x10_0000;
const FLAG_TAMPERED: u32 = 0x20_0000;

const RDP_SIGNING_EKUS: [&str; 3] = ["1.3.6.1.5.5.7.3.1", "1.3.6.1.5.5.7.3.3", "1.3.6.1.4.1.311.54.1.1"];

const TERMINAL_SERVICES_POLICY_KEY: &str = "Software\\Policies\\Microsoft\\Windows NT\\Terminal Services";
const TERMINAL_SERVER_CLIENT_KEY: &str = "SOFTWARE\\Microsoft\\Terminal Server Client";

/// Publisher classification of an `.rdp` file (`LoadPublisherCategory`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PublisherStatus {
    /// Signed, but only the certificate's validity period failed.
    Unsigned,
    /// Signed by a certificate that does not chain to a trusted root.
    Unknown,
    /// Not signed, or the signature cannot be decoded.
    User,
    /// Signed by a valid certificate that is not in `TrustedCertThumbprints`.
    ThirdParty,
    /// Signed by a valid certificate listed in `TrustedCertThumbprints`.
    Trusted,
    /// The signature does not match the settings, or the certificate is unusable.
    Blocked,
}

impl PublisherStatus {
    /// The `RedirectionWarningType` that the Windows client applies for this classification.
    pub(crate) fn redirection_warning_type(self) -> RedirectionWarningType {
        match self {
            Self::Unsigned | Self::Blocked => RedirectionWarningType::Unsigned,
            Self::Unknown => RedirectionWarningType::Unknown,
            Self::User => RedirectionWarningType::User,
            Self::ThirdParty => RedirectionWarningType::ThirdPartySigned,
            Self::Trusted => RedirectionWarningType::Trusted,
        }
    }

    fn from_failure_flags(flags: u32) -> Self {
        if flags & FLAG_TAMPERED != 0 {
            Self::Blocked
        } else if flags & FLAG_CERTIFICATE_CLASS != 0 {
            if flags == FLAG_NOT_TIME_VALID {
                Self::Unsigned
            } else {
                Self::Blocked
            }
        } else if flags & FLAG_OTHER != 0 {
            Self::Unknown
        } else {
            Self::Unsigned
        }
    }
}

#[derive(Debug)]
pub(crate) struct SignatureVerification {
    pub(crate) status: PublisherStatus,
    /// The verified publisher chain, present only for `ThirdParty` and `Trusted`.
    pub(crate) chain: Option<PublisherChain>,
}

/// Whether the machine policy lets a file with this classification be used.
pub(crate) fn signature_policy_allows(status: PublisherStatus, policy: &SignaturePolicy) -> bool {
    match status {
        PublisherStatus::Unsigned | PublisherStatus::Unknown => policy.allow_unsigned_files,
        PublisherStatus::User | PublisherStatus::ThirdParty => policy.allow_signed_files,
        PublisherStatus::Trusted => true,
        PublisherStatus::Blocked => false,
    }
}

/// `AllowUnsignedFiles`, `AllowSignedFiles`, and `TrustedCertThumbprints` group policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SignaturePolicy {
    pub(crate) allow_unsigned_files: bool,
    pub(crate) allow_signed_files: bool,
    pub(crate) trusted_thumbprints: Vec<Thumbprint>,
    pub(crate) revocation_check: bool,
}

impl Default for SignaturePolicy {
    fn default() -> Self {
        Self {
            allow_unsigned_files: true,
            allow_signed_files: true,
            trusted_thumbprints: Vec::new(),
            revocation_check: false,
        }
    }
}

impl SignaturePolicy {
    // UNTESTED: policy values come from HKLM before HKCU, and `RdpSignCertChainRevocationCheck` from
    // HKCU before HKLM, matching the Windows client. Tests use explicit policies instead of the
    // registry; when changing these lookups, recheck the hive order and the defaults for absent values.
    pub(crate) fn from_registry() -> Self {
        Self::from_values(
            policy_dword,
            policy_string,
            user_setting(|root| registry_dword(root, TERMINAL_SERVER_CLIENT_KEY, "RdpSignCertChainRevocationCheck")),
        )
    }

    /// Builds the policy from the Terminal Services policy values and the user's revocation setting.
    fn from_values(
        policy_dword: impl Fn(&str) -> Option<u32>,
        policy_string: impl Fn(&str) -> Option<String>,
        revocation_check: Option<u32>,
    ) -> Self {
        let disable_sha1 = policy_dword("DisableSHA1CertThumbprints").is_some_and(|value| value != 0);
        Self {
            allow_unsigned_files: policy_dword("AllowUnsignedFiles").is_none_or(|value| value != 0),
            allow_signed_files: policy_dword("AllowSignedFiles").is_none_or(|value| value != 0),
            trusted_thumbprints: policy_string("TrustedCertThumbprints")
                .map(|value| parse_trusted_thumbprints(&value, disable_sha1))
                .unwrap_or_default(),
            revocation_check: revocation_check == Some(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThumbprintAlgorithm {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl ThumbprintAlgorithm {
    fn cng_name(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA1",
            Self::Sha256 => "SHA256",
            Self::Sha384 => "SHA384",
            Self::Sha512 => "SHA512",
        }
    }

    fn digest_len(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Thumbprint {
    pub(crate) algorithm: ThumbprintAlgorithm,
    pub(crate) digest: Vec<u8>,
}

/// Parses `TrustedCertThumbprints`: comma-separated hex digests with an optional `sha256:`,
/// `sha384:`, or `sha512:` prefix (SHA-1 otherwise). Whitespace and a leading U+200E mark are
/// ignored; entries of the wrong length are dropped.
pub(crate) fn parse_trusted_thumbprints(value: &str, disable_sha1: bool) -> Vec<Thumbprint> {
    value
        .split(',')
        .filter_map(|entry| {
            let entry = entry
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            let entry = entry.strip_prefix('\u{200E}').unwrap_or(&entry);
            let lowercase = entry.to_ascii_lowercase();
            let (algorithm, hex) = [
                ("sha256:", ThumbprintAlgorithm::Sha256),
                ("sha384:", ThumbprintAlgorithm::Sha384),
                ("sha512:", ThumbprintAlgorithm::Sha512),
            ]
            .into_iter()
            .find_map(|(prefix, algorithm)| lowercase.strip_prefix(prefix).map(|hex| (algorithm, hex)))
            .unwrap_or((ThumbprintAlgorithm::Sha1, lowercase.as_str()));
            if algorithm == ThumbprintAlgorithm::Sha1 && disable_sha1 {
                return None;
            }
            let digest = decode_hex(hex)?;
            (digest.len() == algorithm.digest_len()).then_some(Thumbprint { algorithm, digest })
        })
        .collect()
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect()
}

/// Checks that the scope covers exactly the sensitive settings present (`VerifySensitiveSettings`)
/// and returns the signed bytes (`GenerateSecureSettingsBlob`).
pub(crate) fn secure_settings_blob(records: &RdpRecords) -> Option<Vec<u8>> {
    let Some(RdpRecordValue::Str(scope)) = records.get("signscope") else {
        return None;
    };
    if scope.is_empty() || MAX_SIGN_SCOPE_CHARS <= scope.encode_utf16().count() {
        return None;
    }
    let tokens = scope.split(',').collect::<Vec<_>>();
    records.get("full address")?;
    for setting in SENSITIVE_SETTINGS {
        let present = records.get(setting).is_some();
        let scoped = tokens.iter().any(|token| token.eq_ignore_ascii_case(setting));
        if present != scoped {
            return None;
        }
    }

    let mut text = String::new();
    for token in tokens {
        let value = records.get(token)?;
        push_signed_record(&mut text, token, value)?;
    }
    push_signed_record(&mut text, "signscope", &RdpRecordValue::Str(scope.clone()))?;
    let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(core::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    Some(bytes)
}

fn push_signed_record(text: &mut String, name: &str, value: &RdpRecordValue) -> Option<()> {
    text.push_str(&name.to_lowercase());
    match value {
        RdpRecordValue::Int(value) => {
            text.push_str(":i:");
            text.push_str(&value.to_string());
        }
        RdpRecordValue::Str(value) => {
            text.push_str(":s:");
            text.push_str(value);
        }
        // UNTESTED: Windows renders signed binary records in a form that could only be read from
        // decompiled code. None of the sensitive settings are binary, so scoped binary records are
        // rejected instead of risking a false match. Revisit with a Windows-signed fixture.
        RdpRecordValue::Binary(_) => return None,
    }
    text.push_str("\r\n");
    Some(())
}

/// Decodes `signature:s:` into its PKCS #7 SignedData, or `None` when the record is not a
/// certificate signature (`CTsRdpCertSignature::InitializeSignature`).
pub(crate) fn decode_signature(records: &RdpRecords) -> Option<Vec<u8>> {
    let Some(RdpRecordValue::Str(encoded)) = records.get("signature") else {
        return None;
    };
    let encoded = encoded
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    let blob = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    if blob.len() < SIGNATURE_HEADER_LEN || blob[0] != 1 {
        return None;
    }
    let signature_type = u16::from_le_bytes([blob[2], blob[3]]);
    let flags = u32::from_le_bytes(blob[4..8].try_into().ok()?);
    let length = u32::from_le_bytes(blob[8..12].try_into().ok()?) as usize;
    if signature_type != SIGNATURE_TYPE_CERTIFICATE
        || flags & 1 == 0
        || length == 0
        || blob.len() - SIGNATURE_HEADER_LEN < length
    {
        return None;
    }
    Some(blob[SIGNATURE_HEADER_LEN..SIGNATURE_HEADER_LEN + length].to_vec())
}

/// Classifies the publisher of an `.rdp` file the way the Windows client does.
pub(crate) fn verify_rdp_signature(records: &RdpRecords, policy: &SignaturePolicy) -> SignatureVerification {
    let Some(pkcs7) = decode_signature(records) else {
        return SignatureVerification {
            status: PublisherStatus::User,
            chain: None,
        };
    };
    let Some(blob) = secure_settings_blob(records) else {
        // A missing scope fails with ERROR_NOT_FOUND and a scope mismatch with ERROR_INVALID_DATA.
        let flags = match records.get("signscope") {
            Some(RdpRecordValue::Str(scope)) if !scope.is_empty() => FLAG_TAMPERED,
            _ => FLAG_MISSING_SCOPE,
        };
        return SignatureVerification {
            status: PublisherStatus::from_failure_flags(flags),
            chain: None,
        };
    };
    match verify_certificate_signature(&pkcs7, &blob, policy.revocation_check) {
        Ok(chain) => {
            let trusted = policy
                .trusted_thumbprints
                .iter()
                .any(|thumbprint| certificate_matches(chain.leaf_certificate(), thumbprint));
            SignatureVerification {
                status: if trusted {
                    PublisherStatus::Trusted
                } else {
                    PublisherStatus::ThirdParty
                },
                chain: Some(chain),
            }
        }
        Err(flags) => SignatureVerification {
            status: PublisherStatus::from_failure_flags(flags),
            chain: None,
        },
    }
}

const ENCODING: CERT_QUERY_ENCODING_TYPE = CERT_QUERY_ENCODING_TYPE(X509_ASN_ENCODING.0 | PKCS_7_ASN_ENCODING.0);

fn verify_certificate_signature(pkcs7: &[u8], blob: &[u8], revocation_check: bool) -> Result<PublisherChain, u32> {
    let para = CRYPT_VERIFY_MESSAGE_PARA {
        cbSize: size_of::<CRYPT_VERIFY_MESSAGE_PARA>() as u32,
        dwMsgAndCertEncodingType: ENCODING.0,
        ..Default::default()
    };
    let pointers = [blob.as_ptr()];
    let lengths = [u32::try_from(blob.len()).map_err(|_| FLAG_OTHER)?];
    let mut signer: *mut CERT_CONTEXT = core::ptr::null_mut();
    let verified = unsafe {
        CryptVerifyDetachedMessageSignature(
            &para,
            0,
            pkcs7,
            1,
            pointers.as_ptr(),
            lengths.as_ptr(),
            Some(&mut signer),
        )
    };
    if let Err(error) = verified {
        if !signer.is_null() {
            unsafe {
                let _ = CertFreeCertificateContext(Some(signer));
            }
        }
        // The Windows client treats only NTE_BAD_SIGNATURE as tampering. CNG-backed verification
        // reports a mismatch as STATUS_INVALID_SIGNATURE, which that check would downgrade to an
        // unverifiable publisher, so IronRDP treats every signature mismatch as tampering.
        let tampered = [
            NTE_BAD_SIGNATURE,
            windows_core::HRESULT(STATUS_INVALID_SIGNATURE.0),
            CRYPT_E_HASH_VALUE,
        ]
        .contains(&error.code());
        return Err(if tampered { FLAG_TAMPERED } else { FLAG_OTHER });
    }
    let result = validate_signer(signer, pkcs7, revocation_check);
    unsafe {
        let _ = CertFreeCertificateContext(Some(signer));
    }
    result
}

fn validate_signer(signer: *const CERT_CONTEXT, pkcs7: &[u8], revocation_check: bool) -> Result<PublisherChain, u32> {
    if !has_rdp_signing_usage(signer) {
        return Err(FLAG_WRONG_USAGE);
    }
    if unsafe { CertVerifyTimeValidity(None::<*const FILETIME>, (*signer).pCertInfo) } != 0 {
        return Err(FLAG_NOT_TIME_VALID);
    }

    let message = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(pkcs7.len()).map_err(|_| FLAG_OTHER)?,
        pbData: pkcs7.as_ptr().cast_mut(),
    };
    let store = unsafe {
        CertOpenStore(
            CERT_STORE_PROV_PKCS7,
            ENCODING,
            None,
            CERT_OPEN_STORE_FLAGS(0),
            Some((&raw const message).cast()),
        )
    }
    .map_err(|_| FLAG_OTHER)?;
    let chain_flags = if revocation_check {
        CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT
    } else {
        0
    };
    let para = CERT_CHAIN_PARA {
        cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
        ..Default::default()
    };
    let mut chain = None;
    // HCCE_CURRENT_USER, then HCCE_LOCAL_MACHINE.
    for engine in [
        HCERTCHAINENGINE(core::ptr::null_mut()),
        HCERTCHAINENGINE(core::ptr::without_provenance_mut(1)),
    ] {
        let mut context: *mut CERT_CHAIN_CONTEXT = core::ptr::null_mut();
        let built = unsafe {
            CertGetCertificateChain(
                Some(engine),
                signer,
                None,
                Some(store),
                &para,
                chain_flags,
                None,
                &mut context,
            )
        };
        let Some(built) = built.ok().and_then(|()| unsafe { PublisherChain::from_owned(context) }) else {
            continue;
        };
        let clean = unsafe { (*built.as_ptr()).TrustStatus.dwErrorStatus } == 0;
        chain = Some(built);
        if clean {
            break;
        }
    }
    unsafe {
        let _ = CertCloseStore(Some(store), 0);
    }
    let chain = chain.ok_or(FLAG_OTHER)?;

    let mut flags = chain_status_flags(unsafe { (*chain.as_ptr()).TrustStatus.dwErrorStatus });
    let policy_para = CERT_CHAIN_POLICY_PARA {
        cbSize: size_of::<CERT_CHAIN_POLICY_PARA>() as u32,
        // CERT_CHAIN_POLICY_IGNORE_ALL_REV_UNKNOWN_FLAGS unless revocation is checked.
        dwFlags: CERT_CHAIN_POLICY_FLAGS(if revocation_check { 0 } else { 0xF00 }),
        ..Default::default()
    };
    let mut status = CERT_CHAIN_POLICY_STATUS {
        cbSize: size_of::<CERT_CHAIN_POLICY_STATUS>() as u32,
        ..Default::default()
    };
    let policy_ok = unsafe {
        CertVerifyCertificateChainPolicy(
            CERT_CHAIN_POLICY_AUTHENTICODE,
            chain.as_ptr(),
            &policy_para,
            &mut status,
        )
    }
    .as_bool();
    flags |= if policy_ok {
        policy_error_flags(status.dwError)
    } else {
        FLAG_OTHER
    };
    if flags == 0 { Ok(chain) } else { Err(flags) }
}

fn has_rdp_signing_usage(certificate: *const CERT_CONTEXT) -> bool {
    let mut size = 0u32;
    if unsafe { CertGetEnhancedKeyUsage(certificate, CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0, None, &mut size) }.is_err()
    {
        return false;
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    let usage = buffer.as_mut_ptr().cast::<CTL_USAGE>();
    if unsafe {
        CertGetEnhancedKeyUsage(
            certificate,
            CERT_FIND_EXT_ONLY_ENHKEY_USAGE_FLAG.0,
            Some(usage),
            &mut size,
        )
    }
    .is_err()
    {
        return false;
    }
    let usage = unsafe { &*usage };
    (0..usage.cUsageIdentifier as usize).any(|index| {
        let oid = unsafe { (*usage.rgpszUsageIdentifier.add(index)).to_string() };
        oid.is_ok_and(|oid| RDP_SIGNING_EKUS.contains(&oid.as_str()))
    })
}

fn chain_status_flags(status: u32) -> u32 {
    let mut flags = 0;
    let mut remaining = status;
    for (bit, flag) in [
        (CERT_TRUST_IS_NOT_TIME_VALID, FLAG_NOT_TIME_VALID),
        (CERT_TRUST_IS_REVOKED, FLAG_REVOKED),
        (CERT_TRUST_IS_NOT_SIGNATURE_VALID, FLAG_BAD_CERTIFICATE_SIGNATURE),
        (CERT_TRUST_IS_UNTRUSTED_ROOT, FLAG_UNTRUSTED_ROOT),
        (CERT_TRUST_IS_OFFLINE_REVOCATION, FLAG_REVOCATION_UNKNOWN),
        (CERT_TRUST_REVOCATION_STATUS_UNKNOWN, FLAG_REVOCATION_UNKNOWN),
    ] {
        if status & bit != 0 {
            flags |= flag;
            remaining &= !bit;
        }
    }
    if remaining != 0 {
        flags |= FLAG_OTHER;
    }
    flags
}

fn policy_error_flags(error: u32) -> u32 {
    use windows::Win32::Foundation::{
        CERT_E_EXPIRED, CERT_E_REVOCATION_FAILURE, CERT_E_REVOKED, CERT_E_UNTRUSTEDROOT, CERT_E_WRONG_USAGE,
        CRYPT_E_REVOCATION_OFFLINE, CRYPT_E_REVOKED, TRUST_E_CERT_SIGNATURE,
    };

    if error == ERROR_SUCCESS.0 {
        return 0;
    }
    let error = windows_core::HRESULT(error as i32);
    if error == CERT_E_EXPIRED {
        FLAG_NOT_TIME_VALID
    } else if error == CRYPT_E_REVOKED || error == CERT_E_REVOKED {
        FLAG_REVOKED
    } else if error == CRYPT_E_REVOCATION_OFFLINE || error == CERT_E_REVOCATION_FAILURE {
        FLAG_REVOCATION_UNKNOWN
    } else if error == TRUST_E_CERT_SIGNATURE {
        FLAG_BAD_CERTIFICATE_SIGNATURE
    } else if error == CERT_E_UNTRUSTEDROOT {
        FLAG_UNTRUSTED_ROOT
    } else if error == CERT_E_WRONG_USAGE {
        FLAG_WRONG_USAGE
    } else {
        FLAG_OTHER
    }
}

fn certificate_matches(certificate: *const CERT_CONTEXT, thumbprint: &Thumbprint) -> bool {
    let encoded =
        unsafe { core::slice::from_raw_parts((*certificate).pbCertEncoded, (*certificate).cbCertEncoded as usize) };
    let mut digest = vec![0u8; thumbprint.algorithm.digest_len()];
    let mut length = digest.len() as u32;
    let algorithm = HSTRING::from(thumbprint.algorithm.cng_name());
    let hashed = unsafe {
        CryptHashCertificate2(
            PCWSTR(algorithm.as_ptr()),
            0,
            None,
            Some(encoded),
            Some(digest.as_mut_ptr()),
            &mut length,
        )
    };
    hashed.is_ok() && length as usize == digest.len() && digest == thumbprint.digest
}

/// Reads a Terminal Services policy value from HKLM, then HKCU.
pub(crate) fn policy_dword(name: &str) -> Option<u32> {
    machine_policy(|root| registry_dword(root, TERMINAL_SERVICES_POLICY_KEY, name))
}

fn policy_string(name: &str) -> Option<String> {
    machine_policy(|root| registry_string(root, TERMINAL_SERVICES_POLICY_KEY, name))
}

/// Reads a Terminal Services policy value, where machine policy overrides user policy, as mstscax does.
fn machine_policy<T>(lookup: impl Fn(HKEY) -> Option<T>) -> Option<T> {
    lookup(HKEY_LOCAL_MACHINE).or_else(|| lookup(HKEY_CURRENT_USER))
}

/// Reads a Terminal Server Client setting, where the user's value overrides the machine's, as mstscax does.
fn user_setting<T>(lookup: impl Fn(HKEY) -> Option<T>) -> Option<T> {
    lookup(HKEY_CURRENT_USER).or_else(|| lookup(HKEY_LOCAL_MACHINE))
}

fn registry_dword(root: HKEY, key: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let key = HSTRING::from(key);
    let name = HSTRING::from(name);
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(value)
}

fn registry_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let key = HSTRING::from(key);
    let name = HSTRING::from(name);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    String::from_utf16(&buffer[..length]).ok()
}

/// An `.rdp` file split into its connection settings and its signature classification.
#[derive(Debug)]
pub(crate) struct SignedRdpFile {
    /// The file without the `signscope`, `signature`, and `showpublisherwarningdialog` records.
    pub(crate) settings: String,
    pub(crate) verification: SignatureVerification,
    /// `ShowPublisherWarningDialog`, which defaults to enabled.
    pub(crate) show_publisher_warning: bool,
}

/// Verifies an `.rdp` file before its settings are applied. Files that group policy blocks for
/// their publisher classification, including every file whose signature does not match, return
/// `E_ACCESSDENIED`.
pub(crate) fn prepare_rdp_file(contents: &str, policy: &SignaturePolicy) -> windows_core::Result<SignedRdpFile> {
    let records = RdpRecords::parse(contents);
    let verification = verify_rdp_signature(&records, policy);
    if !signature_policy_allows(verification.status, policy) {
        return Err(windows_core::Error::from_hresult(
            windows::Win32::Foundation::E_ACCESSDENIED,
        ));
    }
    let show_publisher_warning = !matches!(records.get("showpublisherwarningdialog"), Some(RdpRecordValue::Int(0)));
    // Apply the same case-insensitively merged records that were verified, so a duplicate that differs
    // only in case cannot replace a signed value.
    let mut records = records;
    records.remove_where(|name| {
        SIGNATURE_RECORDS
            .iter()
            .chain(core::iter::once(&"showpublisherwarningdialog"))
            .any(|record| name.eq_ignore_ascii_case(record))
    });
    // Like mstsc, ignore binary records and lines it cannot parse instead of refusing the file; the
    // signature above already covered them.
    records.retain_typed_text();
    let settings = records.serialize().replace("\r\n", "\n");
    Ok(SignedRdpFile {
        settings,
        verification,
        show_publisher_warning,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use windows::Win32::Foundation::SYSTEMTIME;
    use windows::Win32::Security::Cryptography::{
        BCRYPT_RSA_ALGORITHM, CERT_CREATE_SELFSIGN_NO_KEY_INFO, CERT_EXTENSION, CERT_EXTENSIONS, CERT_KEY_CONTEXT,
        CERT_KEY_CONTEXT_0, CERT_KEY_CONTEXT_PROP_ID, CERT_KEY_SPEC, CERT_NCRYPT_KEY_SPEC, CERT_X500_NAME_STR,
        CRYPT_ALGORITHM_IDENTIFIER, CRYPT_SIGN_MESSAGE_PARA, CertCreateSelfSignCertificate,
        CertSetCertificateContextProperty, CertStrToNameW, CryptSignMessage, HCRYPTPROV_OR_NCRYPT_KEY_HANDLE,
        MS_KEY_STORAGE_PROVIDER, NCRYPT_FLAGS, NCRYPT_HANDLE, NCRYPT_KEY_HANDLE, NCRYPT_PROV_HANDLE,
        NCryptCreatePersistedKey, NCryptFinalizeKey, NCryptFreeObject, NCryptOpenStorageProvider,
    };
    use windows::Win32::System::SystemInformation::GetSystemTime;
    use windows_core::{PSTR, w};

    use super::*;

    pub(crate) const CODE_SIGNING_EKU: &[u8] =
        &[0x30, 0x0A, 0x06, 0x08, 0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x03];
    const CLIENT_AUTH_EKU: &[u8] = &[0x30, 0x0A, 0x06, 0x08, 0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02];

    /// A self-signed RSA certificate whose private key lives only in memory.
    pub(crate) struct TestSigner {
        provider: NCRYPT_PROV_HANDLE,
        key: NCRYPT_KEY_HANDLE,
        pub(crate) certificate: *mut CERT_CONTEXT,
    }

    impl TestSigner {
        pub(crate) fn new(eku: &[u8], expired: bool) -> Self {
            let mut provider = NCRYPT_PROV_HANDLE::default();
            let mut key = NCRYPT_KEY_HANDLE::default();
            unsafe {
                NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0).unwrap();
                NCryptCreatePersistedKey(
                    provider,
                    &mut key,
                    BCRYPT_RSA_ALGORITHM,
                    PCWSTR::null(),
                    CERT_KEY_SPEC(0),
                    NCRYPT_FLAGS(0),
                )
                .unwrap();
                NCryptFinalizeKey(key, NCRYPT_FLAGS(0)).unwrap();
            }

            let mut subject = vec![0u8; 256];
            let mut subject_len = subject.len() as u32;
            unsafe {
                CertStrToNameW(
                    X509_ASN_ENCODING,
                    w!("CN=IronRDP test publisher"),
                    CERT_X500_NAME_STR,
                    None,
                    Some(subject.as_mut_ptr()),
                    &mut subject_len,
                    None,
                )
                .unwrap();
            }
            let subject = CRYPT_INTEGER_BLOB {
                cbData: subject_len,
                pbData: subject.as_mut_ptr(),
            };
            let mut eku = eku.to_vec();
            let mut extension = CERT_EXTENSION {
                pszObjId: PSTR(c"2.5.29.37".as_ptr().cast_mut().cast()),
                fCritical: false.into(),
                Value: CRYPT_INTEGER_BLOB {
                    cbData: eku.len() as u32,
                    pbData: eku.as_mut_ptr(),
                },
            };
            let extensions = CERT_EXTENSIONS {
                cExtension: 1,
                rgExtension: &mut extension,
            };
            let now = unsafe { GetSystemTime() };
            let years = |offset: i16| SYSTEMTIME {
                wYear: now.wYear.wrapping_add_signed(offset),
                wDay: now.wDay.min(28),
                ..now
            };
            let (start, end) = if expired {
                (years(-3), years(-1))
            } else {
                (years(-1), years(1))
            };
            let algorithm = CRYPT_ALGORITHM_IDENTIFIER {
                pszObjId: PSTR(c"1.2.840.113549.1.1.11".as_ptr().cast_mut().cast()),
                ..Default::default()
            };
            let certificate = unsafe {
                CertCreateSelfSignCertificate(
                    Some(HCRYPTPROV_OR_NCRYPT_KEY_HANDLE(key.0)),
                    &subject,
                    // An ephemeral key has no provider name to record in the certificate.
                    CERT_CREATE_SELFSIGN_NO_KEY_INFO,
                    None,
                    Some(&algorithm),
                    Some(&start),
                    Some(&end),
                    Some(&extensions),
                )
            };
            assert!(
                !certificate.is_null(),
                "create self-signed certificate: {:?}",
                windows_core::Error::from_thread()
            );
            let key_context = CERT_KEY_CONTEXT {
                cbSize: size_of::<CERT_KEY_CONTEXT>() as u32,
                Anonymous: CERT_KEY_CONTEXT_0 { hNCryptKey: key },
                dwKeySpec: CERT_NCRYPT_KEY_SPEC.0,
            };
            unsafe {
                // CERT_STORE_NO_CRYPT_RELEASE_FLAG: `Drop` releases the key, not the certificate.
                CertSetCertificateContextProperty(
                    certificate,
                    CERT_KEY_CONTEXT_PROP_ID,
                    1,
                    Some((&raw const key_context).cast()),
                )
                .unwrap();
            }
            Self {
                provider,
                key,
                certificate,
            }
        }

        /// Signs the settings like mstsc and returns the complete file.
        pub(crate) fn sign(&self, unsigned: &str, scope: &str) -> String {
            let mut contents = format!("{unsigned}signscope:s:{scope}\r\n");
            let blob = secure_settings_blob(&RdpRecords::parse(&contents)).expect("signable settings");
            let mut certificates = [self.certificate];
            let para = CRYPT_SIGN_MESSAGE_PARA {
                cbSize: size_of::<CRYPT_SIGN_MESSAGE_PARA>() as u32,
                dwMsgEncodingType: ENCODING.0,
                pSigningCert: self.certificate,
                HashAlgorithm: CRYPT_ALGORITHM_IDENTIFIER {
                    pszObjId: PSTR(c"2.16.840.1.101.3.4.2.1".as_ptr().cast_mut().cast()),
                    ..Default::default()
                },
                cMsgCert: 1,
                rgpMsgCert: certificates.as_mut_ptr(),
                ..Default::default()
            };
            let pointers = [blob.as_ptr()];
            let lengths = [blob.len() as u32];
            let mut size = 0u32;
            unsafe {
                CryptSignMessage(
                    &para,
                    true,
                    1,
                    Some(pointers.as_ptr()),
                    lengths.as_ptr(),
                    None,
                    &mut size,
                )
                .unwrap();
            }
            let mut pkcs7 = vec![0u8; size as usize];
            unsafe {
                CryptSignMessage(
                    &para,
                    true,
                    1,
                    Some(pointers.as_ptr()),
                    lengths.as_ptr(),
                    Some(pkcs7.as_mut_ptr()),
                    &mut size,
                )
                .unwrap();
            }
            pkcs7.truncate(size as usize);
            let mut signature = vec![0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00];
            signature.extend_from_slice(&size.to_le_bytes());
            signature.extend_from_slice(&pkcs7);
            let encoded = base64::engine::general_purpose::STANDARD.encode(signature);
            // mstsc separates the 64-character Base64 lines with spaces.
            let wrapped = encoded
                .as_bytes()
                .chunks(64)
                .map(|chunk| core::str::from_utf8(chunk).expect("Base64 is ASCII"))
                .collect::<Vec<_>>()
                .join("  ");
            contents.push_str(&format!("signature:s:{wrapped}  \r\n"));
            contents
        }
    }

    impl Drop for TestSigner {
        fn drop(&mut self) {
            unsafe {
                let _ = CertFreeCertificateContext(Some(self.certificate));
                let _ = NCryptFreeObject(NCRYPT_HANDLE(self.key.0));
                let _ = NCryptFreeObject(NCRYPT_HANDLE(self.provider.0));
            }
        }
    }

    pub(crate) const UNSIGNED: &str = "full address:s:host.example\r\nserver port:i:3389\r\nscreen mode id:i:2\r\n";
    pub(crate) const SCOPE: &str = "Full Address,Server Port";

    fn classify(contents: &str) -> SignatureVerification {
        verify_rdp_signature(&RdpRecords::parse(contents), &SignaturePolicy::default())
    }

    #[test]
    fn signed_bytes_follow_scope_order_and_end_with_signscope_and_nul() {
        // oracle: reference mstscax.dll 10.0.26100.9711 CTsRdpCertSignature signed-settings blob
        let records = RdpRecords::parse(
            "Server Port:i:3389\r\nfull address:s:host  \r\nsignscope:s:Full Address,Server Port\r\n",
        );
        let expected = "full address:s:host  \r\nserver port:i:3389\r\nsignscope:s:Full Address,Server Port\r\n\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(secure_settings_blob(&records), Some(expected));
    }

    #[test]
    fn signed_bytes_require_every_present_sensitive_setting() {
        // Server Port is sensitive, so leaving it out of the scope invalidates the file.
        let records = RdpRecords::parse("full address:s:host\r\nserver port:i:3389\r\nsignscope:s:Full Address\r\n");
        assert_eq!(secure_settings_blob(&records), None);
        // Scope tokens are not trimmed.
        let records =
            RdpRecords::parse("full address:s:host\r\nserver port:i:1\r\nsignscope:s:Full Address, Server Port\r\n");
        assert_eq!(secure_settings_blob(&records), None);
        // Full Address is mandatory.
        let records = RdpRecords::parse("username:s:a\r\nsignscope:s:username\r\n");
        assert_eq!(secure_settings_blob(&records), None);
        // Settings outside the sensitive list may be signed too.
        let records = RdpRecords::parse("full address:s:h\r\nusername:s:a\r\nsignscope:s:Full Address,username\r\n");
        assert!(secure_settings_blob(&records).is_some());
    }

    // oracle: reference mstscax.dll 10.0.26100.9711 CTsRdpCertSignature::InitializeSignature 12-byte header checks
    #[test]
    fn signature_header_must_describe_a_certificate_signature() {
        let decode = |bytes: &[u8]| {
            let text = format!(
                "signature:s:{}\r\n",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            );
            decode_signature(&RdpRecords::parse(&text))
        };
        assert_eq!(
            decode(&[1, 0, 1, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0x30, 0x80, 9]),
            Some(vec![0x30, 0x80])
        );
        assert_eq!(
            decode(&[2, 0, 1, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0x30, 0x80]),
            None,
            "version"
        );
        assert_eq!(
            decode(&[1, 0, 2, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0x30, 0x80]),
            None,
            "user signature"
        );
        assert_eq!(
            decode(&[1, 0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0x30, 0x80]),
            None,
            "signature flag"
        );
        assert_eq!(
            decode(&[1, 0, 1, 0, 1, 0, 0, 0, 3, 0, 0, 0, 0x30, 0x80]),
            None,
            "length"
        );
        assert_eq!(decode(&[1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0]), None, "empty");
    }

    #[test]
    fn registry_hives_follow_mstscax_precedence() {
        // oracle: reference mstscax.dll 10.0.26100.9711 policy and setting reads
        // mstscax reads Terminal Services policy from HKLM before HKCU, and
        // RdpSignCertChainRevocationCheck from HKCU before HKLM.
        let both = |root: HKEY| Some(if root == HKEY_LOCAL_MACHINE { "machine" } else { "user" });
        assert_eq!(machine_policy(both), Some("machine"));
        assert_eq!(user_setting(both), Some("user"));
        let user_only = |root: HKEY| (root == HKEY_CURRENT_USER).then_some("user");
        assert_eq!(machine_policy(user_only), Some("user"));
        let machine_only = |root: HKEY| (root == HKEY_LOCAL_MACHINE).then_some("machine");
        assert_eq!(user_setting(machine_only), Some("machine"));
    }

    #[test]
    fn signature_policy_values_default_to_allowing_files() {
        let none = SignaturePolicy::from_values(|_| None, |_| None, None);
        assert!(none.allow_unsigned_files && none.allow_signed_files && !none.revocation_check);
        assert!(none.trusted_thumbprints.is_empty());

        let restrictive = SignaturePolicy::from_values(
            |name| matches!(name, "AllowUnsignedFiles" | "AllowSignedFiles").then_some(0),
            |_| None,
            Some(1),
        );
        assert!(
            !restrictive.allow_unsigned_files,
            "AllowUnsignedFiles=0 refuses unsigned files"
        );
        assert!(
            !restrictive.allow_signed_files,
            "AllowSignedFiles=0 refuses signed files"
        );
        assert!(
            restrictive.revocation_check,
            "RdpSignCertChainRevocationCheck=1 enables revocation"
        );
        assert!(!SignaturePolicy::from_values(|_| None, |_| None, Some(2)).revocation_check);
    }

    #[test]
    fn failure_flags_classify_like_load_publisher_category() {
        // oracle: reference mstscax.dll 10.0.26100.9711 LoadPublisherCategory
        // It blocks every
        // certificate-class failure (0x1000 set) except an expired certificate alone (0x1002).
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_REVOCATION_UNKNOWN),
            PublisherStatus::Blocked,
            "an unknown revocation status is blocked, as in mstscax"
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_TAMPERED),
            PublisherStatus::Blocked
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_NOT_TIME_VALID),
            PublisherStatus::Unsigned
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_NOT_TIME_VALID | FLAG_REVOKED),
            PublisherStatus::Blocked
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_WRONG_USAGE),
            PublisherStatus::Blocked
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_UNTRUSTED_ROOT),
            PublisherStatus::Unknown
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_OTHER),
            PublisherStatus::Unknown
        );
        assert_eq!(
            PublisherStatus::from_failure_flags(FLAG_MISSING_SCOPE),
            PublisherStatus::Unsigned
        );
    }

    #[test]
    fn publisher_status_maps_to_warning_type_and_policy() {
        assert_eq!(
            PublisherStatus::User.redirection_warning_type(),
            RedirectionWarningType::User
        );
        assert_eq!(
            PublisherStatus::Blocked.redirection_warning_type(),
            RedirectionWarningType::Unsigned
        );
        assert_eq!(
            PublisherStatus::Trusted.redirection_warning_type(),
            RedirectionWarningType::Trusted
        );

        let blocking = SignaturePolicy {
            allow_unsigned_files: false,
            allow_signed_files: false,
            ..SignaturePolicy::default()
        };
        assert!(signature_policy_allows(
            PublisherStatus::User,
            &SignaturePolicy::default()
        ));
        assert!(!signature_policy_allows(
            PublisherStatus::Blocked,
            &SignaturePolicy::default()
        ));
        assert!(!signature_policy_allows(PublisherStatus::Unknown, &blocking));
        assert!(!signature_policy_allows(PublisherStatus::ThirdParty, &blocking));
        assert!(signature_policy_allows(PublisherStatus::Trusted, &blocking));
    }

    #[test]
    fn trusted_thumbprints_parse_prefixes_and_ignore_bad_entries() {
        let sha1 = "00".repeat(20);
        let sha256 = "AB".repeat(32);
        let parsed = parse_trusted_thumbprints(&format!("\u{200E}{sha1}, sha256: {sha256} ,sha384:00,zz"), false);
        assert_eq!(
            parsed,
            vec![
                Thumbprint {
                    algorithm: ThumbprintAlgorithm::Sha1,
                    digest: vec![0; 20],
                },
                Thumbprint {
                    algorithm: ThumbprintAlgorithm::Sha256,
                    digest: vec![0xAB; 32],
                },
            ]
        );
        assert_eq!(parse_trusted_thumbprints(&sha1, true), Vec::new());
    }

    #[test]
    fn unsigned_file_is_classified_as_user() {
        let verification = classify(UNSIGNED);
        assert_eq!(verification.status, PublisherStatus::User);
        assert!(verification.chain.is_none());
    }

    #[test]
    fn valid_signature_from_untrusted_root_is_unknown() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let verification = classify(&signer.sign(UNSIGNED, SCOPE));
        assert_eq!(verification.status, PublisherStatus::Unknown);
        assert!(verification.chain.is_none());
    }

    #[test]
    fn changed_signed_setting_is_blocked() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let tampered = signer.sign(UNSIGNED, SCOPE).replace("host.example", "attacker.example");
        assert_eq!(classify(&tampered).status, PublisherStatus::Blocked);

        let unscoped = format!("{}redirectdrives:i:1\r\n", signer.sign(UNSIGNED, SCOPE));
        assert_eq!(classify(&unscoped).status, PublisherStatus::Blocked);
    }

    #[test]
    fn unsigned_settings_outside_the_scope_keep_the_signature_valid() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let extended = format!("{}username:s:someone\r\n", signer.sign(UNSIGNED, SCOPE));
        assert_eq!(classify(&extended).status, PublisherStatus::Unknown);
    }

    #[test]
    fn signer_without_rdp_signing_usage_is_blocked() {
        let signer = TestSigner::new(CLIENT_AUTH_EKU, false);
        assert_eq!(classify(&signer.sign(UNSIGNED, SCOPE)).status, PublisherStatus::Blocked);
    }

    #[test]
    fn expired_signer_is_treated_as_unsigned() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, true);
        assert_eq!(
            classify(&signer.sign(UNSIGNED, SCOPE)).status,
            PublisherStatus::Unsigned
        );
    }

    #[test]
    fn thumbprint_matches_the_leaf_certificate_encoding() {
        use sha2::{Digest as _, Sha256};

        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let encoded = unsafe {
            core::slice::from_raw_parts(
                (*signer.certificate).pbCertEncoded,
                (*signer.certificate).cbCertEncoded as usize,
            )
        };
        let mut thumbprint = Thumbprint {
            algorithm: ThumbprintAlgorithm::Sha256,
            digest: Sha256::digest(encoded).to_vec(),
        };
        assert!(certificate_matches(signer.certificate, &thumbprint));
        thumbprint.digest[0] ^= 1;
        assert!(!certificate_matches(signer.certificate, &thumbprint));
    }

    #[test]
    fn prepare_rdp_file_strips_signature_records_and_reads_publisher_switch() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let contents = format!("{}showpublisherwarningdialog:i:0\r\n", signer.sign(UNSIGNED, SCOPE));
        let prepared = prepare_rdp_file(&contents, &SignaturePolicy::default()).unwrap();
        assert_eq!(prepared.settings, UNSIGNED.replace("\r\n", "\n"));
        assert!(!prepared.show_publisher_warning);
        assert_eq!(prepared.verification.status, PublisherStatus::Unknown);

        let tampered = signer.sign(UNSIGNED, SCOPE).replace("3389", "3390");
        assert_eq!(
            prepare_rdp_file(&tampered, &SignaturePolicy::default())
                .unwrap_err()
                .code(),
            windows::Win32::Foundation::E_ACCESSDENIED
        );
    }

    #[test]
    fn prepare_rdp_file_drops_binary_records_and_unparsed_lines() {
        let contents = format!("{UNSIGNED}session id:b:0102\r\nnot a record\r\n");
        let prepared = prepare_rdp_file(&contents, &SignaturePolicy::default()).unwrap();
        assert_eq!(prepared.settings, UNSIGNED.replace("\r\n", "\n"));
    }

    #[test]
    fn prepare_rdp_file_applies_the_verified_value_of_a_case_differing_duplicate() {
        let signer = TestSigner::new(CODE_SIGNING_EKU, false);
        let contents = format!(
            "{}full address:s:attacker.example\r\nFULL ADDRESS:s:host.example\r\n",
            signer.sign(UNSIGNED, SCOPE)
        );
        let prepared = prepare_rdp_file(&contents, &SignaturePolicy::default()).unwrap();
        assert_eq!(prepared.verification.status, PublisherStatus::Unknown);
        assert!(prepared.settings.contains("full address:s:host.example\n"));
        assert!(!prepared.settings.to_ascii_lowercase().contains("attacker.example"));
        assert_eq!(prepared.settings.matches("full address:").count(), 1);
    }
}

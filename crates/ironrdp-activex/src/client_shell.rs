//! `IMsRdpClientShell`: a standalone `.rdp` settings store exposed by `IMsRdpClient5::MsRdpClientShell`.
//!
//! Microsoft's object keeps its own `.rdp` record list, independent of the control's connection
//! settings: `RdpFileContents`, `GetRdpProperty`, and `SetRdpProperty` only edit that list. Its
//! `Launch` hands the list to a separate `mstsc.exe`; IronRDP instead applies it to the owning
//! control and connects in-process. The store below follows the same record rules (lowercase
//! names, file order, `i`/`s`/`b` types, unparsed lines kept verbatim).

use core::cell::{Cell, RefCell};
use core::ptr;

use windows::Win32::Foundation::{
    DISP_E_BADPARAMCOUNT, DISP_E_MEMBERNOTFOUND, E_ACCESSDENIED, E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER,
    E_UNEXPECTED, S_FALSE,
};
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, EXCEPINFO, IDispatch_Impl,
    ITypeInfo,
};
use windows::Win32::System::Variant::{
    VAR_CHANGE_FLAGS, VARIANT, VT_BOOL, VT_BSTR, VT_I1, VT_I2, VT_I4, VT_INT, VT_UI1, VT_UI2, VT_UI4, VT_UINT,
    VariantChangeType,
};
use windows_core::{Error, GUID, HRESULT, PCWSTR, Result, implement};

use crate::control::{
    Control_Impl, ServerObjectLifetime, dispatch_arguments, dispatch_get_ids, dispatch_params, normalize_variant_bool,
    property_put_value, string_from_bstr, variant_bool_i16, variant_bool_value, variant_bstr_fallible, variant_header,
    variant_i32, variant_string, write_bstr, write_optional_variant, write_out,
};
use crate::mstsc::{Bstr, BstrOut, IMsRdpClientShell, IMsRdpClientShell_Impl};
use crate::rdp_signature::policy_dword;

const DISPID_SHELL_LAUNCH: i32 = 201;
const DISPID_SHELL_RDP_FILE_CONTENTS: i32 = 202;
const DISPID_SHELL_SET_RDP_PROPERTY: i32 = 203;
const DISPID_SHELL_GET_RDP_PROPERTY: i32 = 204;
const DISPID_SHELL_IS_REMOTE_PROGRAM_CLIENT_INSTALLED: i32 = 205;
const DISPID_SHELL_PUBLIC_MODE: i32 = 211;
const DISPID_SHELL_SHOW_TRUSTED_SITES_MANAGEMENT_DIALOG: i32 = 212;

/// `RdpFileContents` must fit, with its terminator, in fewer than 64 KiB of UTF-16.
const MAX_RDP_FILE_CONTENTS_BYTES: usize = 0x1_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RdpRecordValue {
    Int(i32),
    Str(String),
    /// Hexadecimal text of a `b:` record, kept as written.
    Binary(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RdpRecord {
    Typed {
        name: String,
        value: RdpRecordValue,
    },
    /// A line without a recognized `name:type:value` shape, written back unchanged.
    Unparsed(String),
}

/// Ordered `.rdp` records with case-insensitive names, following mstscax's settings store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RdpRecords {
    records: Vec<RdpRecord>,
}

impl RdpRecords {
    pub(crate) fn parse(contents: &str) -> Self {
        let mut records = Self::default();
        for line in contents.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.trim().is_empty() {
                continue;
            }
            match parse_record_line(line) {
                Some((name, value)) => records.insert(name, value),
                None => records.records.push(RdpRecord::Unparsed(line.to_owned())),
            }
        }
        records
    }

    pub(crate) fn serialize(&self) -> String {
        let mut output = String::new();
        for record in &self.records {
            match record {
                RdpRecord::Typed { name, value } => {
                    output.push_str(name);
                    match value {
                        RdpRecordValue::Int(value) => {
                            output.push_str(":i:");
                            output.push_str(&value.to_string());
                        }
                        RdpRecordValue::Str(value) => {
                            output.push_str(":s:");
                            output.push_str(value);
                        }
                        RdpRecordValue::Binary(value) => {
                            output.push_str(":b:");
                            output.push_str(value);
                        }
                    }
                }
                RdpRecord::Unparsed(line) => output.push_str(line),
            }
            output.push_str("\r\n");
        }
        output
    }

    pub(crate) fn get(&self, name: &str) -> Option<&RdpRecordValue> {
        self.records.iter().find_map(|record| match record {
            RdpRecord::Typed { name: candidate, value } if candidate.eq_ignore_ascii_case(name) => Some(value),
            _ => None,
        })
    }

    /// Replaces an existing record in place or appends a new one, like `CRdpSettingsStore::InsertRecord`.
    pub(crate) fn insert(&mut self, name: &str, value: RdpRecordValue) {
        let existing = self.records.iter_mut().find_map(|record| match record {
            RdpRecord::Typed {
                name: candidate,
                value: existing,
            } if candidate.eq_ignore_ascii_case(name) => Some(existing),
            _ => None,
        });
        match existing {
            Some(existing) => *existing = value,
            None => self.records.push(RdpRecord::Typed {
                name: name.to_ascii_lowercase(),
                value,
            }),
        }
    }

    /// Keeps only the integer and string records, the ones IronRDP's settings can apply.
    pub(crate) fn retain_typed_text(&mut self) {
        self.records.retain(|record| {
            matches!(
                record,
                RdpRecord::Typed {
                    value: RdpRecordValue::Int(_) | RdpRecordValue::Str(_),
                    ..
                }
            )
        });
    }

    /// Removes every record, parsed or not, whose name matches `is_removed`.
    pub(crate) fn remove_where(&mut self, mut is_removed: impl FnMut(&str) -> bool) {
        self.records.retain(|record| {
            let name = match record {
                RdpRecord::Typed { name, .. } => name.as_str(),
                RdpRecord::Unparsed(line) => line.trim_start().split(':').next().unwrap_or_default(),
            };
            !is_removed(name)
        });
    }
}

fn parse_record_line(line: &str) -> Option<(&str, RdpRecordValue)> {
    let line = line.trim_start();
    let (name, rest) = line.split_once(':')?;
    let rest = rest.trim_start();
    let mut characters = rest.chars();
    let kind = characters.next()?.to_ascii_lowercase();
    let value = characters.as_str().strip_prefix(':')?;
    let value = match kind {
        'i' => RdpRecordValue::Int(parse_record_int(value)),
        's' => RdpRecordValue::Str(value.to_owned()),
        'b' => {
            if !value.chars().all(|character| character.is_ascii_hexdigit()) || value.len() % 2 != 0 {
                return None;
            }
            RdpRecordValue::Binary(value.to_owned())
        }
        _ => return None,
    };
    Some((name, value))
}

/// Integer values use C `wcstol` base-10 rules: leading whitespace, an optional sign, then digits
/// up to the first other character. Out-of-range values saturate to the 32-bit `LONG` range.
fn parse_record_int(value: &str) -> i32 {
    let value = value.trim_start();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let mut magnitude: i64 = 0;
    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
        magnitude = (magnitude * 10 + i64::from(digit - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let signed = if negative { -magnitude } else { magnitude };
    signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(crate) fn validate_rdp_file_contents(contents: &str) -> Result<()> {
    if contents.is_empty() {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    validate_rdp_file_size(contents)
}

fn validate_rdp_file_size(contents: &str) -> Result<()> {
    if MAX_RDP_FILE_CONTENTS_BYTES <= (contents.encode_utf16().count() + 1) * 2 {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    Ok(())
}

/// Converts a `SetRdpProperty` argument the way mstscax does: integer-like and Boolean values become
/// a 32-bit integer record, strings become string records, and every other type is rejected.
fn record_value_from_variant(value: &VARIANT) -> Result<RdpRecordValue> {
    let vt = variant_header(value).vt;
    if vt == VT_BSTR {
        let value = variant_string(value, ptr::null_mut())?;
        if value.contains(['\r', '\n', '\0']) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        return Ok(RdpRecordValue::Str(value));
    }
    if ![VT_I2, VT_I4, VT_BOOL, VT_I1, VT_UI1, VT_UI2, VT_UI4, VT_INT, VT_UINT].contains(&vt) {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let mut converted = VARIANT::default();
    unsafe { VariantChangeType(&mut converted, value, VAR_CHANGE_FLAGS(0), VT_UI4) }?;
    Ok(RdpRecordValue::Int(
        unsafe { variant_header(&converted).Anonymous.ulVal } as i32,
    ))
}

fn record_value_variant(value: &RdpRecordValue) -> Result<VARIANT> {
    match value {
        RdpRecordValue::Int(value) => Ok(variant_i32(*value)),
        RdpRecordValue::Str(value) => variant_bstr_fallible(value),
        RdpRecordValue::Binary(_) => Err(Error::from_hresult(E_FAIL)),
    }
}

#[implement(IMsRdpClientShell)]
pub(crate) struct ClientShell {
    _lifetime: ServerObjectLifetime,
    records: RefCell<RdpRecords>,
    public_mode: Cell<bool>,
    /// The owning control, cleared by `detach` when that control is destroyed. The control owns this
    /// object, so a counted reference back to it would keep both alive forever.
    control: Cell<*const Control_Impl>,
}

impl ClientShell {
    pub(crate) fn new(control: *const Control_Impl) -> Self {
        Self {
            _lifetime: ServerObjectLifetime::new(),
            records: RefCell::new(RdpRecords::default()),
            public_mode: Cell::new(false),
            control: Cell::new(control),
        }
    }

    pub(crate) fn detach(&self) {
        self.control.set(ptr::null());
    }

    /// Applies the records to the owning control and connects it, where mstscax would start
    /// `mstsc.exe /Web /WebFileName:` with them.
    fn launch(&self) -> Result<()> {
        self.launch_with_policy(policy_dword("DisableClientLaunchingFromActiveXControl"))
    }

    /// Launches unless the `DisableClientLaunchingFromActiveXControl` policy value forbids it.
    ///
    /// A nonzero policy refuses with `E_ACCESSDENIED` before anything is applied, as in mstscax.
    fn launch_with_policy(&self, disable_launch: Option<u32>) -> Result<()> {
        if disable_launch.is_some_and(|value| value != 0) {
            return Err(Error::from_hresult(E_ACCESSDENIED));
        }
        let contents = self.records.borrow().serialize();
        validate_rdp_file_size(&contents)?;
        // SAFETY: the control clears this pointer in its destructor, before its memory is released.
        let control = unsafe { self.control.get().as_ref() }.ok_or_else(|| Error::from_hresult(E_UNEXPECTED))?;
        control.launch_from_client_shell(&contents, self.public_mode.get())
    }

    fn set_rdp_file_contents(&self, contents: &str) -> Result<()> {
        validate_rdp_file_contents(contents)?;
        *self.records.borrow_mut() = RdpRecords::parse(contents);
        Ok(())
    }

    fn rdp_property(&self, name: &str) -> Result<VARIANT> {
        let records = self.records.borrow();
        let value = records.get(name).ok_or_else(|| Error::from_hresult(E_FAIL))?;
        record_value_variant(value)
    }

    fn set_rdp_property(&self, name: &str, value: &VARIANT) -> Result<()> {
        if name.is_empty() || name.contains([':', '\r', '\n', '\0']) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let value = record_value_from_variant(value)?;
        self.records.borrow_mut().insert(name, value);
        Ok(())
    }
}

impl IMsRdpClientShell_Impl for ClientShell_Impl {
    unsafe fn Launch(&self) -> Result<()> {
        self.launch()
    }

    unsafe fn put_RdpFileContents(&self, contents: Bstr) -> Result<()> {
        self.set_rdp_file_contents(&string_from_bstr(contents)?)
    }

    unsafe fn get_RdpFileContents(&self, contents: BstrOut) -> Result<()> {
        write_bstr(contents, &self.records.borrow().serialize())
    }

    unsafe fn SetRdpProperty(&self, name: Bstr, value: VARIANT) -> Result<()> {
        if name.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        self.set_rdp_property(&string_from_bstr(name)?, &value)
    }

    unsafe fn GetRdpProperty(&self, name: Bstr, value: *mut VARIANT) -> Result<()> {
        write_out(value, VARIANT::default())?;
        if name.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        write_out(value, self.rdp_property(&string_from_bstr(name)?)?)
    }

    unsafe fn get_IsRemoteProgramClientInstalled(&self, installed: *mut i16) -> Result<()> {
        write_out(installed, variant_bool_i16(true))
    }

    unsafe fn put_PublicMode(&self, public_mode: i16) -> Result<()> {
        self.public_mode.set(public_mode != 0);
        Ok(())
    }

    unsafe fn get_PublicMode(&self, public_mode: *mut i16) -> Result<()> {
        write_out(public_mode, variant_bool_i16(self.public_mode.get()))
    }

    unsafe fn ShowTrustedSitesManagementDialog(&self) -> HRESULT {
        // mstscax returns S_FALSE without showing UI; IronRDP keeps no trusted-site list either.
        S_FALSE
    }
}

impl IDispatch_Impl for ClientShell_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }

    fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn GetIDsOfNames(
        &self,
        riid: *const GUID,
        names: *const PCWSTR,
        count: u32,
        _lcid: u32,
        dispids: *mut i32,
    ) -> Result<()> {
        dispatch_get_ids(riid, names, count, dispids, |name| {
            match name.to_ascii_lowercase().as_str() {
                "launch" => Some(DISPID_SHELL_LAUNCH),
                "rdpfilecontents" => Some(DISPID_SHELL_RDP_FILE_CONTENTS),
                "setrdpproperty" => Some(DISPID_SHELL_SET_RDP_PROPERTY),
                "getrdpproperty" => Some(DISPID_SHELL_GET_RDP_PROPERTY),
                "isremoteprogramclientinstalled" => Some(DISPID_SHELL_IS_REMOTE_PROGRAM_CLIENT_INSTALLED),
                "publicmode" => Some(DISPID_SHELL_PUBLIC_MODE),
                "showtrustedsitesmanagementdialog" => Some(DISPID_SHELL_SHOW_TRUSTED_SITES_MANAGEMENT_DIALOG),
                _ => None,
            }
        })
    }

    fn Invoke(
        &self,
        dispid: i32,
        riid: *const GUID,
        _lcid: u32,
        flags: DISPATCH_FLAGS,
        params: *const DISPPARAMS,
        result: *mut VARIANT,
        _exception: *mut EXCEPINFO,
        _argument_error: *mut u32,
    ) -> Result<()> {
        if !riid.is_null() && unsafe { *riid } != GUID::zeroed() {
            return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND));
        }
        let params = dispatch_params(params)?;
        if flags.contains(DISPATCH_PROPERTYPUT) {
            let value = property_put_value(params)?;
            return match dispid {
                DISPID_SHELL_RDP_FILE_CONTENTS => self.set_rdp_file_contents(&variant_string(value, ptr::null_mut())?),
                DISPID_SHELL_PUBLIC_MODE => {
                    let header = variant_header(value);
                    if header.vt != VT_BOOL {
                        return Err(Error::from_hresult(windows::Win32::Foundation::DISP_E_TYPEMISMATCH));
                    }
                    self.public_mode
                        .set(normalize_variant_bool(unsafe { header.Anonymous.boolVal }.0)? != 0);
                    Ok(())
                }
                _ => Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND)),
            };
        }
        let arguments = dispatch_arguments(params)?;
        if flags.contains(DISPATCH_PROPERTYGET) && arguments.is_empty() {
            match dispid {
                DISPID_SHELL_RDP_FILE_CONTENTS => {
                    return write_optional_variant(result, variant_bstr_fallible(&self.records.borrow().serialize())?);
                }
                DISPID_SHELL_IS_REMOTE_PROGRAM_CLIENT_INSTALLED => {
                    return write_optional_variant(result, variant_bool_value(true));
                }
                DISPID_SHELL_PUBLIC_MODE => {
                    return write_optional_variant(result, variant_bool_value(self.public_mode.get()));
                }
                _ => {}
            }
        }
        if !flags.contains(DISPATCH_METHOD) {
            return Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND));
        }
        match dispid {
            DISPID_SHELL_LAUNCH if arguments.is_empty() => self.launch(),
            DISPID_SHELL_SET_RDP_PROPERTY if arguments.len() == 2 => {
                self.set_rdp_property(&variant_string(&arguments[1], ptr::null_mut())?, &arguments[0])
            }
            DISPID_SHELL_GET_RDP_PROPERTY if arguments.len() == 1 => write_optional_variant(
                result,
                self.rdp_property(&variant_string(&arguments[0], ptr::null_mut())?)?,
            ),
            DISPID_SHELL_SHOW_TRUSTED_SITES_MANAGEMENT_DIALOG if arguments.is_empty() => Ok(()),
            DISPID_SHELL_LAUNCH
            | DISPID_SHELL_SET_RDP_PROPERTY
            | DISPID_SHELL_GET_RDP_PROPERTY
            | DISPID_SHELL_SHOW_TRUSTED_SITES_MANAGEMENT_DIALOG => Err(Error::from_hresult(DISP_E_BADPARAMCOUNT)),
            _ => Err(Error::from_hresult(DISP_E_MEMBERNOTFOUND)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_follow_mstscax_store_rules() {
        // oracle: reference mstscax.dll 10.0.26100.9711 CRdpSettingsStore record handling
        let records = RdpRecords::parse(
            "Full Address:s:host.example\r\n  screen mode id : i:2\r\nbogus line\r\n\r\nhash:b:0aFF\r\nFULL ADDRESS:s:other\n",
        );
        assert_eq!(
            records.get("full address"),
            Some(&RdpRecordValue::Str("other".to_owned()))
        );
        assert_eq!(records.get("Screen Mode Id "), Some(&RdpRecordValue::Int(2)));
        assert_eq!(records.get("hash"), Some(&RdpRecordValue::Binary("0aFF".to_owned())));
        assert_eq!(records.get("bogus line"), None);
        assert_eq!(
            records.serialize(),
            "full address:s:other\r\nscreen mode id :i:2\r\nbogus line\r\nhash:b:0aFF\r\n"
        );
    }

    #[test]
    fn records_keep_unrecognized_types_and_odd_binary_verbatim() {
        let records = RdpRecords::parse("a:x:1\nb:b:abc\nc:s\n");
        assert!(
            records
                .records
                .iter()
                .all(|record| matches!(record, RdpRecord::Unparsed(_)))
        );
        assert_eq!(records.serialize(), "a:x:1\r\nb:b:abc\r\nc:s\r\n");
    }

    #[test]
    fn integer_records_use_wcstol_rules() {
        // oracle: reference mstscax.dll 10.0.26100.9711 `wcstol` conversion of `i:` records
        assert_eq!(parse_record_int(" 42abc"), 42);
        assert_eq!(parse_record_int("-7"), -7);
        assert_eq!(parse_record_int("+3"), 3);
        assert_eq!(parse_record_int("x"), 0);
        assert_eq!(parse_record_int("2147483648"), i32::MAX);
        assert_eq!(parse_record_int("99999999999"), i32::MAX);
        assert_eq!(parse_record_int("-99999999999"), i32::MIN);
    }

    #[test]
    fn rdp_file_contents_size_matches_mstscax_limit() {
        assert!(validate_rdp_file_contents("").is_err());
        assert!(validate_rdp_file_contents(&"a".repeat(32_766)).is_ok());
        assert!(validate_rdp_file_contents(&"a".repeat(32_767)).is_err());
    }

    #[test]
    fn shell_round_trips_properties_through_its_own_store() {
        let shell = ClientShell::new(ptr::null());
        shell.set_rdp_file_contents("full address:s:host\r\n").unwrap();
        shell.set_rdp_property("Desktop Width", &variant_i32(1280)).unwrap();
        shell
            .set_rdp_property("full address", &variant_bstr_fallible("next").unwrap())
            .unwrap();
        shell
            .set_rdp_property("redirectclipboard", &variant_bool_value(true))
            .unwrap();
        assert_eq!(
            shell.records.borrow().serialize(),
            "full address:s:next\r\ndesktop width:i:1280\r\nredirectclipboard:i:-1\r\n"
        );
        assert_eq!(
            shell.rdp_property("missing").unwrap_err().code(),
            E_FAIL,
            "mstscax reports E_FAIL for an absent property"
        );
        assert_eq!(
            shell
                .set_rdp_property("full address", &variant_bstr_fallible("a\r\nb:s:c").unwrap())
                .unwrap_err()
                .code(),
            E_INVALIDARG
        );
    }

    #[test]
    fn launch_without_a_control_reports_an_unexpected_state() {
        let shell = ClientShell::new(ptr::null());
        shell.set_rdp_file_contents("full address:s:host\r\n").unwrap();
        assert_eq!(shell.launch_with_policy(None).unwrap_err().code(), E_UNEXPECTED);
    }

    #[test]
    fn launch_policy_refuses_before_reaching_the_control() {
        let shell = ClientShell::new(ptr::null());
        shell.set_rdp_file_contents("full address:s:host\r\n").unwrap();
        // A nonzero policy refuses even without a control; zero falls through to the missing control.
        assert_eq!(shell.launch_with_policy(Some(1)).unwrap_err().code(), E_ACCESSDENIED);
        assert_eq!(shell.launch_with_policy(Some(0)).unwrap_err().code(), E_UNEXPECTED);
    }

    #[test]
    fn set_rdp_property_converts_integer_like_variants() {
        let shell = ClientShell::new(ptr::null());
        shell.set_rdp_property("x", &variant_i32(7)).unwrap();
        assert_eq!(shell.records.borrow().get("x"), Some(&RdpRecordValue::Int(7)));
        // OLE converts negative integers and VARIANT_TRUE to UI4 bit patterns, which `%d` prints as -1.
        shell.set_rdp_property("x", &variant_i32(-1)).unwrap();
        assert_eq!(shell.records.borrow().get("x"), Some(&RdpRecordValue::Int(-1)));
        assert_eq!(
            shell.set_rdp_property("x", &VARIANT::default()).unwrap_err().code(),
            E_INVALIDARG
        );
    }

    fn record_line() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::prelude::*;
        prop_oneof![
            ("[A-Za-z][A-Za-z0-9 ]{0,10}", "-?[0-9]{1,12}").prop_map(|(name, value)| format!("{name}:i:{value}")),
            ("[A-Za-z][A-Za-z0-9 ]{0,10}", "[ -~]{0,16}").prop_map(|(name, value)| format!("{name}:s:{value}")),
            ("[A-Za-z][A-Za-z0-9 ]{0,10}", "[0-9A-F]{0,16}").prop_map(|(name, value)| format!("{name}:b:{value}")),
            "[ -~]{0,24}",
        ]
    }

    proptest::proptest! {
        /// Serializing a parsed store and parsing it again yields the same store.
        #[test]
        fn records_round_trip_through_serialization(lines in proptest::collection::vec(record_line(), 0..24)) {
            let records = RdpRecords::parse(&lines.join("\r\n"));
            proptest::prop_assert_eq!(RdpRecords::parse(&records.serialize()), records);
        }

        /// Stands in for a fuzz target: this crate is Windows-only, so the cross-platform fuzz harness
        /// cannot build it. Untrusted `.rdp` text never panics the parser or the signature decoder.
        #[test]
        fn untrusted_rdp_text_never_panics(contents in r"\PC{0,512}", signature in r"\PC{0,256}") {
            let records = RdpRecords::parse(&contents);
            let _ = records.serialize();
            let _ = crate::rdp_signature::decode_signature(&records);
            let _ = crate::rdp_signature::secure_settings_blob(&records);
            let signed = RdpRecords::parse(&format!("signature:s:{signature}\r\nsignscope:s:{signature}\r\n"));
            let _ = crate::rdp_signature::decode_signature(&signed);
            let _ = crate::rdp_signature::secure_settings_blob(&signed);
        }
    }
}

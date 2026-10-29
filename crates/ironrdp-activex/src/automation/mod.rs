//! Late-bound `IDispatch` access to the published MSTSCLib dual interfaces.
//!
//! The control and its settings objects implement the MSTSCLib vtables directly. Late-bound callers reach the same
//! vtable functions through the member tables in [`members`]: `GetIDsOfNames` resolves the published names and
//! parameter names, and `Invoke` coerces each argument to the published Automation type before calling the vtable
//! slot with `DispCallFunc`. No type library is created or registered, so `GetTypeInfo` remains unavailable.

mod members;

use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::{ptr, slice};

use windows::Win32::Foundation::{
    DISP_E_BADPARAMCOUNT, DISP_E_EXCEPTION, DISP_E_MEMBERNOTFOUND, DISP_E_PARAMNOTFOUND, DISP_E_UNKNOWNINTERFACE,
    DISP_E_UNKNOWNNAME, E_POINTER, S_OK,
};
use windows::Win32::System::Com::{
    CC_STDCALL, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPATCH_PROPERTYPUTREF,
    DISPPARAMS, EXCEPINFO, SetErrorInfo,
};
use windows::Win32::System::Ole::{DISPID_PROPERTYPUT, DispCallFunc};
use windows::Win32::System::Variant::{
    VAR_CHANGE_FLAGS, VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_BYREF, VT_DISPATCH,
    VT_ERROR, VT_I2, VT_I4, VT_UI4, VT_UINT, VT_VARIANT, VariantChangeType, VariantCopyInd,
};
use windows::core::{BSTR, GUID, PCWSTR};

const DISPID_UNKNOWN: i32 = -1;
const EXCEPTION_SOURCE: &str = "IronRDP.ActiveX";

/// A published MSTSCLib dual interface whose members are reachable by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Interface {
    /// `IMsRdpClient10` and the earlier `IMsTscAx`/`IMsRdpClient*` versions it extends.
    Client,
    /// `IMsRdpClientAdvancedSettings8` and its earlier versions.
    AdvancedSettings,
    /// `IMsRdpClientSecuredSettings2` and its earlier versions.
    SecuredSettings,
    /// `IMsRdpClientTransportSettings4` and its earlier versions.
    TransportSettings,
    /// `ITSRemoteProgram3` and its earlier versions.
    RemoteProgram,
}

impl Interface {
    fn members(self) -> &'static [Member] {
        match self {
            Self::Client => members::CLIENT,
            Self::AdvancedSettings => members::ADVANCED_SETTINGS,
            Self::SecuredSettings => members::SECURED_SETTINGS,
            Self::TransportSettings => members::TRANSPORT_SETTINGS,
            Self::RemoteProgram => members::REMOTE_PROGRAM,
        }
    }
}

/// The Automation type of a parameter or return value, as published in MSTSCLib.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueType {
    Bool,
    Bstr,
    Dispatch,
    I2,
    I4,
    Ui4,
    Uint,
    /// A `VARIANT*` input, passed through without coercion.
    VariantRef,
}

impl ValueType {
    fn vt(self) -> VARENUM {
        match self {
            Self::Bool => VT_BOOL,
            Self::Bstr => VT_BSTR,
            Self::Dispatch => VT_DISPATCH,
            Self::I2 => VT_I2,
            Self::I4 => VT_I4,
            Self::Ui4 => VT_UI4,
            Self::Uint => VT_UINT,
            Self::VariantRef => VT_VARIANT,
        }
    }
}

#[derive(Debug)]
struct Parameter {
    name: &'static str,
    value_type: ValueType,
}

impl Parameter {
    const fn new(name: &'static str, value_type: ValueType) -> Self {
        Self { name, value_type }
    }
}

#[derive(Debug)]
enum Signature {
    Get(ValueType),
    Put(ValueType),
    Method(&'static [Parameter], Option<ValueType>),
}

/// One published dispatch member and the vtable slot that implements it.
#[derive(Debug)]
pub(crate) struct Member {
    name: &'static str,
    dispid: i32,
    slot: usize,
    signature: Signature,
}

impl Member {
    const fn get(name: &'static str, dispid: i32, slot: usize, value_type: ValueType) -> Self {
        Self {
            name,
            dispid,
            slot,
            signature: Signature::Get(value_type),
        }
    }

    const fn put(name: &'static str, dispid: i32, slot: usize, value_type: ValueType) -> Self {
        Self {
            name,
            dispid,
            slot,
            signature: Signature::Put(value_type),
        }
    }

    const fn method(
        name: &'static str,
        dispid: i32,
        slot: usize,
        parameters: &'static [Parameter],
        result: Option<ValueType>,
    ) -> Self {
        Self {
            name,
            dispid,
            slot,
            signature: Signature::Method(parameters, result),
        }
    }

    fn parameter_dispid(&self, name: &str) -> Option<i32> {
        let Signature::Method(parameters, _) = self.signature else {
            return None;
        };
        parameters
            .iter()
            .position(|parameter| parameter.name.eq_ignore_ascii_case(name))
            .and_then(|index| i32::try_from(index).ok())
    }
}

/// Resolves a member name and its named parameters the way `ITypeInfo::GetIDsOfNames` does.
///
/// Every resolvable name receives its DISPID even when another name is unknown, and unknown names receive
/// `DISPID_UNKNOWN`.
fn ids_of_names(members: &[Member], names: &[impl AsRef<str>], ids: &mut [i32]) -> windows::core::HRESULT {
    ids.fill(DISPID_UNKNOWN);
    let (Some(member_name), Some(member_id)) = (names.first(), ids.first_mut()) else {
        return S_OK;
    };
    let Some(member) = members
        .iter()
        .find(|member| member.name.eq_ignore_ascii_case(member_name.as_ref()))
    else {
        return DISP_E_UNKNOWNNAME;
    };
    *member_id = member.dispid;

    // A property and its accessor pair share a DISPID; only methods declare named parameters.
    let method = members
        .iter()
        .find(|candidate| candidate.dispid == member.dispid && matches!(candidate.signature, Signature::Method(..)));
    let mut status = S_OK;
    for (name, id) in names.iter().zip(ids.iter_mut()).skip(1) {
        match method.and_then(|method| method.parameter_dispid(name.as_ref())) {
            Some(parameter_id) => *id = parameter_id,
            None => status = DISP_E_UNKNOWNNAME,
        }
    }
    status
}

/// `IDispatch::GetIDsOfNames` for an object whose dispatch surface is exactly `interface`.
///
/// # Safety
///
/// `names` must point to `count` readable `PCWSTR` values and `ids` to `count` writable DISPIDs. `iid` must be
/// null or readable.
pub(crate) unsafe fn get_ids_of_names(
    interface: Interface,
    iid: *const GUID,
    names: *const PCWSTR,
    count: u32,
    ids: *mut i32,
) -> windows::core::HRESULT {
    if names.is_null() || ids.is_null() {
        return E_POINTER;
    }
    if !iid.is_null() && unsafe { *iid } != GUID::zeroed() {
        return DISP_E_UNKNOWNINTERFACE;
    }
    let count = count as usize;
    let ids = unsafe { slice::from_raw_parts_mut(ids, count) };
    let names = unsafe { slice::from_raw_parts(names, count) };
    let mut decoded = Vec::with_capacity(count);
    for name in names {
        match unsafe { name.to_string() } {
            Ok(name) => decoded.push(name),
            Err(_) => {
                ids.fill(DISPID_UNKNOWN);
                return DISP_E_UNKNOWNNAME;
            }
        }
    }
    ids_of_names(interface.members(), &decoded, ids)
}

/// Selects the member that `IDispatch::Invoke` targets, following the `ITypeInfo::Invoke` precedence: a put request
/// only matches a property put, while a combined get/method request prefers the property get.
pub(crate) fn member(interface: Interface, dispid: i32, flags: DISPATCH_FLAGS) -> Option<&'static Member> {
    select_member(interface.members(), dispid, flags)
}

fn select_member(members: &[Member], dispid: i32, flags: DISPATCH_FLAGS) -> Option<&Member> {
    let mut candidates = members.iter().filter(|member| member.dispid == dispid);
    if flags.contains(DISPATCH_PROPERTYPUT) || flags.contains(DISPATCH_PROPERTYPUTREF) {
        return candidates.find(|member| matches!(member.signature, Signature::Put(_)));
    }
    // A property get takes precedence when the caller allows both kinds, as for `Invoke` from scripting hosts.
    if flags.contains(DISPATCH_PROPERTYGET)
        && let Some(getter) = members
            .iter()
            .find(|member| member.dispid == dispid && matches!(member.signature, Signature::Get(_)))
    {
        return Some(getter);
    }
    if flags.contains(DISPATCH_METHOD) {
        return candidates.find(|member| matches!(member.signature, Signature::Method(..)));
    }
    None
}

/// `IDispatch::Invoke` for an object whose dispatch surface is exactly `interface`.
///
/// # Safety
///
/// `this` must be an interface pointer whose vtable implements `interface`. The remaining pointers follow the
/// `IDispatch::Invoke` contract.
#[expect(clippy::too_many_arguments, reason = "mirrors IDispatch::Invoke")]
pub(crate) unsafe fn invoke(
    interface: Interface,
    this: *mut c_void,
    dispid: i32,
    iid: *const GUID,
    flags: DISPATCH_FLAGS,
    params: *const DISPPARAMS,
    result: *mut VARIANT,
    exception: *mut EXCEPINFO,
    argument_error: *mut u32,
) -> windows::core::HRESULT {
    if !iid.is_null() && unsafe { *iid } != GUID::zeroed() {
        return DISP_E_UNKNOWNINTERFACE;
    }
    let Some(params) = (unsafe { params.as_ref() }) else {
        return E_POINTER;
    };
    let Some(member) = member(interface, dispid, flags) else {
        return DISP_E_MEMBERNOTFOUND;
    };
    unsafe { invoke_member(this, member, params, result, exception, argument_error) }
}

/// Calls `member` on `this` with arguments coerced from `params`.
///
/// A failure returned by the vtable function is reported as `DISP_E_EXCEPTION` with the failure code in
/// `EXCEPINFO::scode` when the caller supplies `exception`, as `ITypeInfo::Invoke` does, and returned directly
/// otherwise.
///
/// # Safety
///
/// `this` must be an interface pointer whose vtable implements `member`. `result`, `exception`, and `argument_error`
/// must each be null or writable.
pub(crate) unsafe fn invoke_member(
    this: *mut c_void,
    member: &Member,
    params: &DISPPARAMS,
    result: *mut VARIANT,
    exception: *mut EXCEPINFO,
    argument_error: *mut u32,
) -> windows::core::HRESULT {
    let (parameter_types, result_type) = match &member.signature {
        Signature::Get(value_type) => (Vec::new(), Some(*value_type)),
        Signature::Put(value_type) => (vec![*value_type], None),
        Signature::Method(parameters, result_type) => (
            parameters.iter().map(|parameter| parameter.value_type).collect(),
            *result_type,
        ),
    };
    let bound = match bind_arguments(params, &parameter_types, matches!(member.signature, Signature::Put(_))) {
        Ok(bound) => bound,
        Err((status, raw_index)) => {
            write_argument_error(argument_error, raw_index);
            return status;
        }
    };

    let mut arguments = Vec::with_capacity(bound.len());
    for (&value_type, &(value, raw_index)) in parameter_types.iter().zip(&bound) {
        match coerce(value, value_type) {
            Ok(argument) => arguments.push(argument),
            Err(status) => {
                write_argument_error(argument_error, Some(raw_index));
                return status;
            }
        }
    }

    // `DispCallFunc` reads a by-value argument from the variant field matching its type, and a by-reference
    // argument from the variant's pointer. The by-reference variants point into `arguments` and `returned`, which
    // are not moved until the call returns.
    let references = arguments
        .iter_mut()
        .zip(&parameter_types)
        .map(|(argument, &value_type)| {
            (value_type == ValueType::VariantRef).then(|| by_reference(VT_VARIANT, ptr::from_mut(argument).cast()))
        })
        .collect::<Vec<_>>();
    let mut types = parameter_types
        .iter()
        .map(|&value_type| match value_type {
            ValueType::VariantRef => (VT_BYREF | VT_VARIANT).0,
            value_type => value_type.vt().0,
        })
        .collect::<Vec<_>>();
    let mut pointers = arguments
        .iter()
        .zip(&references)
        .map(|(argument, reference)| ptr::from_ref(reference.as_ref().unwrap_or(argument)))
        .collect::<Vec<_>>();
    let mut returned = VARIANT::default();
    let returned_reference = result_type.map(|result_type| {
        let payload = unsafe { &mut (*returned.Anonymous.Anonymous).Anonymous };
        (
            result_type,
            by_reference(result_type.vt(), ptr::from_mut(payload).cast()),
        )
    });
    if let Some((result_type, reference)) = &returned_reference {
        types.push((VT_BYREF | result_type.vt()).0);
        pointers.push(ptr::from_ref(reference));
    }

    // UNTESTED: on x86, `DispCallFunc` pushes stdcall arguments sized by the member row and the vtable function
    // pops them by its own signature, so a function whose parameters differ from its row corrupts the caller's
    // stack. The tests run on x64, where such a mismatch goes unnoticed. When a row or a slot function changes,
    // check that the function takes `this` plus exactly the row's parameters, with a trailing pointer for a
    // result.
    // Clear any stale error object so a failure description always belongs to this call.
    let _ = unsafe { SetErrorInfo(0, None) };
    let call = unsafe {
        DispCallFunc(
            Some(this.cast_const()),
            member.slot * size_of::<usize>(),
            CC_STDCALL,
            VT_ERROR,
            types.len() as u32,
            types.as_ptr(),
            pointers.as_ptr(),
        )
    };
    let status = match call {
        Ok(call_result) => windows::core::HRESULT(unsafe { call_result.Anonymous.Anonymous.Anonymous.scode }),
        Err(error) => return error.code(),
    };
    if status.is_err() {
        // The out parameter is undefined after a failure and `returned` still reports VT_EMPTY, so nothing is freed.
        return unsafe { report_failure(status, exception) };
    }

    drop(returned_reference);
    if let Some(result_type) = result_type {
        unsafe { (*returned.Anonymous.Anonymous).vt = result_type.vt() };
        if !result.is_null() {
            unsafe { result.write(returned) };
        }
    }
    S_OK
}

type BindError = (windows::core::HRESULT, Option<u32>);

fn bind_arguments<'a>(
    params: &'a DISPPARAMS,
    parameter_types: &[ValueType],
    property_put: bool,
) -> Result<Vec<(&'a VARIANT, u32)>, BindError> {
    let expected = parameter_types.len();
    if params.cArgs as usize != expected || params.cNamedArgs > params.cArgs {
        return Err((DISP_E_BADPARAMCOUNT, None));
    }
    if expected == 0 {
        return Ok(Vec::new());
    }
    if params.rgvarg.is_null() || (params.cNamedArgs != 0 && params.rgdispidNamedArgs.is_null()) {
        return Err((E_POINTER, None));
    }
    let values = unsafe { slice::from_raw_parts(params.rgvarg, params.cArgs as usize) };
    let named_ids = if params.cNamedArgs == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(params.rgdispidNamedArgs, params.cNamedArgs as usize) }
    };

    if property_put {
        return match named_ids {
            [] | [DISPID_PROPERTYPUT] => Ok(vec![(&values[0], 0)]),
            _ => Err((DISP_E_PARAMNOTFOUND, Some(0))),
        };
    }

    // Named arguments come first in `rgvarg`; positional arguments follow in reverse order.
    let mut bound: Vec<Option<(&VARIANT, u32)>> = vec![None; expected];
    for (raw_index, (&named_id, value)) in named_ids.iter().zip(values).enumerate() {
        let raw_index = raw_index as u32;
        let slot = usize::try_from(named_id).ok().and_then(|index| bound.get_mut(index));
        match slot {
            Some(slot @ None) => *slot = Some((value, raw_index)),
            Some(Some(_)) => return Err((DISP_E_BADPARAMCOUNT, None)),
            None => return Err((DISP_E_PARAMNOTFOUND, Some(raw_index))),
        }
    }
    for (raw_index, value) in values.iter().enumerate().skip(named_ids.len()) {
        let Some(slot) = bound.iter_mut().rev().find(|slot| slot.is_none()) else {
            return Err((DISP_E_BADPARAMCOUNT, None));
        };
        *slot = Some((value, raw_index as u32));
    }
    bound
        .into_iter()
        .map(|argument| argument.ok_or((DISP_E_BADPARAMCOUNT, None)))
        .collect()
}

fn coerce(value: &VARIANT, value_type: ValueType) -> Result<VARIANT, windows::core::HRESULT> {
    let mut direct = VARIANT::default();
    unsafe { VariantCopyInd(&mut direct, value) }.map_err(|error| error.code())?;
    if value_type == ValueType::VariantRef {
        return Ok(direct);
    }
    let mut coerced = VARIANT::default();
    unsafe { VariantChangeType(&mut coerced, &direct, VAR_CHANGE_FLAGS(0), value_type.vt()) }
        .map_err(|error| error.code())?;
    Ok(coerced)
}

fn by_reference(vt: VARENUM, target: *mut c_void) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt: VT_BYREF | vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: VARIANT_0_0_0 { byref: target },
            }),
        },
    }
}

fn write_argument_error(argument_error: *mut u32, raw_index: Option<u32>) {
    if let Some(raw_index) = raw_index
        && !argument_error.is_null()
    {
        unsafe { argument_error.write(raw_index) };
    }
}

unsafe fn report_failure(status: windows::core::HRESULT, exception: *mut EXCEPINFO) -> windows::core::HRESULT {
    if exception.is_null() {
        return status;
    }
    // Converting the code picks up the thread's error object, including a restricted description originated by
    // `windows::core::Error::new`, and falls back to the system text for the code.
    let description = BSTR::from(windows::core::Error::from(status).message());
    unsafe {
        exception.write(EXCEPINFO {
            bstrSource: ManuallyDrop::new(BSTR::from(EXCEPTION_SOURCE)),
            bstrDescription: ManuallyDrop::new(description),
            scode: status.0,
            ..EXCEPINFO::default()
        });
    }
    DISP_E_EXCEPTION
}

#[cfg(test)]
mod tests;

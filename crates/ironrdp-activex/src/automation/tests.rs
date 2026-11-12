use core::cell::Cell;
use std::collections::BTreeMap;

use windows::Win32::Foundation::{DISP_E_TYPEMISMATCH, E_INVALIDARG, E_NOTIMPL};
use windows::Win32::System::Com::{IClassFactory, IDispatch};
use windows::Win32::System::Variant::VT_EMPTY;
use windows::core::{HRESULT, Interface as _, w};

use super::ValueType::{Bool, Bstr, I4, VariantRef};
use super::*;
use crate::com::DllGetClassObject;
use crate::control::CLSID_IRONRDP_ACTIVEX;
use crate::mstsc::IMsRdpClient10;

const DISPID_NUMBER: i32 = 1;
const DISPID_FLAG: i32 = 2;
const DISPID_JOIN: i32 = 3;
const DISPID_FAIL: i32 = 4;
const DISPID_ENDPOINT: i32 = 5;

static FAKE_MEMBERS: &[Member] = &[
    Member::put("Number", DISPID_NUMBER, 7, I4),
    Member::get("Number", DISPID_NUMBER, 8, I4),
    Member::put("Flag", DISPID_FLAG, 9, Bool),
    Member::get("Flag", DISPID_FLAG, 10, Bool),
    Member::method(
        "Join",
        DISPID_JOIN,
        11,
        &[Parameter::new("text", Bstr), Parameter::new("count", I4)],
        Some(Bstr),
    ),
    Member::method("Fail", DISPID_FAIL, 12, &[], None),
    Member::put("Endpoint", DISPID_ENDPOINT, 13, VariantRef),
];

/// A minimal object whose vtable follows `FAKE_MEMBERS`. Slots 0 through 6 are never called.
#[repr(C)]
struct FakeObject {
    vtable: *const [usize; 14],
    _slots: Box<[usize; 14]>,
    number: Cell<i32>,
    flag: Cell<i16>,
    endpoint_type: Cell<VARENUM>,
}

impl FakeObject {
    fn new() -> Self {
        let slots = Box::new([
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            fake_put_number as *const () as usize,
            fake_get_number as *const () as usize,
            fake_put_flag as *const () as usize,
            fake_get_flag as *const () as usize,
            fake_join as *const () as usize,
            fake_fail as *const () as usize,
            fake_put_endpoint as *const () as usize,
        ]);
        Self {
            vtable: ptr::from_ref(&*slots),
            _slots: slots,
            number: Cell::new(0),
            flag: Cell::new(0),
            endpoint_type: Cell::new(VT_EMPTY),
        }
    }

    fn this(&self) -> *mut c_void {
        ptr::from_ref(self).cast_mut().cast()
    }
}

fn fake(this: *mut c_void) -> &'static FakeObject {
    unsafe { &*this.cast::<FakeObject>() }
}

unsafe extern "system" fn fake_put_number(this: *mut c_void, value: i32) -> HRESULT {
    fake(this).number.set(value);
    S_OK
}

unsafe extern "system" fn fake_get_number(this: *mut c_void, value: *mut i32) -> HRESULT {
    unsafe { value.write(fake(this).number.get()) };
    S_OK
}

unsafe extern "system" fn fake_put_flag(this: *mut c_void, value: i16) -> HRESULT {
    fake(this).flag.set(value);
    S_OK
}

unsafe extern "system" fn fake_get_flag(this: *mut c_void, value: *mut i16) -> HRESULT {
    unsafe { value.write(fake(this).flag.get()) };
    S_OK
}

unsafe extern "system" fn fake_join(
    _this: *mut c_void,
    text: *const u16,
    count: i32,
    joined: *mut ManuallyDrop<BSTR>,
) -> HRESULT {
    let text = ManuallyDrop::new(unsafe { BSTR::from_raw(text) });
    unsafe { joined.write(ManuallyDrop::new(BSTR::from(format!("{}:{count}", *text)))) };
    S_OK
}

unsafe extern "system" fn fake_fail(_this: *mut c_void) -> HRESULT {
    windows::core::Error::new(E_NOTIMPL, "fake failure").into()
}

unsafe extern "system" fn fake_put_endpoint(this: *mut c_void, value: *const VARIANT) -> HRESULT {
    fake(this).endpoint_type.set(unsafe { &*value }.vt());
    S_OK
}

struct Call {
    arguments: Vec<VARIANT>,
    named: Vec<i32>,
}

impl Call {
    /// Arguments in declaration order; `DISPPARAMS` receives them reversed, as Automation callers pass them.
    fn positional(arguments: Vec<VARIANT>) -> Self {
        let mut arguments = arguments;
        arguments.reverse();
        Self {
            arguments,
            named: Vec::new(),
        }
    }

    fn property_put(value: VARIANT) -> Self {
        Self {
            arguments: vec![value],
            named: vec![DISPID_PROPERTYPUT],
        }
    }

    fn params(&mut self) -> DISPPARAMS {
        DISPPARAMS {
            rgvarg: self.arguments.as_mut_ptr(),
            rgdispidNamedArgs: if self.named.is_empty() {
                ptr::null_mut()
            } else {
                self.named.as_mut_ptr()
            },
            cArgs: self.arguments.len() as u32,
            cNamedArgs: self.named.len() as u32,
        }
    }
}

struct Outcome {
    status: HRESULT,
    result: VARIANT,
    exception: EXCEPINFO,
    argument_error: u32,
}

fn call_fake(object: &FakeObject, dispid: i32, flags: DISPATCH_FLAGS, mut call: Call) -> Outcome {
    let member = select_member(FAKE_MEMBERS, dispid, flags).expect("fake member exists");
    let params = call.params();
    let mut outcome = Outcome {
        status: S_OK,
        result: VARIANT::default(),
        exception: EXCEPINFO::default(),
        argument_error: u32::MAX,
    };
    outcome.status = unsafe {
        invoke_member(
            object.this(),
            member,
            &params,
            &mut outcome.result,
            &mut outcome.exception,
            &mut outcome.argument_error,
        )
    };
    outcome
}

// oracle: reference mstscax.dll 10.0.26100.9711 embedded MSTSCLib type library
/// Each count is the
/// interface's `cFuncs` minus its seven `IUnknown` and `IDispatch` members: IMsRdpClient10 73,
/// IMsRdpClientAdvancedSettings8 198, IMsRdpClientSecuredSettings2 19,
/// IMsRdpClientTransportSettings4 47 and ITSRemoteProgram3 14. The counts are independent of the
/// member tables, so a dropped member fails `member_tables_cover_every_vtable_slot_once`.
fn expected_slot_count(interface: Interface) -> usize {
    match interface {
        // IMsRdpClient10 ends with RemoteProgram3 at slot 72.
        Interface::Client => 66,
        Interface::AdvancedSettings => 191,
        Interface::SecuredSettings => 12,
        Interface::TransportSettings => 40,
        Interface::RemoteProgram => 7,
    }
}

#[test]
fn member_tables_cover_every_vtable_slot_once() {
    for interface in [
        Interface::Client,
        Interface::AdvancedSettings,
        Interface::SecuredSettings,
        Interface::TransportSettings,
        Interface::RemoteProgram,
    ] {
        let members = interface.members();
        let slots = members.iter().map(|member| member.slot).collect::<Vec<_>>();
        let expected = (7..7 + expected_slot_count(interface)).collect::<Vec<_>>();
        assert_eq!(slots, expected, "{interface:?} slots");

        let mut dispids = BTreeMap::new();
        for member in members {
            let lowercase = member.name.to_ascii_lowercase();
            let dispid = *dispids.entry(lowercase).or_insert(member.dispid);
            assert_eq!(
                dispid, member.dispid,
                "{interface:?}::{} shares one DISPID",
                member.name
            );
        }
    }
}

#[test]
fn member_names_and_parameter_names_resolve_case_insensitively() {
    let mut ids = [0; 2];
    assert_eq!(
        ids_of_names(members::ADVANCED_SETTINGS, &["rdpport"], &mut ids[..1]),
        S_OK
    );
    assert_eq!(ids[0], 108);

    assert_eq!(
        ids_of_names(members::CLIENT, &["SendOnVirtualChannel", "CHANDATA"], &mut ids),
        S_OK
    );
    assert_eq!(ids, [34, 1]);

    assert_eq!(
        ids_of_names(members::CLIENT, &["SendOnVirtualChannel", "unknown"], &mut ids),
        DISP_E_UNKNOWNNAME
    );
    assert_eq!(ids, [34, DISPID_UNKNOWN]);

    assert_eq!(
        ids_of_names(members::TRANSPORT_SETTINGS, &["NotAMember"], &mut ids[..1]),
        DISP_E_UNKNOWNNAME
    );
    assert_eq!(ids[0], DISPID_UNKNOWN);
}

#[test]
fn invoke_flags_select_the_published_member_kind() {
    let advanced_settings9 = member(Interface::Client, 701, DISPATCH_PROPERTYGET).expect("AdvancedSettings9");
    assert_eq!(advanced_settings9.name, "AdvancedSettings9");
    assert!(member(Interface::Client, 701, DISPATCH_METHOD).is_none());

    let reconnect = member(Interface::Client, 702, DISPATCH_METHOD).expect("Reconnect");
    assert_eq!(reconnect.name, "Reconnect");

    let combined = DISPATCH_METHOD | DISPATCH_PROPERTYGET;
    assert!(matches!(
        member(Interface::Client, 1, combined).expect("Server").signature,
        Signature::Get(_)
    ));
    assert!(matches!(
        member(Interface::Client, 1, DISPATCH_PROPERTYPUT)
            .expect("Server")
            .signature,
        Signature::Put(_)
    ));
    assert!(member(Interface::Client, 6, DISPATCH_PROPERTYPUT).is_none());
}

#[test]
fn property_arguments_are_coerced_to_the_published_type() {
    let object = FakeObject::new();

    let outcome = call_fake(
        &object,
        DISPID_NUMBER,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("42"))),
    );
    assert_eq!(outcome.status, S_OK);
    assert_eq!(object.number.get(), 42);

    let outcome = call_fake(
        &object,
        DISPID_NUMBER,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(7i16)),
    );
    assert_eq!(outcome.status, S_OK);
    let outcome = call_fake(&object, DISPID_NUMBER, DISPATCH_PROPERTYGET, Call::positional(vec![]));
    assert_eq!(outcome.status, S_OK);
    assert_eq!(outcome.result.vt(), VT_I4);
    assert_eq!(i32::try_from(&outcome.result), Ok(7));

    let outcome = call_fake(
        &object,
        DISPID_FLAG,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(1i32)),
    );
    assert_eq!(outcome.status, S_OK);
    assert_eq!(object.flag.get(), -1);
    let outcome = call_fake(&object, DISPID_FLAG, DISPATCH_PROPERTYGET, Call::positional(vec![]));
    assert_eq!(outcome.result.vt(), VT_BOOL);
    assert_eq!(bool::try_from(&outcome.result), Ok(true));
}

#[test]
fn uncoercible_arguments_report_the_argument_index() {
    let object = FakeObject::new();
    let outcome = call_fake(
        &object,
        DISPID_JOIN,
        DISPATCH_METHOD,
        Call::positional(vec![VARIANT::from(BSTR::from("text")), VARIANT::from(BSTR::from("x"))]),
    );
    assert_eq!(outcome.status, DISP_E_TYPEMISMATCH);
    // `count` is the second parameter, so it is the first entry of the reversed `rgvarg`.
    assert_eq!(outcome.argument_error, 0);
}

#[test]
fn methods_bind_positional_and_named_arguments() {
    let object = FakeObject::new();
    let outcome = call_fake(
        &object,
        DISPID_JOIN,
        DISPATCH_METHOD,
        Call::positional(vec![VARIANT::from(BSTR::from("ab")), VARIANT::from(3i32)]),
    );
    assert_eq!(outcome.status, S_OK);
    assert_eq!(outcome.result.vt(), VT_BSTR);
    assert_eq!(BSTR::try_from(&outcome.result).expect("BSTR result"), "ab:3");

    let outcome = call_fake(
        &object,
        DISPID_JOIN,
        DISPATCH_METHOD,
        Call {
            arguments: vec![VARIANT::from(4i32), VARIANT::from(BSTR::from("named"))],
            named: vec![1],
        },
    );
    assert_eq!(outcome.status, S_OK);
    assert_eq!(BSTR::try_from(&outcome.result).expect("BSTR result"), "named:4");

    let outcome = call_fake(
        &object,
        DISPID_JOIN,
        DISPATCH_METHOD,
        Call::positional(vec![VARIANT::from(BSTR::from("ab"))]),
    );
    assert_eq!(outcome.status, DISP_E_BADPARAMCOUNT);

    let outcome = call_fake(
        &object,
        DISPID_JOIN,
        DISPATCH_METHOD,
        Call {
            arguments: vec![VARIANT::from(4i32), VARIANT::from(BSTR::from("named"))],
            named: vec![7],
        },
    );
    assert_eq!(outcome.status, DISP_E_PARAMNOTFOUND);
    assert_eq!(outcome.argument_error, 0);
}

#[test]
fn variant_reference_parameters_receive_the_caller_value() {
    let object = FakeObject::new();
    let outcome = call_fake(
        &object,
        DISPID_ENDPOINT,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("endpoint"))),
    );
    assert_eq!(outcome.status, S_OK);
    assert_eq!(object.endpoint_type.get(), VT_BSTR);
}

#[test]
fn member_failures_surface_through_excepinfo() {
    let object = FakeObject::new();
    let mut outcome = call_fake(&object, DISPID_FAIL, DISPATCH_METHOD, Call::positional(vec![]));
    assert_eq!(outcome.status, DISP_E_EXCEPTION);
    assert_eq!(outcome.exception.scode, E_NOTIMPL.0);
    assert_eq!(*outcome.exception.bstrDescription, "fake failure");
    assert_eq!(*outcome.exception.bstrSource, EXCEPTION_SOURCE);
    unsafe {
        ManuallyDrop::drop(&mut outcome.exception.bstrSource);
        ManuallyDrop::drop(&mut outcome.exception.bstrDescription);
        ManuallyDrop::drop(&mut outcome.exception.bstrHelpFile);
    }

    let member = select_member(FAKE_MEMBERS, DISPID_FAIL, DISPATCH_METHOD).expect("Fail");
    let mut call = Call::positional(vec![]);
    let params = call.params();
    let status = unsafe {
        invoke_member(
            object.this(),
            member,
            &params,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    assert_eq!(status, E_NOTIMPL);
}

fn create_control() -> IDispatch {
    let mut raw_factory = ptr::null_mut();
    let status = unsafe { DllGetClassObject(&CLSID_IRONRDP_ACTIVEX, &IClassFactory::IID, &mut raw_factory) };
    assert_eq!(status, S_OK);
    let factory = unsafe { IClassFactory::from_raw(raw_factory) };
    let client: IMsRdpClient10 = unsafe { factory.CreateInstance(None) }.expect("create control");
    client.cast().expect("control dispatch")
}

fn dispid(dispatch: &IDispatch, names: &[PCWSTR]) -> Vec<i32> {
    let mut ids = vec![0; names.len()];
    unsafe { dispatch.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), names.len() as u32, 0, ids.as_mut_ptr()) }
        .expect("names resolve");
    ids
}

fn invoke_dispatch(
    dispatch: &IDispatch,
    dispid: i32,
    flags: DISPATCH_FLAGS,
    mut call: Call,
) -> windows::core::Result<VARIANT> {
    let params = call.params();
    let mut result = VARIANT::default();
    unsafe {
        dispatch.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            flags,
            &params,
            Some(&mut result),
            None,
            None,
        )
    }?;
    Ok(result)
}

fn property_dispatch(dispatch: &IDispatch, name: PCWSTR) -> IDispatch {
    let id = dispid(dispatch, &[name])[0];
    let value = invoke_dispatch(dispatch, id, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("property get");
    assert_eq!(value.vt(), VT_DISPATCH);
    IDispatch::try_from(&value).expect("settings object")
}

// oracle: reference mstscax.dll 10.0.26100.9711 embedded MSTSCLib type library (DISPIDs)
#[test]
fn settings_objects_are_reachable_by_name() {
    let control = create_control();

    let advanced = property_dispatch(&control, w!("AdvancedSettings9"));
    let rdp_port = dispid(&advanced, &[w!("RDPPort")])[0];
    assert_eq!(rdp_port, 108);
    invoke_dispatch(
        &advanced,
        rdp_port,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(3390i16)),
    )
    .expect("put RDPPort");
    let port = invoke_dispatch(&advanced, rdp_port, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(i32::try_from(&port), Ok(3390));
    let rejected = invoke_dispatch(
        &advanced,
        rdp_port,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(0i32)),
    );
    assert_eq!(rejected.expect_err("port 0 is invalid").code(), E_INVALIDARG);

    // Every AdvancedSettings version shares the same object state.
    let advanced2 = property_dispatch(&control, w!("AdvancedSettings2"));
    let port = invoke_dispatch(&advanced2, rdp_port, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(i32::try_from(&port), Ok(3390));

    let transport = property_dispatch(&control, w!("TransportSettings4"));
    let hostname = dispid(&transport, &[w!("GatewayHostname")])[0];
    invoke_dispatch(
        &transport,
        hostname,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("gateway.example"))),
    )
    .expect("put GatewayHostname");
    let value = invoke_dispatch(&transport, hostname, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(BSTR::try_from(&value).expect("hostname"), "gateway.example");

    let secured = property_dispatch(&control, w!("SecuredSettings3"));
    let start_program = dispid(&secured, &[w!("StartProgram")])[0];
    invoke_dispatch(
        &secured,
        start_program,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("notepad.exe"))),
    )
    .expect("put StartProgram");
    let value = invoke_dispatch(&secured, start_program, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(BSTR::try_from(&value).expect("start program"), "notepad.exe");

    let remote_program = property_dispatch(&control, w!("RemoteProgram3"));
    let mode = dispid(&remote_program, &[w!("RemoteProgramMode")])[0];
    assert_eq!(mode, 200);
    invoke_dispatch(
        &remote_program,
        mode,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(1i32)),
    )
    .expect("an integer coerces to VARIANT_BOOL");
    let value = invoke_dispatch(&remote_program, mode, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(value.vt(), VT_BOOL);
    assert_eq!(bool::try_from(&value), Ok(true));
}

// oracle: reference mstscax.dll 10.0.26100.9711 embedded MSTSCLib type library (DISPIDs)
#[test]
fn virtual_channel_methods_are_reachable_by_name() {
    let control = create_control();

    let create = dispid(&control, &[w!("CreateVirtualChannels")])[0];
    invoke_dispatch(
        &control,
        create,
        DISPATCH_METHOD,
        Call::positional(vec![VARIANT::from(BSTR::from("chan1,chan2"))]),
    )
    .expect("CreateVirtualChannels");

    let ids = dispid(&control, &[w!("GetVirtualChannelOptions"), w!("chanName")]);
    assert_eq!(ids, [36, 0]);
    let options = invoke_dispatch(
        &control,
        ids[0],
        DISPATCH_METHOD,
        Call::positional(vec![VARIANT::from(BSTR::from("chan2"))]),
    )
    .expect("GetVirtualChannelOptions");
    assert_eq!(options.vt(), VT_I4);

    let send = dispid(&control, &[w!("SendOnVirtualChannel")])[0];
    invoke_dispatch(
        &control,
        send,
        DISPATCH_METHOD,
        Call::positional(vec![
            VARIANT::from(BSTR::from("chan1")),
            VARIANT::from(BSTR::from("data")),
        ]),
    )
    .expect("a send before connecting is accepted");
    let unknown = invoke_dispatch(
        &control,
        send,
        DISPATCH_METHOD,
        Call::positional(vec![
            VARIANT::from(BSTR::from("other")),
            VARIANT::from(BSTR::from("data")),
        ]),
    );
    assert_eq!(unknown.expect_err("unknown channel").code(), E_INVALIDARG);
}

#[test]
fn late_binding_reaches_hotkeys_and_the_client_shell() {
    let control = create_control();

    let advanced = property_dispatch(&control, w!("AdvancedSettings9"));
    let full_screen = dispid(&advanced, &[w!("HotKeyFullScreen")])[0];
    invoke_dispatch(
        &advanced,
        full_screen,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(0x0di32)),
    )
    .expect("put HotKeyFullScreen");
    let key = invoke_dispatch(&advanced, full_screen, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert_eq!(i32::try_from(&key), Ok(0x0d));

    let shell = property_dispatch(&control, w!("MsRdpClientShell"));
    let contents = dispid(&shell, &[w!("RdpFileContents")])[0];
    invoke_dispatch(
        &shell,
        contents,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("full address:s:host.example\r\n"))),
    )
    .expect("put RdpFileContents");
    let value = invoke_dispatch(&shell, contents, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("get");
    assert!(
        BSTR::try_from(&value)
            .expect("contents")
            .to_string()
            .contains("full address:s:host.example")
    );
}

// oracle: reference mstscax.dll 10.0.26100.9711 embedded MSTSCLib type library (DISPIDs and Automation types)
#[test]
fn client_properties_use_the_published_automation_types() {
    let control = create_control();

    let connected = invoke_dispatch(&control, 6, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("Connected");
    assert_eq!(connected.vt(), VT_I2);

    invoke_dispatch(
        &control,
        12,
        DISPATCH_PROPERTYPUT,
        Call::property_put(VARIANT::from(BSTR::from("1280"))),
    )
    .expect("DesktopWidth accepts a coercible string");
    let width = invoke_dispatch(&control, 12, DISPATCH_PROPERTYGET, Call::positional(vec![])).expect("DesktopWidth");
    assert_eq!(i32::try_from(&width), Ok(1280));

    // IronRDP's password property and the modern facade keep their own DISPIDs.
    assert_eq!(dispid(&control, &[w!("IronRdpPassword")]), [0x10000]);
    assert_eq!(dispid(&control, &[w!("Settings")]), [710]);
}

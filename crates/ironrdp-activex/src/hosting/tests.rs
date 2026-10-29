use core::cell::RefCell;
use std::collections::BTreeMap;

use windows::Win32::Foundation::{S_FALSE, S_OK};
use windows::Win32::System::Com::IPersistStreamInit;
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag_Impl, IPropertyBag2_Impl};
use windows::Win32::System::Ole::{IPersistPropertyBag, IPersistPropertyBag2, IProvideClassInfo2, IQuickActivate};
use windows::Win32::System::Variant::{VAR_CHANGE_FLAGS, VARIANT_0_0, VariantChangeType};
use windows_core::{AsImpl as _, BSTR, IUnknown, PCWSTR, implement};

use super::*;
use crate::control::Control;

/// An in-memory property bag keyed by case-insensitive property name, as the HTML `<param>` bag behaves.
#[derive(Default)]
struct Store {
    values: RefCell<BTreeMap<String, VARIANT>>,
}

impl Store {
    fn read(&self, name: &str, value: *mut VARIANT) -> Result<()> {
        let values = self.values.borrow();
        let stored = values
            .get(&name.to_ascii_lowercase())
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
        let value = unsafe { &mut *value };
        let requested = value.vt();
        if requested == VT_EMPTY {
            *value = stored.clone();
            return Ok(());
        }
        let mut coerced = VARIANT::default();
        unsafe { VariantChangeType(&mut coerced, stored, VAR_CHANGE_FLAGS(0), requested) }?;
        *value = coerced;
        Ok(())
    }

    fn write(&self, name: &str, value: *const VARIANT) {
        self.values
            .borrow_mut()
            .insert(name.to_ascii_lowercase(), unsafe { &*value }.clone());
    }

    fn insert(&self, name: &str, value: VARIANT) {
        self.values.borrow_mut().insert(name.to_ascii_lowercase(), value);
    }

    fn get(&self, name: &str) -> Option<VARIANT> {
        self.values.borrow().get(&name.to_ascii_lowercase()).cloned()
    }
}

#[implement(IPropertyBag)]
#[derive(Default)]
struct Bag(Store);

impl IPropertyBag_Impl for Bag_Impl {
    fn Read(&self, name: &PCWSTR, value: *mut VARIANT, _log: Ref<'_, IErrorLog>) -> Result<()> {
        self.0.read(&unsafe { name.to_string() }?, value)
    }

    fn Write(&self, name: &PCWSTR, value: *const VARIANT) -> Result<()> {
        self.0.write(&unsafe { name.to_string() }?, value);
        Ok(())
    }
}

#[implement(IPropertyBag2)]
#[derive(Default)]
struct Bag2(Store);

fn bag2_name(description: &PROPBAG2) -> Result<String> {
    Ok(unsafe { description.pstrName.to_string() }?)
}

impl IPropertyBag2_Impl for Bag2_Impl {
    fn Read(
        &self,
        count: u32,
        descriptions: *const PROPBAG2,
        _log: Ref<'_, IErrorLog>,
        values: *mut VARIANT,
        statuses: *mut HRESULT,
    ) -> Result<()> {
        for index in 0..count as usize {
            let description = unsafe { &*descriptions.add(index) };
            let value = unsafe { values.add(index) };
            unsafe { (*value.cast::<VARIANT_0_0>()).vt = description.vt };
            let status = self.0.read(&bag2_name(description)?, value);
            unsafe {
                statuses
                    .add(index)
                    .write(status.map_or_else(|error| error.code(), |()| S_OK))
            };
        }
        Ok(())
    }

    fn Write(&self, count: u32, descriptions: *const PROPBAG2, values: *const VARIANT) -> Result<()> {
        for index in 0..count as usize {
            let description = unsafe { &*descriptions.add(index) };
            self.0.write(&bag2_name(description)?, unsafe { values.add(index) });
        }
        Ok(())
    }

    fn CountProperties(&self) -> Result<u32> {
        Ok(self.0.values.borrow().len() as u32)
    }

    fn GetPropertyInfo(
        &self,
        _first: u32,
        _count: u32,
        _descriptions: *mut PROPBAG2,
        _fetched: *mut u32,
    ) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }

    fn LoadObject(
        &self,
        _name: &PCWSTR,
        _hint: u32,
        _object: Ref<'_, IUnknown>,
        _log: Ref<'_, IErrorLog>,
    ) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

fn control() -> IUnknown {
    Control::new().into()
}

fn get(control: &IUnknown, dispid: i32) -> VARIANT {
    let dispatch: IDispatch = control.cast().expect("dispatch");
    let params = DISPPARAMS::default();
    let mut value = VARIANT::default();
    unsafe {
        dispatch.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYGET,
            &params,
            Some(&mut value),
            None,
            None,
        )
    }
    .expect("property get");
    value
}

fn put(control: &IUnknown, dispid: i32, mut value: VARIANT) {
    let dispatch: IDispatch = control.cast().expect("dispatch");
    let mut named = DISPID_PROPERTYPUT;
    let params = DISPPARAMS {
        rgvarg: &mut value,
        rgdispidNamedArgs: &mut named,
        cArgs: 1,
        cNamedArgs: 1,
    };
    unsafe {
        dispatch.Invoke(
            dispid,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYPUT,
            &params,
            None,
            None,
            None,
        )
    }
    .expect("property put");
}

#[test]
fn class_info_names_the_default_event_interface_without_type_information() {
    let info: IProvideClassInfo2 = control().cast().expect("IProvideClassInfo2");
    assert_eq!(
        unsafe { info.GetGUID(GUIDKIND_DEFAULT_SOURCE_DISP_IID.0 as u32) }.expect("default source"),
        IID_MSTSCLIB_EVENTS
    );
    assert_eq!(
        unsafe { info.GetGUID(0) }.expect_err("unknown GUID kind").code(),
        E_INVALIDARG
    );
    assert_eq!(
        unsafe { info.GetClassInfo() }.expect_err("no type info").code(),
        E_NOTIMPL
    );
}

#[test]
fn property_bag_round_trips_non_secret_settings() {
    let source = control();
    put(&source, 1, VARIANT::from(BSTR::from("rdp.example")));
    put(&source, 3, VARIANT::from(BSTR::from("alice")));
    put(&source, 12, VARIANT::from(1280i32));
    put(&source, 100, VARIANT::from(16i32));
    put(&source, 0x10000, VARIANT::from(BSTR::from("secret")));

    let bag: IPropertyBag = Bag::default().into();
    let persist: IPersistPropertyBag = source.cast().expect("IPersistPropertyBag");
    unsafe { persist.Save(&bag, true, true) }.expect("save");
    let stored = unsafe { bag.as_impl() };
    assert_eq!(
        BSTR::try_from(&stored.0.get("Server").expect("Server")).expect("BSTR"),
        "rdp.example"
    );
    assert_eq!(i32::try_from(&stored.0.get("DesktopWidth").expect("width")), Ok(1280));
    assert!(stored.0.get("FullScreenTitle").is_none());
    assert!(
        stored.0.values.borrow().keys().all(|name| !name.contains("password")),
        "passwords are never saved"
    );

    let target = control();
    let persist: IPersistPropertyBag = target.cast().expect("IPersistPropertyBag");
    unsafe { persist.Load(&bag, None) }.expect("load");
    assert_eq!(BSTR::try_from(&get(&target, 1)).expect("Server"), "rdp.example");
    assert_eq!(BSTR::try_from(&get(&target, 3)).expect("UserName"), "alice");
    assert_eq!(i32::try_from(&get(&target, 12)), Ok(1280));
    assert_eq!(i32::try_from(&get(&target, 100)), Ok(16));
    let stream: IPersistStreamInit = target.cast().expect("IPersistStreamInit");
    assert_eq!(unsafe { stream.IsDirty() }, S_FALSE);
}

#[test]
fn property_bag_values_are_coerced_and_validated() {
    let bag = Bag::default();
    bag.0.insert("server", VARIANT::from(BSTR::from("host")));
    bag.0.insert("DESKTOPHEIGHT", VARIANT::from(BSTR::from("900")));
    bag.0.insert("StartConnected", VARIANT::from(BSTR::from("0")));
    let bag: IPropertyBag = bag.into();

    let target = control();
    let persist: IPersistPropertyBag = target.cast().expect("IPersistPropertyBag");
    unsafe { persist.Load(&bag, None) }.expect("load coerced strings");
    assert_eq!(BSTR::try_from(&get(&target, 1)).expect("Server"), "host");
    assert_eq!(i32::try_from(&get(&target, 13)), Ok(900));

    let invalid = Bag::default();
    invalid.0.insert("ColorDepth", VARIANT::from(24i32));
    let invalid: IPropertyBag = invalid.into();
    let error = unsafe { persist.Load(&invalid, None) }.expect_err("24-bit color is rejected");
    assert_eq!(error.code(), E_INVALIDARG);

    let malformed = Bag::default();
    malformed.0.insert("DesktopHeight", VARIANT::from(BSTR::from("abc")));
    let malformed: IPropertyBag = malformed.into();
    unsafe { persist.Load(&malformed, None) }.expect_err("a nonnumeric height is rejected, not skipped");
    assert_eq!(i32::try_from(&get(&target, 13)), Ok(900));
}

#[test]
fn property_bag2_rejects_values_that_cannot_convert() {
    let target = control();
    put(&target, 13, VARIANT::from(720i32));

    let bag = Bag2::default();
    bag.0.insert("DesktopHeight", VARIANT::from(BSTR::from("abc")));
    let bag: IPropertyBag2 = bag.into();
    let persist: IPersistPropertyBag2 = target.cast().expect("IPersistPropertyBag2");
    unsafe { persist.Load(&bag, None) }.expect_err("a nonnumeric height is rejected, not skipped");
    assert_eq!(i32::try_from(&get(&target, 13)), Ok(720));
}

#[test]
fn property_bag2_round_trips_non_secret_settings() {
    let source = control();
    put(&source, 2, VARIANT::from(BSTR::from("CONTOSO")));
    put(&source, 13, VARIANT::from(720i32));

    let bag: IPropertyBag2 = Bag2::default().into();
    let persist: IPersistPropertyBag2 = source.cast().expect("IPersistPropertyBag2");
    unsafe { persist.Save(&bag, false, true) }.expect("save");
    assert_eq!(unsafe { persist.IsDirty() }, S_OK);

    let target = control();
    let persist: IPersistPropertyBag2 = target.cast().expect("IPersistPropertyBag2");
    unsafe { persist.Load(&bag, None) }.expect("load");
    assert_eq!(BSTR::try_from(&get(&target, 2)).expect("Domain"), "CONTOSO");
    assert_eq!(i32::try_from(&get(&target, 13)), Ok(720));
    assert_eq!(unsafe { persist.IsDirty() }, S_FALSE);

    unsafe { persist.InitNew() }.expect("InitNew");
    assert_eq!(BSTR::try_from(&get(&target, 2)).expect("Domain"), "");
}

#[test]
fn quick_activate_reports_status_and_extent() {
    let target = control();
    let activate: IQuickActivate = target.cast().expect("IQuickActivate");
    let mut container: QACONTAINER = unsafe { core::mem::zeroed() };
    container.cbSize = size_of::<QACONTAINER>() as u32;
    let mut control = QACONTROL {
        cbSize: size_of::<QACONTROL>() as u32,
        dwEventCookie: 7,
        ..QACONTROL::default()
    };
    unsafe { activate.QuickActivate(&container, &mut control) }.expect("quick activate");
    assert_eq!(control.cbSize, size_of::<QACONTROL>() as u32);
    assert_ne!(control.dwMiscStatus, 0);
    assert_eq!(control.dwEventCookie, 0);
    assert_eq!(control.dwPropNotifyCookie, 0);

    let extent = SIZE { cx: 2540, cy: 1905 };
    unsafe { activate.SetContentExtent(&extent) }.expect("set extent");
    assert_eq!(unsafe { activate.GetContentExtent() }.expect("get extent"), extent);
}

#[test]
fn quick_activate_rejects_null_structures() {
    let target = control();
    let activate: IQuickActivate = target.cast().expect("IQuickActivate");
    let mut control = QACONTROL::default();
    assert_eq!(
        unsafe { activate.QuickActivate(ptr::null(), &mut control) }
            .expect_err("null container")
            .code(),
        E_POINTER
    );

    let mut container: QACONTAINER = unsafe { core::mem::zeroed() };
    container.cbSize = size_of::<QACONTAINER>() as u32;
    let mut truncated = QACONTROL {
        cbSize: 8,
        ..QACONTROL::default()
    };
    assert_eq!(
        unsafe { activate.QuickActivate(&container, &mut truncated) }
            .expect_err("QACONTROL too small")
            .code(),
        E_INVALIDARG
    );
}

#[test]
fn quick_activate_preserves_the_pointer_policy_of_short_structures() {
    let target = control();
    let activate: IQuickActivate = target.cast().expect("IQuickActivate");
    let mut container: QACONTAINER = unsafe { core::mem::zeroed() };
    container.cbSize = size_of::<QACONTAINER>() as u32;
    let mut control = QACONTROL {
        cbSize: offset_of!(QACONTROL, dwPointerActivationPolicy) as u32,
        dwPointerActivationPolicy: 0xA5A5_A5A5,
        ..QACONTROL::default()
    };
    unsafe { activate.QuickActivate(&container, &mut control) }.expect("short QACONTROL");
    assert_ne!(control.dwMiscStatus, 0);
    assert_eq!(control.dwPointerActivationPolicy, 0xA5A5_A5A5);
}

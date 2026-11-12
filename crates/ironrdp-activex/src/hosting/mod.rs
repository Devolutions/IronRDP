//! Container-facing OLE contracts that sit beside the core embedding interfaces.
//!
//! `IProvideClassInfo2` names the default event interface, `IQuickActivate` performs the usual container handshake
//! in one call, and `IPersistPropertyBag`/`IPersistPropertyBag2` load and save the control's non-secret connection
//! properties by their MSTSCLib names, for example from HTML `<param>` elements. Each operation goes through the same
//! `IOleObject`, `IViewObject`, connection-point, `IPersistStreamInit`, and `IDispatch` paths a container could call
//! directly.

use core::mem::offset_of;
use core::ptr;

use windows::Win32::Foundation::{E_INVALIDARG, E_NOTIMPL, E_POINTER, SIZE};
use windows::Win32::System::Com::StructuredStorage::{IPropertyBag, IPropertyBag2, PROPBAG2};
use windows::Win32::System::Com::{
    DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, DVASPECT, IAdviseSink, IConnectionPointContainer,
    IDispatch, IErrorLog, IPersistStreamInit_Impl, ITypeInfo,
};
use windows::Win32::System::Ole::{
    DISPID_PROPERTYPUT, GUIDKIND_DEFAULT_SOURCE_DISP_IID, IOleObject, IPersistPropertyBag_Impl,
    IPersistPropertyBag2_Impl, IProvideClassInfo_Impl, IProvideClassInfo2_Impl, IQuickActivate_Impl, IViewObject,
    IViewObjectEx, PROPBAG2_TYPE_DATA, QACONTAINER, QACONTROL,
};
use windows::Win32::System::Variant::{VARENUM, VARIANT, VT_BOOL, VT_BSTR, VT_EMPTY, VT_I4};
use windows_core::{BOOL, Error, GUID, HRESULT, IUnknownImpl as _, Interface as _, PWSTR, Ref, Result};

use crate::control::{Control_Impl, IID_MSTSCLIB_EVENTS};
use crate::mstsc::IMsRdpClient10;

/// `IQuickActivate` always negotiates the content aspect.
const CONTENT_ASPECT: DVASPECT = DVASPECT(1);

/// A control property exchanged with a property bag under its MSTSCLib name.
struct BagProperty {
    name: &'static str,
    dispid: i32,
    vt: VARENUM,
    /// MSTSCLib exposes `FullScreenTitle` as write-only, so it can be loaded but never saved.
    saved: bool,
}

const fn bag_property(name: &'static str, dispid: i32, vt: VARENUM) -> BagProperty {
    BagProperty {
        name,
        dispid,
        vt,
        saved: true,
    }
}

/// The non-secret settings `IPersistStreamInit` persists that are reachable on the control's dispatch surface.
/// Passwords are never read from or written to a property bag.
const BAG_PROPERTIES: &[BagProperty] = &[
    bag_property("Server", 1, VT_BSTR),
    bag_property("Domain", 2, VT_BSTR),
    bag_property("UserName", 3, VT_BSTR),
    bag_property("DisconnectedText", 4, VT_BSTR),
    bag_property("ConnectingText", 5, VT_BSTR),
    bag_property("DesktopWidth", 12, VT_I4),
    bag_property("DesktopHeight", 13, VT_I4),
    bag_property("StartConnected", 16, VT_I4),
    BagProperty {
        name: "FullScreenTitle",
        dispid: 19,
        vt: VT_BSTR,
        saved: false,
    },
    bag_property("ColorDepth", 100, VT_I4),
    bag_property("FullScreen", 104, VT_BOOL),
    bag_property("ConnectedStatusText", 201, VT_BSTR),
];

impl IProvideClassInfo_Impl for Control_Impl {
    fn GetClassInfo(&self) -> Result<ITypeInfo> {
        // The control serves no type information; see `IDispatch::GetTypeInfo`.
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

impl IProvideClassInfo2_Impl for Control_Impl {
    fn GetGUID(&self, kind: u32) -> Result<GUID> {
        if kind == GUIDKIND_DEFAULT_SOURCE_DISP_IID.0 as u32 {
            Ok(IID_MSTSCLIB_EVENTS)
        } else {
            Err(Error::from_hresult(E_INVALIDARG))
        }
    }
}

impl IQuickActivate_Impl for Control_Impl {
    fn QuickActivate(&self, container: *const QACONTAINER, control: *mut QACONTROL) -> Result<()> {
        let Some(container) = (unsafe { container.as_ref() }) else {
            return Err(Error::from_hresult(E_POINTER));
        };
        if control.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // Older containers pass a `QACONTROL` without the trailing pointer-activation policy, so only the fields every
        // version has are written. The policy keeps the caller's value because the control has no `IPointerInactive`.
        let size = unsafe { ptr::addr_of!((*control).cbSize).read_unaligned() } as usize;
        if size < offset_of!(QACONTROL, dwPointerActivationPolicy) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let mut status = QuickActivateStatus::default();
        let result = quick_activate(self, container, &mut status);
        unsafe {
            ptr::addr_of_mut!((*control).dwMiscStatus).write_unaligned(status.misc);
            ptr::addr_of_mut!((*control).dwViewStatus).write_unaligned(status.view);
            ptr::addr_of_mut!((*control).dwEventCookie).write_unaligned(status.event_cookie);
            // No MSTSCLib property is bindable, so a property-notification sink is never connected.
            ptr::addr_of_mut!((*control).dwPropNotifyCookie).write_unaligned(0);
        }
        result
    }

    fn SetContentExtent(&self, extent: *const SIZE) -> Result<()> {
        let extent = unsafe { extent.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
        let object: IOleObject = self.to_interface();
        unsafe { object.SetExtent(CONTENT_ASPECT, extent) }
    }

    fn GetContentExtent(&self) -> Result<SIZE> {
        let object: IOleObject = self.to_interface();
        unsafe { object.GetExtent(CONTENT_ASPECT) }
    }
}

#[derive(Default)]
struct QuickActivateStatus {
    misc: u32,
    view: u32,
    event_cookie: u32,
}

/// Performs the steps a container would otherwise take one call at a time, recording what `QACONTROL` reports.
fn quick_activate(control: &Control_Impl, container: &QACONTAINER, status: &mut QuickActivateStatus) -> Result<()> {
    let object: IOleObject = control.to_interface();
    unsafe { object.SetClientSite(container.pClientSite.as_ref()) }?;
    if let Some(sink) = container.pAdviseSink.as_ref() {
        let sink: IAdviseSink = sink.cast()?;
        let view: IViewObject = control.to_interface();
        unsafe { view.SetAdvise(CONTENT_ASPECT, 0, &sink) }?;
    }
    if let Some(sink) = container.pUnkEventSink.as_ref() {
        let points: IConnectionPointContainer = control.to_interface();
        let point = unsafe { points.FindConnectionPoint(&IID_MSTSCLIB_EVENTS) }?;
        status.event_cookie = unsafe { point.Advise(sink) }?;
    }
    status.misc = unsafe { object.GetMiscStatus(CONTENT_ASPECT) }?.0 as u32;
    let view: IViewObjectEx = control.to_interface();
    status.view = unsafe { view.GetViewStatus() }?;
    Ok(())
}

impl IPersistPropertyBag_Impl for Control_Impl {
    fn InitNew(&self) -> Result<()> {
        IPersistStreamInit_Impl::InitNew(self)
    }

    fn Load(&self, bag: Ref<'_, IPropertyBag>, error_log: Ref<'_, IErrorLog>) -> Result<()> {
        let bag = bag.ok()?;
        load_bag_properties(self, |property| {
            // Request the stored type so a value that cannot convert fails the property put instead of reading as
            // missing.
            let mut value = VARIANT::default();
            let name = wide(property.name);
            unsafe { bag.Read(windows_core::PCWSTR(name.as_ptr()), &mut value, error_log.as_ref()) }
                .ok()
                .map(|()| value)
        })
    }

    fn Save(&self, bag: Ref<'_, IPropertyBag>, clear_dirty: BOOL, _save_all: BOOL) -> Result<()> {
        let bag = bag.ok()?;
        save_bag_properties(self, clear_dirty, |property, value| {
            let name = wide(property.name);
            unsafe { bag.Write(windows_core::PCWSTR(name.as_ptr()), value) }
        })
    }
}

impl IPersistPropertyBag2_Impl for Control_Impl {
    fn InitNew(&self) -> Result<()> {
        IPersistStreamInit_Impl::InitNew(self)
    }

    fn Load(&self, bag: Ref<'_, IPropertyBag2>, error_log: Ref<'_, IErrorLog>) -> Result<()> {
        let bag = bag.ok()?;
        load_bag_properties(self, |property| {
            let mut name = wide(property.name);
            // As with `IPropertyBag`, read the stored type and leave conversion to the property put.
            let description = PROPBAG2 {
                vt: VT_EMPTY,
                ..bag2_description(property, &mut name)
            };
            let mut value = VARIANT::default();
            let mut status = HRESULT(0);
            let read = unsafe { bag.Read(1, &description, error_log.as_ref(), &mut value, &mut status) };
            (read.is_ok() && status.is_ok()).then_some(value)
        })
    }

    fn Save(&self, bag: Ref<'_, IPropertyBag2>, clear_dirty: BOOL, _save_all: BOOL) -> Result<()> {
        let bag = bag.ok()?;
        save_bag_properties(self, clear_dirty, |property, value| {
            let mut name = wide(property.name);
            let description = bag2_description(property, &mut name);
            unsafe { bag.Write(1, &description, value) }
        })
    }

    fn IsDirty(&self) -> HRESULT {
        IPersistStreamInit_Impl::IsDirty(self)
    }
}

/// Applies every property the bag holds through the control's own property puts.
///
/// A property missing from the bag keeps its current value. A value the control rejects fails the load, so an invalid
/// `<param>` is never silently ignored.
fn load_bag_properties(control: &Control_Impl, mut read: impl FnMut(&BagProperty) -> Option<VARIANT>) -> Result<()> {
    control.ensure_persisted_settings_replaceable()?;
    let dispatch = control_dispatch(control)?;
    for property in BAG_PROPERTIES {
        let Some(mut value) = read(property) else {
            continue;
        };
        let mut named = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: &mut value,
            rgdispidNamedArgs: &mut named,
            cArgs: 1,
            cNamedArgs: 1,
        };
        unsafe {
            dispatch.Invoke(
                property.dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYPUT,
                &params,
                None,
                None,
                None,
            )
        }?;
    }
    control.set_persistence_dirty(false);
    Ok(())
}

fn save_bag_properties(
    control: &Control_Impl,
    clear_dirty: BOOL,
    mut write: impl FnMut(&BagProperty, &VARIANT) -> Result<()>,
) -> Result<()> {
    let dispatch = control_dispatch(control)?;
    for property in BAG_PROPERTIES.iter().filter(|property| property.saved) {
        let params = DISPPARAMS::default();
        let mut value = VARIANT::default();
        unsafe {
            dispatch.Invoke(
                property.dispid,
                &GUID::zeroed(),
                0,
                DISPATCH_PROPERTYGET,
                &params,
                Some(&mut value),
                None,
                None,
            )
        }?;
        write(property, &value)?;
    }
    if clear_dirty.as_bool() {
        control.set_persistence_dirty(false);
    }
    Ok(())
}

/// The control's dispatch surface, which `IMsRdpClient10` carries as its dual-interface base.
fn control_dispatch(control: &Control_Impl) -> Result<IDispatch> {
    control.to_interface::<IMsRdpClient10>().cast()
}

/// Encodes `value` as UTF-16 with a trailing NUL, as property-bag names require.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn bag2_description(property: &BagProperty, name: &mut [u16]) -> PROPBAG2 {
    PROPBAG2 {
        dwType: PROPBAG2_TYPE_DATA.0 as u32,
        vt: property.vt,
        pstrName: PWSTR(name.as_mut_ptr()),
        ..PROPBAG2::default()
    }
}

#[cfg(test)]
mod tests;

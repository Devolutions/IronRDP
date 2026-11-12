//! Windows backend driving the in-box RemoteFX USB redirection drivers.
//!
//! The hub filter (TsUsbFlt) lets a process claim devices, which are then re-enumerated with a
//! generic redirection driver (TsUsbGD) in place of their own, so that any device can be redirected
//! and not only WinUSB ones. The `fUsbRedirectionEnableMode` policy decides who may claim devices;
//! when it does not allow the current user, the nusb backend is used instead.

mod cfgmgr;
mod control;
mod device;
mod discovery;
mod io;

pub(crate) use discovery::start_discovery;

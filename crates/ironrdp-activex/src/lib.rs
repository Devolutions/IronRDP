//! Windows COM Automation server foundation for IronRDP.
//!
//! This crate exposes a carefully scoped compatibility surface for the classic MSTSCLib control and the modern `IRemoteDesktopClient` facade.
//! It runs the resulting RDP connection with IronRDP, implements the complete raw client-interface hierarchy through `IMsRdpClient10`, and provides the OLE contracts required for fresh windowed ActiveX hosting.
//! Unsupported semantic members continue to fail explicitly.

#![cfg_attr(windows, allow(clippy::inline_always))]
#![cfg_attr(windows, allow(clippy::as_pointer_underscore))]
#![cfg_attr(windows, allow(clippy::as_conversions))]
#![cfg_attr(windows, allow(clippy::cast_possible_truncation))]
#![cfg_attr(windows, allow(clippy::cast_possible_wrap))]
#![cfg_attr(windows, allow(clippy::cast_sign_loss))]
#![cfg_attr(windows, allow(clippy::fn_to_numeric_cast_any))]
#![cfg_attr(windows, allow(clippy::multiple_unsafe_ops_per_block))]
#![cfg_attr(windows, allow(clippy::non_zero_suggestions))]
#![cfg_attr(windows, allow(clippy::ptr_cast_constness))]
#![cfg_attr(windows, allow(clippy::renamed_function_params))]
#![cfg_attr(windows, allow(clippy::std_instead_of_core))]
#![cfg_attr(windows, allow(clippy::undocumented_unsafe_blocks))]
#![cfg_attr(windows, allow(clippy::unnecessary_safety_comment))]

#[cfg(windows)]
mod com;
#[cfg(windows)]
mod control;
#[cfg(windows)]
mod mstsc;
#[cfg(windows)]
mod registration;
#[cfg(windows)]
mod rpc;
#[cfg(windows)]
mod touch;

#[cfg(windows)]
pub use com::{
    DllCanUnloadNow, DllCancelAuthentication, DllDeleteSavedCreds, DllGetClaimsToken, DllGetClassObject,
    DllGetTscCtlVer, DllLogoffClaimsToken, DllRegisterServer, DllSetAuthProperties, DllSetClaimsToken,
    DllUnregisterServer,
};

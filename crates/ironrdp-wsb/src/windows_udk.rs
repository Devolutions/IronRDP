// The checked-in projection must retain the ABI emitted by windows-bindgen rather than conform to local lints.
#[allow(warnings)]
#[path = "windows_udk_bindings.rs"]
pub(crate) mod bindings;

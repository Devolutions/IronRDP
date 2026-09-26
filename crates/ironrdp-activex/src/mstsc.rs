//! Public MSTSCLib client-interface declarations.
//!
//! The definitions in this module are derived from the published MSTSCLib type library metadata.
//! They intentionally use raw Automation-compatible parameter types so input BSTRs remain caller
//! owned and unsupported interface-returning members can clear their out parameters safely.

#![allow(non_camel_case_types, non_snake_case)]
// The `windows_core::interface` expansion performs the ABI-preserving vtable casts below.
#![allow(clippy::too_many_arguments, clippy::transmute_ptr_to_ptr)]

use core::ffi::c_void;

use windows::Win32::System::Com::{IDispatch, IDispatch_Impl, IDispatch_Vtbl};
use windows::Win32::System::Variant::VARIANT;
use windows_core::{IUnknown, IUnknown_Vtbl, Result, interface};

pub(crate) type Bstr = *const u16;
pub(crate) type BstrOut = *mut *const u16;
pub(crate) type InterfaceOut = *mut *mut c_void;

#[interface("48A0F2A7-2713-431F-BBAC-6F4558E7D64D")]
pub(crate) unsafe trait IRemoteDesktopClientSettings: IDispatch {
    pub(crate) fn ApplySettings(&self, rdp_file_contents: Bstr) -> Result<()>;
    pub(crate) fn RetrieveSettings(&self, rdp_file_contents: BstrOut) -> Result<()>;
    pub(crate) fn GetRdpProperty(&self, property_name: Bstr, value: *mut VARIANT) -> Result<()>;
    pub(crate) fn SetRdpProperty(&self, property_name: Bstr, value: VARIANT) -> Result<()>;
}

#[interface("7D54BC4E-1028-45D4-8B0A-B9B6BFFBA176")]
pub(crate) unsafe trait IRemoteDesktopClientActions: IDispatch {
    pub(crate) fn SuspendScreenUpdates(&self) -> Result<()>;
    pub(crate) fn ResumeScreenUpdates(&self) -> Result<()>;
    pub(crate) fn ExecuteRemoteAction(&self, remote_action: i32) -> Result<()>;
    pub(crate) fn GetSnapshot(
        &self,
        snapshot_encoding: i32,
        snapshot_format: i32,
        snapshot_width: u32,
        snapshot_height: u32,
        snapshot_data: BstrOut,
    ) -> Result<()>;
}

#[interface("260EC22D-8CBC-44B5-9E88-2A37F6C93AE9")]
pub(crate) unsafe trait IRemoteDesktopClientTouchPointer: IDispatch {
    pub(crate) fn put_Enabled(&self, enabled: i16) -> Result<()>;
    pub(crate) fn get_Enabled(&self, enabled: *mut i16) -> Result<()>;
    pub(crate) fn put_EventsEnabled(&self, events_enabled: i16) -> Result<()>;
    pub(crate) fn get_EventsEnabled(&self, events_enabled: *mut i16) -> Result<()>;
    pub(crate) fn put_PointerSpeed(&self, pointer_speed: u32) -> Result<()>;
    pub(crate) fn get_PointerSpeed(&self, pointer_speed: *mut u32) -> Result<()>;
}

#[interface("57D25668-625A-4905-BE4E-304CAA13F89C")]
pub(crate) unsafe trait IRemoteDesktopClient: IDispatch {
    pub(crate) fn Connect(&self) -> Result<()>;
    pub(crate) fn Disconnect(&self) -> Result<()>;
    pub(crate) fn Reconnect(&self, width: u32, height: u32) -> Result<()>;
    pub(crate) fn get_Settings(&self, settings: InterfaceOut) -> Result<()>;
    pub(crate) fn get_Actions(&self, actions: InterfaceOut) -> Result<()>;
    pub(crate) fn get_TouchPointer(&self, touch_pointer: InterfaceOut) -> Result<()>;
    pub(crate) fn DeleteSavedCredentials(&self, server_name: Bstr) -> Result<()>;
    pub(crate) fn UpdateSessionDisplaySettings(&self, width: u32, height: u32) -> Result<()>;
    pub(crate) fn attachEvent(&self, event_name: Bstr, callback: *mut c_void) -> Result<()>;
    pub(crate) fn detachEvent(&self, event_name: Bstr, callback: *mut c_void) -> Result<()>;
}

#[interface("FDD029F9-467A-4C49-8529-64B521DBD1B4")]
pub(crate) unsafe trait ITSRemoteProgram: IDispatch {
    pub(crate) fn put_RemoteProgramMode(&self, value: i16) -> Result<()>;
    pub(crate) fn get_RemoteProgramMode(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn ServerStartProgram(
        &self,
        executable: Bstr,
        file: Bstr,
        working_directory: Bstr,
        expand_working_directory: i16,
        arguments: Bstr,
        expand_arguments: i16,
    ) -> Result<()>;
}

#[interface("92C38A7D-241A-418C-9936-099872C9AF20")]
pub(crate) unsafe trait ITSRemoteProgram2: ITSRemoteProgram {
    pub(crate) fn put_RemoteApplicationName(&self, value: Bstr) -> Result<()>;
    pub(crate) fn put_RemoteApplicationProgram(&self, value: Bstr) -> Result<()>;
    pub(crate) fn put_RemoteApplicationArgs(&self, value: Bstr) -> Result<()>;
}

#[interface("4B84EA77-ACEA-418C-881A-4A8C28AB1510")]
pub(crate) unsafe trait ITSRemoteProgram3: ITSRemoteProgram2 {
    pub(crate) fn ServerStartApp(&self, app_user_model_id: Bstr, arguments: Bstr, expand_arguments: i16) -> Result<()>;
}

#[interface("56540617-D281-488C-8738-6A8FDF64A118")]
pub(crate) unsafe trait IMsRdpDeviceCollection: IUnknown {
    pub(crate) fn RescanDevices(&self, dynamic_redirection: i16) -> Result<()>;
    fn get_DeviceByIndex(&self, index: u32, device: InterfaceOut) -> Result<()>;
    fn get_DeviceById(&self, instance_id: Bstr, device: InterfaceOut) -> Result<()>;
    pub(crate) fn get_DeviceCount(&self, count: *mut u32) -> Result<()>;
}

#[interface("7FF17599-DA2C-4677-AD35-F60C04FE1585")]
pub(crate) unsafe trait IMsRdpDriveCollection: IUnknown {
    pub(crate) fn RescanDrives(&self, dynamic_redirection: i16) -> Result<()>;
    pub(crate) fn get_DriveByIndex(&self, index: u32, drive: InterfaceOut) -> Result<()>;
    pub(crate) fn get_DriveCount(&self, count: *mut u32) -> Result<()>;
}

#[interface("D28B5458-F694-47A8-8E61-40356A767E46")]
pub(crate) unsafe trait IMsRdpDrive: IUnknown {
    pub(crate) fn get_Name(&self, name: BstrOut) -> Result<()>;
    pub(crate) fn put_RedirectionState(&self, state: i16) -> Result<()>;
    pub(crate) fn get_RedirectionState(&self, state: *mut i16) -> Result<()>;
}

#[interface("AE45252B-AAAB-4504-B681-649D6073A37A")]
pub(crate) unsafe trait IMsRdpCameraRedirConfigCollection: IUnknown {
    pub(crate) fn Rescan(&self) -> Result<()>;
    pub(crate) fn get_Count(&self, count: *mut u32) -> Result<()>;
    pub(crate) fn get_ByIndex(&self, index: u32, config: InterfaceOut) -> Result<()>;
    pub(crate) fn get_BySymbolicLink(&self, link: Bstr, config: InterfaceOut) -> Result<()>;
    pub(crate) fn get_ByInstanceId(&self, id: Bstr, config: InterfaceOut) -> Result<()>;
    pub(crate) fn AddConfig(&self, link: Bstr, redirected: i16) -> Result<()>;
    pub(crate) fn put_RedirectByDefault(&self, redirect: i16) -> Result<()>;
    pub(crate) fn get_RedirectByDefault(&self, redirect: *mut i16) -> Result<()>;
    pub(crate) fn put_EncodeVideo(&self, encode: i16) -> Result<()>;
    pub(crate) fn get_EncodeVideo(&self, encode: *mut i16) -> Result<()>;
    pub(crate) fn put_EncodingQuality(&self, quality: i32) -> Result<()>;
    pub(crate) fn get_EncodingQuality(&self, quality: *mut i32) -> Result<()>;
}

#[interface("09750604-D625-47C1-9FCD-F09F735705D7")]
pub(crate) unsafe trait IMsRdpCameraRedirConfig: IUnknown {
    pub(crate) fn get_FriendlyName(&self, name: BstrOut) -> Result<()>;
    pub(crate) fn get_SymbolicLink(&self, link: BstrOut) -> Result<()>;
    pub(crate) fn get_InstanceId(&self, id: BstrOut) -> Result<()>;
    pub(crate) fn get_ParentInstanceId(&self, id: BstrOut) -> Result<()>;
    pub(crate) fn put_Redirected(&self, redirected: i16) -> Result<()>;
    pub(crate) fn get_Redirected(&self, redirected: *mut i16) -> Result<()>;
    pub(crate) fn get_DeviceExists(&self, exists: *mut i16) -> Result<()>;
}

#[interface("FDD029F9-9574-4DEF-8529-64B521CCCAA4")]
pub(crate) unsafe trait IMsRdpPreferredRedirectionInfo: IUnknown {
    pub(crate) fn put_UseRedirectionServerName(&self, value: i16) -> Result<()>;
    pub(crate) fn get_UseRedirectionServerName(&self, value: *mut i16) -> Result<()>;
}

#[interface("2E769EE8-00C7-43DC-AFD9-235D75B72A40")]
pub(crate) unsafe trait IMsRdpClipboard: IUnknown {
    pub(crate) fn CanSyncLocalClipboardToRemoteSession(&self, can_sync: *mut i16) -> Result<()>;
    pub(crate) fn SyncLocalClipboardToRemoteSession(&self) -> Result<()>;
    pub(crate) fn CanSyncRemoteClipboardToLocalSession(&self, can_sync: *mut i16) -> Result<()>;
    pub(crate) fn SyncRemoteClipboardToLocalSession(&self) -> Result<()>;
}

#[interface("C1E6743A-41C1-4A74-832A-0DD06C1C7A0E")]
pub(crate) unsafe trait IMsTscNonScriptable: IUnknown {
    pub(crate) fn put_ClearTextPassword(&self, password: Bstr) -> Result<()>;
    fn put_PortablePassword(&self, password: Bstr) -> Result<()>;
    pub(crate) fn get_PortablePassword(&self, password: BstrOut) -> Result<()>;
    fn put_PortableSalt(&self, salt: Bstr) -> Result<()>;
    fn get_PortableSalt(&self, salt: BstrOut) -> Result<()>;
    fn put_BinaryPassword(&self, password: Bstr) -> Result<()>;
    fn get_BinaryPassword(&self, password: BstrOut) -> Result<()>;
    fn put_BinarySalt(&self, salt: Bstr) -> Result<()>;
    fn get_BinarySalt(&self, salt: BstrOut) -> Result<()>;
    pub(crate) fn ResetPassword(&self) -> Result<()>;
}

#[interface("2F079C4C-87B2-4AFD-97AB-20CDB43038AE")]
pub(crate) unsafe trait IMsRdpClientNonScriptable: IMsTscNonScriptable {
    fn NotifyRedirectDeviceChange(&self, wparam: usize, lparam: isize) -> Result<()>;
    fn SendKeys(&self, key_count: i32, key_up: *mut i16, key_data: *mut i32) -> Result<()>;
}

#[interface("17A5E535-4072-4FA4-AF32-C8D0D47345E9")]
pub(crate) unsafe trait IMsRdpClientNonScriptable2: IMsRdpClientNonScriptable {
    fn put_UIParentWindowHandle(&self, parent: isize) -> Result<()>;
    fn get_UIParentWindowHandle(&self, parent: *mut isize) -> Result<()>;
}

#[interface("B3378D90-0728-45C7-8ED7-B6159FB92219")]
pub(crate) unsafe trait IMsRdpClientNonScriptable3: IMsRdpClientNonScriptable2 {
    fn put_ShowRedirectionWarningDialog(&self, value: i16) -> Result<()>;
    fn get_ShowRedirectionWarningDialog(&self, value: *mut i16) -> Result<()>;
    fn put_PromptForCredentials(&self, value: i16) -> Result<()>;
    pub(crate) fn get_PromptForCredentials(&self, value: *mut i16) -> Result<()>;
    fn put_NegotiateSecurityLayer(&self, value: i16) -> Result<()>;
    fn get_NegotiateSecurityLayer(&self, value: *mut i16) -> Result<()>;
    fn put_EnableCredSspSupport(&self, value: i16) -> Result<()>;
    fn get_EnableCredSspSupport(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_RedirectDynamicDrives(&self, value: i16) -> Result<()>;
    pub(crate) fn get_RedirectDynamicDrives(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_RedirectDynamicDevices(&self, value: i16) -> Result<()>;
    pub(crate) fn get_RedirectDynamicDevices(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn get_DeviceCollection(&self, collection: InterfaceOut) -> Result<()>;
    pub(crate) fn get_DriveCollection(&self, collection: InterfaceOut) -> Result<()>;
    fn put_WarnAboutSendingCredentials(&self, value: i16) -> Result<()>;
    fn get_WarnAboutSendingCredentials(&self, value: *mut i16) -> Result<()>;
    fn put_WarnAboutClipboardRedirection(&self, value: i16) -> Result<()>;
    fn get_WarnAboutClipboardRedirection(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_ConnectionBarText(&self, value: Bstr) -> Result<()>;
    pub(crate) fn get_ConnectionBarText(&self, value: BstrOut) -> Result<()>;
}

#[interface("F50FA8AA-1C7D-4F59-B15C-A90CACAE1FCB")]
pub(crate) unsafe trait IMsRdpClientNonScriptable4: IMsRdpClientNonScriptable3 {
    fn put_RedirectionWarningType(&self, value: i32) -> Result<()>;
    fn get_RedirectionWarningType(&self, value: *mut i32) -> Result<()>;
    fn put_MarkRdpSettingsSecure(&self, value: i16) -> Result<()>;
    fn get_MarkRdpSettingsSecure(&self, value: *mut i16) -> Result<()>;
    fn put_PublisherCertificateChain(&self, value: *mut VARIANT) -> Result<()>;
    fn get_PublisherCertificateChain(&self, value: *mut VARIANT) -> Result<()>;
    fn put_WarnAboutPrinterRedirection(&self, value: i16) -> Result<()>;
    fn get_WarnAboutPrinterRedirection(&self, value: *mut i16) -> Result<()>;
    fn put_AllowCredentialSaving(&self, value: i16) -> Result<()>;
    fn get_AllowCredentialSaving(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_PromptForCredsOnClient(&self, value: i16) -> Result<()>;
    pub(crate) fn get_PromptForCredsOnClient(&self, value: *mut i16) -> Result<()>;
    fn put_LaunchedViaClientShellInterface(&self, value: i16) -> Result<()>;
    fn get_LaunchedViaClientShellInterface(&self, value: *mut i16) -> Result<()>;
    fn put_TrustedZoneSite(&self, value: i16) -> Result<()>;
    fn get_TrustedZoneSite(&self, value: *mut i16) -> Result<()>;
}

#[interface("4F6996D5-D7B1-412C-B0FF-063718566907")]
pub(crate) unsafe trait IMsRdpClientNonScriptable5: IMsRdpClientNonScriptable4 {
    fn put_UseMultimon(&self, value: i16) -> Result<()>;
    fn get_UseMultimon(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn get_RemoteMonitorCount(&self, count: *mut u32) -> Result<()>;
    fn GetRemoteMonitorsBoundingBox(
        &self,
        left: *mut i32,
        top: *mut i32,
        right: *mut i32,
        bottom: *mut i32,
    ) -> Result<()>;
    fn get_RemoteMonitorLayoutMatchesLocal(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_DisableConnectionBar(&self, value: i16) -> Result<()>;
    fn put_DisableRemoteAppCapsCheck(&self, value: i16) -> Result<()>;
    fn get_DisableRemoteAppCapsCheck(&self, value: *mut i16) -> Result<()>;
    fn put_WarnAboutDirectXRedirection(&self, value: i16) -> Result<()>;
    fn get_WarnAboutDirectXRedirection(&self, value: *mut i16) -> Result<()>;
    pub(crate) fn put_AllowPromptingForCredentials(&self, value: i16) -> Result<()>;
    fn get_AllowPromptingForCredentials(&self, value: *mut i16) -> Result<()>;
}

#[interface("05293249-B28B-4BD8-BE64-1B2F496B910E")]
pub(crate) unsafe trait IMsRdpClientNonScriptable6: IMsRdpClientNonScriptable5 {
    pub(crate) fn SendLocation2D(&self, latitude: f64, longitude: f64) -> Result<()>;
    pub(crate) fn SendLocation3D(&self, latitude: f64, longitude: f64, altitude: i32) -> Result<()>;
}

#[interface("71B4A60A-FE21-46D8-A39B-8E32BA0C5ECC")]
pub(crate) unsafe trait IMsRdpClientNonScriptable7: IMsRdpClientNonScriptable6 {
    pub(crate) fn get_CameraRedirConfigCollection(&self, collection: InterfaceOut) -> Result<()>;
    fn DisableDpiCursorScalingForProcess(&self) -> Result<()>;
    pub(crate) fn get_Clipboard(&self, clipboard: InterfaceOut) -> Result<()>;
}

#[interface("B2B3FA47-3F11-4148-AD24-DFF8684A16D0")]
pub(crate) unsafe trait IMsRdpClientNonScriptable8: IMsRdpClientNonScriptable7 {
    pub(crate) fn get_CorrelationId(&self, correlation_id: *mut windows_core::GUID) -> Result<()>;
    fn StartWorkspaceExtension(
        &self,
        is_web_hosted: i16,
        workspace_id: Bstr,
        publisher_thumbprint: *const u8,
        publisher_thumbprint_length: u32,
    ) -> Result<()>;
    fn put_SupportsWorkspaceReconnect(&self, value: i16) -> Result<()>;
}

#[interface("302D8188-0052-4807-806A-362B628F9AC5")]
pub(crate) unsafe trait IMsRdpExtendedSettings: IUnknown {
    pub(crate) fn put_Property(&self, name: Bstr, value: *mut VARIANT) -> Result<()>;
    pub(crate) fn get_Property(&self, name: Bstr, value: *mut VARIANT) -> Result<()>;
}

#[interface("327BB5CD-834E-4400-AEF2-B30E15E5D682")]
pub(crate) unsafe trait IMsTscAx_Redist: IDispatch {}

#[interface("8C11EFAE-92C3-11D1-BC1E-00C04FA31489")]
pub(crate) unsafe trait IMsTscAx: IMsTscAx_Redist {
    pub(crate) fn put_Server(&self, server: Bstr) -> Result<()>;
    pub(crate) fn get_Server(&self, server: BstrOut) -> Result<()>;
    fn put_Domain(&self, domain: Bstr) -> Result<()>;
    fn get_Domain(&self, domain: BstrOut) -> Result<()>;
    fn put_UserName(&self, username: Bstr) -> Result<()>;
    fn get_UserName(&self, username: BstrOut) -> Result<()>;
    fn put_DisconnectedText(&self, text: Bstr) -> Result<()>;
    fn get_DisconnectedText(&self, text: BstrOut) -> Result<()>;
    fn put_ConnectingText(&self, text: Bstr) -> Result<()>;
    fn get_ConnectingText(&self, text: BstrOut) -> Result<()>;
    fn get_Connected(&self, connected: *mut i16) -> Result<()>;
    fn put_DesktopWidth(&self, width: i32) -> Result<()>;
    fn get_DesktopWidth(&self, width: *mut i32) -> Result<()>;
    fn put_DesktopHeight(&self, height: i32) -> Result<()>;
    fn get_DesktopHeight(&self, height: *mut i32) -> Result<()>;
    fn put_StartConnected(&self, start_connected: i32) -> Result<()>;
    fn get_StartConnected(&self, start_connected: *mut i32) -> Result<()>;
    fn get_HorizontalScrollBarVisible(&self, visible: *mut i32) -> Result<()>;
    fn get_VerticalScrollBarVisible(&self, visible: *mut i32) -> Result<()>;
    fn put_FullScreenTitle(&self, title: Bstr) -> Result<()>;
    fn get_CipherStrength(&self, strength: *mut i32) -> Result<()>;
    fn get_Version(&self, version: BstrOut) -> Result<()>;
    fn get_SecuredSettingsEnabled(&self, enabled: *mut i32) -> Result<()>;
    fn get_SecuredSettings(&self, settings: InterfaceOut) -> Result<()>;
    fn get_AdvancedSettings(&self, settings: InterfaceOut) -> Result<()>;
    fn get_Debugger(&self, debugger: InterfaceOut) -> Result<()>;
    fn Connect(&self) -> Result<()>;
    fn Disconnect(&self) -> Result<()>;
    pub(crate) fn CreateVirtualChannels(&self, channels: Bstr) -> Result<()>;
    pub(crate) fn SendOnVirtualChannel(&self, channel: Bstr, data: Bstr) -> Result<()>;
}

#[interface("92B4A539-7115-4B7C-A5A9-E5D9EFC2780A")]
pub(crate) unsafe trait IMsRdpClient: IMsTscAx {
    fn put_ColorDepth(&self, color_depth: i32) -> Result<()>;
    fn get_ColorDepth(&self, color_depth: *mut i32) -> Result<()>;
    fn get_AdvancedSettings2(&self, settings: InterfaceOut) -> Result<()>;
    fn get_SecuredSettings2(&self, settings: InterfaceOut) -> Result<()>;
    fn get_ExtendedDisconnectReason(&self, reason: *mut i32) -> Result<()>;
    fn put_FullScreen(&self, fullscreen: i16) -> Result<()>;
    fn get_FullScreen(&self, fullscreen: *mut i16) -> Result<()>;
    pub(crate) fn SetVirtualChannelOptions(&self, channel: Bstr, options: i32) -> Result<()>;
    pub(crate) fn GetVirtualChannelOptions(&self, channel: Bstr, options: *mut i32) -> Result<()>;
    fn RequestClose(&self, status: *mut i32) -> Result<()>;
}

#[interface("E7E17DC4-3B71-4BA7-A8E6-281FFADCA28F")]
pub(crate) unsafe trait IMsRdpClient2: IMsRdpClient {
    fn get_AdvancedSettings3(&self, settings: InterfaceOut) -> Result<()>;
    fn put_ConnectedStatusText(&self, text: Bstr) -> Result<()>;
    fn get_ConnectedStatusText(&self, text: BstrOut) -> Result<()>;
}

#[interface("91B7CBC5-A72E-4FA0-9300-D647D7E897FF")]
pub(crate) unsafe trait IMsRdpClient3: IMsRdpClient2 {
    fn get_AdvancedSettings4(&self, settings: InterfaceOut) -> Result<()>;
}

#[interface("095E0738-D97D-488B-B9F6-DD0E8D66C0DE")]
pub(crate) unsafe trait IMsRdpClient4: IMsRdpClient3 {
    fn get_AdvancedSettings5(&self, settings: InterfaceOut) -> Result<()>;
}

#[interface("4EB5335B-6429-477D-B922-E06A28ECD8BF")]
pub(crate) unsafe trait IMsRdpClient5: IMsRdpClient4 {
    fn get_TransportSettings(&self, settings: InterfaceOut) -> Result<()>;
    fn get_AdvancedSettings6(&self, settings: InterfaceOut) -> Result<()>;
    fn GetErrorDescription(&self, disconnect_reason: u32, extended_reason: u32, message: BstrOut) -> Result<()>;
    pub(crate) fn get_RemoteProgram(&self, program: InterfaceOut) -> Result<()>;
    fn get_MsRdpClientShell(&self, shell: InterfaceOut) -> Result<()>;
}

#[interface("D43B7D80-8517-4B6D-9EAC-96AD6800D7F2")]
pub(crate) unsafe trait IMsRdpClient6: IMsRdpClient5 {
    pub(crate) fn get_AdvancedSettings7(&self, settings: InterfaceOut) -> Result<()>;
    fn get_TransportSettings2(&self, settings: InterfaceOut) -> Result<()>;
}

#[interface("B2A5B5CE-3461-444A-91D4-ADD26D070638")]
pub(crate) unsafe trait IMsRdpClient7: IMsRdpClient6 {
    fn get_AdvancedSettings8(&self, settings: InterfaceOut) -> Result<()>;
    fn get_TransportSettings3(&self, settings: InterfaceOut) -> Result<()>;
    fn GetStatusText(&self, status: u32, text: BstrOut) -> Result<()>;
    fn get_SecuredSettings3(&self, settings: InterfaceOut) -> Result<()>;
    pub(crate) fn get_RemoteProgram2(&self, program: InterfaceOut) -> Result<()>;
}

#[interface("4247E044-9271-43A9-BC49-E2AD9E855D62")]
pub(crate) unsafe trait IMsRdpClient8: IMsRdpClient7 {
    pub(crate) fn SendRemoteAction(&self, action: i32) -> Result<()>;
    fn get_AdvancedSettings9(&self, settings: InterfaceOut) -> Result<()>;
    fn Reconnect(&self, width: u32, height: u32, status: *mut i32) -> Result<()>;
}

#[interface("28904001-04B6-436C-A55B-0AF1A0883DC9")]
pub(crate) unsafe trait IMsRdpClient9: IMsRdpClient8 {
    fn get_TransportSettings4(&self, settings: InterfaceOut) -> Result<()>;
    pub(crate) fn SyncSessionDisplaySettings(&self) -> Result<()>;
    fn UpdateSessionDisplaySettings(
        &self,
        desktop_width: u32,
        desktop_height: u32,
        physical_width: u32,
        physical_height: u32,
        orientation: u32,
        desktop_scale_factor: u32,
        device_scale_factor: u32,
    ) -> Result<()>;
    fn attachEvent(&self, event_name: Bstr, callback: *mut c_void) -> Result<()>;
    fn detachEvent(&self, event_name: Bstr, callback: *mut c_void) -> Result<()>;
}

#[interface("7ED92C39-EB38-4927-A70A-708AC5A59321")]
pub(crate) unsafe trait IMsRdpClient10: IMsRdpClient9 {
    pub(crate) fn get_RemoteProgram3(&self, program: InterfaceOut) -> Result<()>;
}

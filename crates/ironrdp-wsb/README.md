# ironrdp-wsb

Experimental Windows-only bindings for directly managing the private Windows Sandbox
Undocked Development Kit (UDK) VM lifecycle.

This crate is intentionally limited to creating, starting, inspecting, and explicitly
terminating a `ManagedWindowsVM`. It does not reproduce `WindowsSandboxServer.exe` guest
provisioning, policy handling, folder sharing, RDP configuration generation, or RDP transport.
It therefore cannot yet create an RDP-usable Windows Sandbox independently.

The API is unsupported and version-sensitive. It requires the Windows Sandbox runtime and its
private `WindowsUdk.Security.Isolation` WinRT implementation to be registered on the host.
Call `SandboxRuntime::initialize` with the `Microsoft.WindowsAppRuntime.dll` from the installed
Windows Sandbox package before creating a VM.

On the tested host, the installed DLL has a conditional executable-mapping ACL requiring the
`MicrosoftWindows.WindowsSandbox_cw5n1h2txyewy` package identity. An unpackaged process receives
`E_ACCESSDENIED` from `LoadLibraryW` before `WindowsAppRuntime_EnsureIsLoaded` runs; that export is
a no-op returning `S_OK`. An application with its own package identity does not receive the
Sandbox package identity. Copying the DLL was useful only as a diagnostic and is not a supported
or redistributable bootstrap strategy.

`WindowsUdk.Security.Isolation.ManagedWindowsVM` is backed by the registered out-of-process WinRT
server `C:\Windows\System32\ManagedWindowsVM.exe`. That server calls the private Container Manager
(`Cms*`) APIs that create and run the underlying container. This crate invokes the private
orchestration ABI; it does not replace the privileged container backend.

## Generated bindings

`src/windows_udk_bindings.rs` was generated with `windows-bindgen` 0.62.1 from the installed
`windowsudk.winmd` version 10.0.26100.1 (SHA-256
`65381108EA5512CC45EBEB2D4E29F85BA1F41E05EE020D2AF114F157BE171B74`) using the standard
Windows metadata plus these UDK filters:

```text
WindowsUdk.Security.Isolation.ManagedWindowsVM
WindowsUdk.Security.Isolation.VMNetworkInformation
WindowsUdk.Security.Isolation.VMNetworkInterface
WindowsUdk.Security.Isolation.VMNetworkingMode
WindowsUdk.Security.Isolation.VMOptions
WindowsUdk.Security.Isolation.VMRunningReference
```

The private WinMD is deliberately not included, so CI does not need the Undocked Development
Kit. Regenerate the checked-in projection only as a deliberate compatibility update.

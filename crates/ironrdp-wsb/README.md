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

An unpackaged executable is not currently sufficient: on the tested host,
`WindowsAppRuntime_EnsureIsLoaded` returns `E_ACCESSDENIED` even when the process points at the
installed package directory. Direct activation must therefore run from a compatible full-trust
package-identity context before this crate can create a VM.

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

use std::path::Path;

use windows::Win32::System::{
    LibraryLoader::{GetProcAddress, LoadLibraryW},
    WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
};
use windows_core::{Error, GUID, HRESULT, HSTRING, s};

use crate::Result;
use crate::windows_udk::bindings::WindowsUdk::Security::Isolation::{
    ManagedWindowsVM, VMNetworkingMode, VMOptions, VMRunningReference,
};

/// The private Windows Sandbox runtime initialized for a package-identity process.
///
/// The Windows App Runtime library must remain loaded for the process lifetime. This type performs
/// the same `WindowsAppRuntime_EnsureIsLoaded` call made by `WindowsSandboxServer.exe`.
#[derive(Debug)]
pub struct SandboxRuntime;

impl SandboxRuntime {
    /// Initializes the Windows App Runtime library shipped with the Windows Sandbox package.
    ///
    /// `windows_app_runtime_path` must name that package's `Microsoft.WindowsAppRuntime.dll`. The
    /// calling process must have a compatible Windows Sandbox full-trust package identity.
    pub fn initialize(windows_app_runtime_path: &Path) -> Result<Self> {
        let windows_app_runtime_path = HSTRING::from(windows_app_runtime_path.to_string_lossy().as_ref());

        // SAFETY: The path is NUL-free because HSTRING owns a Windows string. The module is deliberately
        // retained for the process lifetime so the private WinRT classes remain available after initialization.
        let module = unsafe { LoadLibraryW(&windows_app_runtime_path)? };
        // SAFETY: `module` was returned by LoadLibraryW and the requested export name is NUL-terminated.
        let procedure =
            unsafe { GetProcAddress(module, s!("WindowsAppRuntime_EnsureIsLoaded")).ok_or_else(Error::from_thread)? };

        type EnsureIsLoaded = unsafe extern "system" fn() -> HRESULT;

        // SAFETY: The package's managed projection declares this exact export with the same system ABI.
        let ensure_is_loaded: EnsureIsLoaded = unsafe { core::mem::transmute(procedure) };
        // SAFETY: `ensure_is_loaded` was resolved from the loaded Windows App Runtime module with its documented ABI.
        if let Err(error) = unsafe { ensure_is_loaded().ok() } {
            return Err(Error::new(
                error.code(),
                "Windows App Runtime initialization failed; the process may require Windows Sandbox package identity",
            ));
        }

        Ok(Self)
    }

    /// Creates a VM with the UDK runtime's default options.
    pub fn create_default_vm(&self) -> Result<SandboxVm> {
        SandboxVm::create_default()
    }
}

/// An unstarted Windows Sandbox VM created through the private UDK runtime.
///
/// The VM is not RDP-usable until the missing guest-provisioning workflow has been replicated.
#[derive(Debug)]
pub struct SandboxVm {
    vm: ManagedWindowsVM,
}

impl SandboxVm {
    /// Creates a VM with the UDK runtime's default options.
    fn create_default() -> Result<Self> {
        let _apartment = Mta::initialize()?;
        let options = VMOptions::new()?;
        let vm = ManagedWindowsVM::CreateInstance(&options)?;

        Ok(Self { vm })
    }

    /// Gets the VM identifier assigned by the UDK runtime.
    pub fn id(&self) -> Result<GUID> {
        let _apartment = Mta::initialize()?;
        self.vm.Id()
    }

    /// Gets the default guest account name selected by the UDK runtime.
    pub fn default_user_name(&self) -> Result<String> {
        let _apartment = Mta::initialize()?;
        Ok(self.vm.DefaultUserName()?.to_string_lossy())
    }

    /// Starts the VM and retains its running reference for the returned handle's lifetime.
    pub fn start(self) -> Result<RunningSandboxVm> {
        let _apartment = Mta::initialize()?;
        let running_reference = self.vm.CreateRunningReference()?;

        Ok(RunningSandboxVm {
            vm: self.vm,
            running_reference,
        })
    }
}

/// A started Windows Sandbox VM with its required UDK running-reference lease.
#[derive(Debug)]
pub struct RunningSandboxVm {
    vm: ManagedWindowsVM,
    running_reference: VMRunningReference,
}

impl RunningSandboxVm {
    /// Gets the VM identifier assigned by the UDK runtime.
    pub fn id(&self) -> Result<GUID> {
        let _apartment = Mta::initialize()?;
        self.vm.Id()
    }

    /// Gets the default guest account name selected by the UDK runtime.
    pub fn default_user_name(&self) -> Result<String> {
        let _apartment = Mta::initialize()?;
        Ok(self.vm.DefaultUserName()?.to_string_lossy())
    }

    /// Returns the UDK runtime's current networking information for the VM.
    pub fn network_information(&self) -> Result<NetworkInformation> {
        let _apartment = Mta::initialize()?;
        let network = self.vm.GetNetworkInformation(&self.running_reference)?;
        let interfaces = network.Interfaces()?;
        let mut ip_addresses = Vec::new();

        for index in 0..interfaces.Size()? {
            let interface = interfaces.GetAt(index)?;
            let addresses = interface.IPAddresses()?;

            for address_index in 0..addresses.Size()? {
                ip_addresses.push(addresses.GetAt(address_index)?.to_string_lossy());
            }
        }

        let mode = match network.Mode()? {
            VMNetworkingMode::None => NetworkingMode::None,
            VMNetworkingMode::Nat => NetworkingMode::Nat,
            unknown => NetworkingMode::Unknown(unknown.0),
        };

        Ok(NetworkInformation { mode, ip_addresses })
    }

    /// Terminates the VM and releases the running-reference lease.
    pub fn terminate(self) -> Result<()> {
        let _apartment = Mta::initialize()?;
        let terminate_result = self.vm.Terminate();
        let close_result = self.running_reference.Close();

        match (terminate_result, close_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(terminate_error), Err(close_error)) => Err(Error::new(
                terminate_error.code(),
                format!(
                    "terminating the Windows Sandbox VM failed ({terminate_error}); closing its running reference also failed ({close_error})"
                ),
            )),
        }
    }
}

/// The UDK runtime's reported VM networking mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkingMode {
    /// The VM has no UDK-managed network connection.
    None,
    /// The VM is attached to UDK-managed NAT networking.
    Nat,
    /// The UDK runtime reported a networking mode unknown to this crate.
    Unknown(i32),
}

/// Networking information reported by the UDK runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkInformation {
    /// The UDK runtime's reported networking mode.
    pub mode: NetworkingMode,
    /// Addresses reported across the VM's network interfaces.
    pub ip_addresses: Vec<String>,
}

struct Mta;

impl Mta {
    fn initialize() -> Result<Self> {
        // SAFETY: The guard is dropped on this calling thread, pairing this initialization with RoUninitialize.
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }

        Ok(Self)
    }
}

impl Drop for Mta {
    fn drop(&mut self) {
        // SAFETY: Mta is only constructed after a successful RoInitialize call on this thread.
        unsafe {
            RoUninitialize();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::SandboxRuntime;

    #[test]
    #[ignore = "creates and terminates a real Windows Sandbox VM through the private UDK runtime"]
    fn direct_udk_lifecycle() {
        let windows_app_runtime_path = std::env::var("IRONRDP_WSB_WINDOWS_APP_RUNTIME")
            .expect("IRONRDP_WSB_WINDOWS_APP_RUNTIME must point to Microsoft.WindowsAppRuntime.dll");
        let runtime = SandboxRuntime::initialize(Path::new(&windows_app_runtime_path))
            .expect("the Windows App Runtime should initialize");
        let vm = runtime.create_default_vm().expect("the UDK runtime should create a VM");
        let running_vm = vm.start().expect("the UDK runtime should start the VM");

        running_vm.terminate().expect("the UDK runtime should terminate the VM");
    }
}

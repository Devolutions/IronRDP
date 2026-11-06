# IronRDP TermSrv MSI prototype

This experimental WixSharp package registers `ironrdp-termsrv.exe` as an SCM service.
The executable does not implement SCM service hosting, so this MSI is not a supported deployment path and must not be used to start the companion.

Use the [scheduled-task deployment script](../../crates/ironrdp-termsrv/scripts/deploy-testvm-psremoting.ps1) to run the companion.
Use the [side-by-side setup guide](../../crates/ironrdp-wtsprotocol-provider/SIDEBYSIDE_SETUP.md) to register the WTS protocol provider.

## Build the prototype

Build from the repository root:

```powershell
cargo build -p ironrdp-termsrv --release
cargo build -p ironrdp-wtsprotocol-provider --release
.\server\termsrv\package\CevicheServiceWindowsManaged\build-ceviche-service-msi.ps1 -ServiceExePath .\target\release\ironrdp-termsrv.exe -ProviderDllPath .\target\release\ironrdp_wtsprotocol_provider.dll -Platform x64 -Configuration Release
```

The MSI is written to `server\termsrv\package\CevicheServiceWindowsManaged\Release\IronRdpTermSrv.msi`.
Building it does not make the unsupported SCM startup path usable.

## Remove an older installation

If an older prototype MSI is installed on a test VM, remove it with:

```powershell
msiexec /x .\IronRdpTermSrv.msi /qn /norestart
```

## Build inputs

- `IRDP_TERMSRV_SERVICE_EXECUTABLE` (required): absolute path to the service executable.
- `IRDP_PROVIDER_DLL` (optional): absolute path to `ironrdp_wtsprotocol_provider.dll`.
- `IRDP_TERMSRV_CONFIG_DIR` (optional): directory copied as `config\`.
- `IRDP_TERMSRV_MSI_VERSION` (optional): MSI version with `major.minor.build` constraints.
- `IRDP_TERMSRV_MSI_PLATFORM` (required in Release): `x64`, `x86`, or `arm64`.

Legacy environment variables remain accepted:

- `IRDP_CEVICHE_SERVICE_EXECUTABLE`
- `IRDP_CEVICHE_CONFIG_DIR`
- `IRDP_CEVICHE_MSI_VERSION`
- `IRDP_CEVICHE_MSI_PLATFORM`

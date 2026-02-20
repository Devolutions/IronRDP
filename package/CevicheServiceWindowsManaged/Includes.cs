using System;

namespace IronRdpTermSrvInstaller;

internal static class Includes
{
    internal const string VENDOR_NAME = "IronRDP";
    internal const string PRODUCT_NAME = "IronRDP TermSrv";
    internal const string SHORT_NAME = "TermSrv";

    internal const string SERVICE_NAME = "IronRdpTermSrv";
    internal const string SERVICE_DISPLAY_NAME = "IronRDP TermSrv";
    internal const string SERVICE_DESCRIPTION = "IronRDP TermSrv background service";

    internal const string SERVICE_EXECUTABLE_NAME = "ironrdp-termsrv.exe";
    internal const string PROVIDER_DLL_NAME = "ironrdp_wtsprotocol_provider.dll";

    internal const string INFO_URL = "https://github.com/Devolutions/IronRDP";

    internal static readonly Guid UPGRADE_CODE = new("8bb32736-8f9d-4eb2-b12f-f4d104009bc4");
}

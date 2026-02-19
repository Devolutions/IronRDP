using System;

namespace IronRdpCevicheInstaller;

internal static class Includes
{
    internal const string VENDOR_NAME = "IronRDP";
    internal const string PRODUCT_NAME = "IronRDP Ceviche Service";
    internal const string SHORT_NAME = "CevicheService";

    internal const string SERVICE_NAME = "IronRdpCevicheService";
    internal const string SERVICE_DISPLAY_NAME = "IronRDP Ceviche Service";
    internal const string SERVICE_DESCRIPTION = "IronRDP Ceviche-based background service";

    internal const string SERVICE_EXECUTABLE_NAME = "ironrdp-ceviche-service.exe";
    internal const string PROVIDER_DLL_NAME = "ironrdp_wtsprotocol_provider.dll";

    internal const string INFO_URL = "https://github.com/Devolutions/IronRDP";

    internal static readonly Guid UPGRADE_CODE = new("8bb32736-8f9d-4eb2-b12f-f4d104009bc4");
}

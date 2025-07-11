namespace Devolutions.IronRdp;

public class RdCleanPathConfig
{
    public Uri GatewayUri { get; private set; }

    public string  AuthToken { get; private set; }

    public RdCleanPathConfig(Uri url, string authToken)
    {
        GatewayUri = url;
        AuthToken = authToken;
    }
}
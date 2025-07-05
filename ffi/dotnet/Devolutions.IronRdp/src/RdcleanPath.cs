using System;

public class RdcleanPathConfig
{
    public Uri GatewayUri { get; private set; }

    public string  AuthToken { get; private set; }

    public RdcleanPathConfig(Uri url, string authToken)
    {
        GatewayUri = url;
        AuthToken = authToken;
    }
}
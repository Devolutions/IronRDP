using System;

public class RdcleanPathConfig
{
    public Uri Uri { get; private set; }

    public string  AuthToken { get; private set; }

    public RdcleanPathConfig(Uri url, string authToken)
    {
        Uri = url;
        AuthToken = authToken;
    }
}
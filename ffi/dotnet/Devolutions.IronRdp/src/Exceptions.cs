namespace Devolutions.IronRdp;

[Serializable]
public class IronRdpLibException : Exception
{
    public IronRdpLibExceptionType ErrorType { get; private set; }

    public IronRdpLibException(IronRdpLibExceptionType errorType, string message) : base(message)
    {
        ErrorType = errorType;
    }

}

/// <summary>
/// The server sent a Server Redirection PDU instead of completing the connection (MS-RDPBCGR 2.2.13).
/// </summary>
/// <remarks>
/// Reconnect to the target that <see cref="Redirection"/> describes, sending its load-balancing information as the
/// routing token, its session ID in the cluster data and its password as the Client Info password.
/// </remarks>
public class ServerRedirectionException : IronRdpLibException
{
    public ServerRedirection Redirection { get; private set; }

    public ServerRedirectionException(ServerRedirection redirection)
        : base(IronRdpLibExceptionType.ServerRedirected, "The server redirected the connection")
    {
        Redirection = redirection;
    }
}

public enum IronRdpLibExceptionType
{
    CannotResolveDns,
    ConnectionFailed,
    EndOfFile,
    ServerRedirected,
}
namespace Devolutions.IronRdp;

[Serializable]
public class IronRdpLibException : Exception
{
    public IronRdpLibExceptionType Type { get; private set; }

    public IronRdpLibException(IronRdpLibExceptionType type, string message) : base(message)
    {
        Type = type;
    }

}

public enum IronRdpLibExceptionType
{
    CannotResolveDns,
    ConnectionFailed,
    EndOfFile,
}
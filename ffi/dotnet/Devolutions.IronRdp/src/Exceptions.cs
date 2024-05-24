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

public enum IronRdpLibExceptionType
{
    CannotResolveDns,
    ConnectionFailed,
    EndOfFile,
}
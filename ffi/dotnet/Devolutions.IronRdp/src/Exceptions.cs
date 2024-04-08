[Serializable]
public class IronRdpLibException : Exception
{
    public IronRdpLibExceptionType type { get; private set; }

    public IronRdpLibException(IronRdpLibExceptionType type, string message): base(message)
    {
        this.type = type;
    }

}

public enum IronRdpLibExceptionType
{
    CannotResolveDns,
    ConnectionFailed,
    EndOfFile,
}
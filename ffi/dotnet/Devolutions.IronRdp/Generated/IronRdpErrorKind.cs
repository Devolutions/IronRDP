namespace Devolutions.IronRdp;

public enum IronRdpErrorKind : int
{
    Generic = 0,
    PduError = 1,
    EncodeError = 2,
    DecodeError = 3,
    CredsspError = 4,
    Consumed = 5,
    Io = 6,
    AccessDenied = 7,
    IncorrectEnumType = 8,
    Clipboard = 9,
    WrongOs = 10,
}
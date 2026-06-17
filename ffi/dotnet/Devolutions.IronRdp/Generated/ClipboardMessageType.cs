namespace Devolutions.IronRdp;

public enum ClipboardMessageType : int
{
    SendInitiateCopy = 0,
    SendFormatData = 1,
    SendInitiatePaste = 2,
    SendFileContentsRequest = 3,
    SendFileContentsResponse = 4,
    Error = 5,
}
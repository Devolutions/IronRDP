namespace Devolutions.IronRdp;

public enum ClipboardMessageType : int
{
    SendInitiateCopy = 0,
    SendInitiateFileCopy = 1,
    SendFormatData = 2,
    SendInitiatePaste = 3,
    SendFileContentsRequest = 4,
    SendFileContentsResponse = 5,
    Error = 6,
}
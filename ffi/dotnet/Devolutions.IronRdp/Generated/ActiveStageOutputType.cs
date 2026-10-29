namespace Devolutions.IronRdp;

public enum ActiveStageOutputType : int
{
    ResponseFrame = 0,
    GraphicsUpdate = 1,
    PointerDefault = 2,
    PointerHidden = 3,
    PointerPosition = 4,
    PointerBitmap = 5,
    Terminate = 6,
    DeactivateAll = 7,
    MultitransportRequest = 8,
    AutoDetect = 9,
    SaveSessionInfo = 10,
    AutoReconnectCookie = 11,
    WindowingOrders = 12,
    AutoReconnectFailed = 13,
    MonitorLayout = 14,
    WindowSurfaceUpdates = 15,
    ServerRedirection = 16,
    LogonErrors = 17,
    SessionId = 18,
}
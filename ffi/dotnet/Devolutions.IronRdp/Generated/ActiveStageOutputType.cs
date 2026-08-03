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
}
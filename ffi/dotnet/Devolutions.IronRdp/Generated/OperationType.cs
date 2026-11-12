namespace Devolutions.IronRdp;

public enum OperationType : int
{
    MouseButtonPressed = 0,
    MouseButtonReleased = 1,
    MouseMove = 2,
    WheelRotations = 3,
    KeyPressed = 4,
    KeyReleased = 5,
    UnicodeKeyPressed = 6,
    UnicodeKeyReleased = 7,
}
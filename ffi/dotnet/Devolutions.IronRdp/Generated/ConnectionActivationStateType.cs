namespace Devolutions.IronRdp;

public enum ConnectionActivationStateType : int
{
    Consumed = 0,
    CapabilitiesExchange = 1,
    ConnectionFinalization = 2,
    Finalized = 3,
}
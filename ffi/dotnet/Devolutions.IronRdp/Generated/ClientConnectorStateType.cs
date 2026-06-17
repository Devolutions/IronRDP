namespace Devolutions.IronRdp;

public enum ClientConnectorStateType : int
{
    Consumed = 0,
    ConnectionInitiationSendRequest = 1,
    ConnectionInitiationWaitConfirm = 2,
    EnhancedSecurityUpgrade = 3,
    Credssp = 4,
    BasicSettingsExchangeSendInitial = 5,
    BasicSettingsExchangeWaitResponse = 6,
    ChannelConnection = 7,
    SecureSettingsExchange = 8,
    ConnectTimeAutoDetection = 9,
    LicensingExchange = 10,
    MultitransportBootstrapping = 11,
    CapabilitiesExchange = 12,
    ConnectionFinalization = 13,
    Connected = 14,
}
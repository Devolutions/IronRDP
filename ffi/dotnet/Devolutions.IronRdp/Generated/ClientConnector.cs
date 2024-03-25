// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class ClientConnector: IDisposable
{
    private unsafe Raw.ClientConnector* _inner;

    /// <summary>
    /// Creates a managed <c>ClientConnector</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe ClientConnector(Raw.ClientConnector* handle)
    {
        _inner = handle;
    }

    /// <returns>
    /// A <c>ClientConnector</c> allocated on Rust side.
    /// </returns>
    public static ClientConnector New(Config config)
    {
        unsafe
        {
            Raw.Config* configRaw;
            configRaw = config.AsFFI();
            if (configRaw == null)
            {
                throw new ObjectDisposedException("Config");
            }
            Raw.ClientConnector* retVal = Raw.ClientConnector.New(configRaw);
            return new ClientConnector(retVal);
        }
    }

    /// <summary>
    /// Must use
    /// </summary>
    /// <exception cref="IronRdpException"></exception>
    public void WithServerAddr(SocketAddr serverAddr)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.SocketAddr* serverAddrRaw;
            serverAddrRaw = serverAddr.AsFFI();
            if (serverAddrRaw == null)
            {
                throw new ObjectDisposedException("SocketAddr");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.WithServerAddr(_inner, serverAddrRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <summary>
    /// Must use
    /// </summary>
    /// <exception cref="IronRdpException"></exception>
    public void WithStaticChannelRdpSnd()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.WithStaticChannelRdpSnd(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <summary>
    /// Must use
    /// </summary>
    /// <exception cref="IronRdpException"></exception>
    public void WithStaticChannelRdpdr(string computerName, uint smartCardDeviceId)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            byte[] computerNameBuf = DiplomatUtils.StringToUtf8(computerName);
            nuint computerNameBufLength = (nuint)computerNameBuf.Length;
            fixed (byte* computerNameBufPtr = computerNameBuf)
            {
                Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.WithStaticChannelRdpdr(_inner, computerNameBufPtr, computerNameBufLength, smartCardDeviceId);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public bool ShouldPerformSecurityUpgrade()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultBoolBoxIronRdpError result = Raw.ClientConnector.ShouldPerformSecurityUpgrade(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            bool retVal = result.Ok;
            return retVal;
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public void MarkSecurityUpgradeAsDone()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.MarkSecurityUpgradeAsDone(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public bool ShouldPerformCredssp()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultBoolBoxIronRdpError result = Raw.ClientConnector.ShouldPerformCredssp(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            bool retVal = result.Ok;
            return retVal;
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public void MarkCredsspAsDone()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.MarkCredsspAsDone(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <returns>
    /// A <c>PduHintResult</c> allocated on Rust side.
    /// </returns>
    public PduHintResult NextPduHint()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.PduHintResult* retVal = Raw.ClientConnector.NextPduHint(_inner);
            return new PduHintResult(retVal);
        }
    }

    /// <returns>
    /// A <c>State</c> allocated on Rust side.
    /// </returns>
    public State State()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.State* retVal = Raw.ClientConnector.State(_inner);
            return new State(retVal);
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.ClientConnector* AsFFI()
    {
        return _inner;
    }

    /// <summary>
    /// Destroys the underlying object immediately.
    /// </summary>
    public void Dispose()
    {
        unsafe
        {
            if (_inner == null)
            {
                return;
            }

            Raw.ClientConnector.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~ClientConnector()
    {
        Dispose();
    }
}

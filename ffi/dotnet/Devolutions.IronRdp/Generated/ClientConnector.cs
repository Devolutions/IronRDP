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

    public DynState DynState
    {
        get
        {
            return GetDynState();
        }
    }

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
    public void WithClientAddr(string clientAddr)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            byte[] clientAddrBuf = DiplomatUtils.StringToUtf8(clientAddr);
            nuint clientAddrBufLength = (nuint)clientAddrBuf.Length;
            fixed (byte* clientAddrBufPtr = clientAddrBuf)
            {
                Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.WithClientAddr(_inner, clientAddrBufPtr, clientAddrBufLength);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
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
    public void WithDynamicChannelDisplayControl()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.WithDynamicChannelDisplayControl(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
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

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>Written</c> allocated on Rust side.
    /// </returns>
    public Written Step(byte[] input, WriteBuf writeBuf)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            nuint inputLength = (nuint)input.Length;
            Raw.WriteBuf* writeBufRaw;
            writeBufRaw = writeBuf.AsFFI();
            if (writeBufRaw == null)
            {
                throw new ObjectDisposedException("WriteBuf");
            }
            fixed (byte* inputPtr = input)
            {
                Raw.ConnectorFfiResultBoxWrittenBoxIronRdpError result = Raw.ClientConnector.Step(_inner, inputPtr, inputLength, writeBufRaw);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
                Raw.Written* retVal = result.Ok;
                return new Written(retVal);
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>Written</c> allocated on Rust side.
    /// </returns>
    public Written StepNoInput(WriteBuf writeBuf)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.WriteBuf* writeBufRaw;
            writeBufRaw = writeBuf.AsFFI();
            if (writeBufRaw == null)
            {
                throw new ObjectDisposedException("WriteBuf");
            }
            Raw.ConnectorFfiResultBoxWrittenBoxIronRdpError result = Raw.ClientConnector.StepNoInput(_inner, writeBufRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.Written* retVal = result.Ok;
            return new Written(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public void AttachStaticCliprdr(Cliprdr cliprdr)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.Cliprdr* cliprdrRaw;
            cliprdrRaw = cliprdr.AsFFI();
            if (cliprdrRaw == null)
            {
                throw new ObjectDisposedException("Cliprdr");
            }
            Raw.ConnectorFfiResultVoidBoxIronRdpError result = Raw.ClientConnector.AttachStaticCliprdr(_inner, cliprdrRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>PduHint</c> allocated on Rust side.
    /// </returns>
    public PduHint NextPduHint()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultOptBoxPduHintBoxIronRdpError result = Raw.ClientConnector.NextPduHint(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.PduHint* retVal = result.Ok;
            if (retVal == null)
            {
                return null;
            }
            return new PduHint(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>DynState</c> allocated on Rust side.
    /// </returns>
    public DynState GetDynState()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultBoxDynStateBoxIronRdpError result = Raw.ClientConnector.GetDynState(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.DynState* retVal = result.Ok;
            return new DynState(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>ClientConnectorState</c> allocated on Rust side.
    /// </returns>
    public ClientConnectorState ConsumeAndCastToClientConnectorState()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ClientConnector");
            }
            Raw.ConnectorFfiResultBoxClientConnectorStateBoxIronRdpError result = Raw.ClientConnector.ConsumeAndCastToClientConnectorState(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.ClientConnectorState* retVal = result.Ok;
            return new ClientConnectorState(retVal);
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

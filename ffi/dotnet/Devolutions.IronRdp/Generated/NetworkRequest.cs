// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class NetworkRequest: IDisposable
{
    private unsafe Raw.NetworkRequest* _inner;

    public VecU8 Data
    {
        get
        {
            return GetData();
        }
    }

    public NetworkRequestProtocol Protocol
    {
        get
        {
            return GetProtocol();
        }
    }

    public string Url
    {
        get
        {
            return GetUrl();
        }
    }

    /// <summary>
    /// Creates a managed <c>NetworkRequest</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe NetworkRequest(Raw.NetworkRequest* handle)
    {
        _inner = handle;
    }

    /// <returns>
    /// A <c>VecU8</c> allocated on Rust side.
    /// </returns>
    public VecU8 GetData()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("NetworkRequest");
            }
            Raw.VecU8* retVal = Raw.NetworkRequest.GetData(_inner);
            return new VecU8(retVal);
        }
    }

    /// <returns>
    /// A <c>NetworkRequestProtocol</c> allocated on C# side.
    /// </returns>
    public NetworkRequestProtocol GetProtocol()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("NetworkRequest");
            }
            Raw.NetworkRequestProtocol retVal = Raw.NetworkRequest.GetProtocol(_inner);
            return (NetworkRequestProtocol)retVal;
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public void GetUrl(DiplomatWriteable writeable)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("NetworkRequest");
            }
            Raw.CredsspNetworkFfiResultVoidBoxIronRdpError result = Raw.NetworkRequest.GetUrl(_inner, &writeable);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    public string GetUrl()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("NetworkRequest");
            }
            DiplomatWriteable writeable = new DiplomatWriteable();
            Raw.CredsspNetworkFfiResultVoidBoxIronRdpError result = Raw.NetworkRequest.GetUrl(_inner, &writeable);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            string retVal = writeable.ToUnicode();
            writeable.Dispose();
            return retVal;
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.NetworkRequest* AsFFI()
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

            Raw.NetworkRequest.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~NetworkRequest()
    {
        Dispose();
    }
}
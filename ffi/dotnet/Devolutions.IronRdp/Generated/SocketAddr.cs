// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class SocketAddr: IDisposable
{
    private unsafe Raw.SocketAddr* _inner;

    /// <summary>
    /// Creates a managed <c>SocketAddr</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe SocketAddr(Raw.SocketAddr* handle)
    {
        _inner = handle;
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>SocketAddr</c> allocated on Rust side.
    /// </returns>
    public static SocketAddr LookUp(string host, ushort port)
    {
        unsafe
        {
            byte[] hostBuf = DiplomatUtils.StringToUtf8(host);
            nuint hostBufLength = (nuint)hostBuf.Length;
            fixed (byte* hostBufPtr = hostBuf)
            {
                Raw.UtilsFfiResultBoxSocketAddrBoxIronRdpError result = Raw.SocketAddr.LookUp(hostBufPtr, hostBufLength, port);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
                Raw.SocketAddr* retVal = result.Ok;
                return new SocketAddr(retVal);
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>SocketAddr</c> allocated on Rust side.
    /// </returns>
    public static SocketAddr FromFfiStr(string addr)
    {
        unsafe
        {
            byte[] addrBuf = DiplomatUtils.StringToUtf8(addr);
            nuint addrBufLength = (nuint)addrBuf.Length;
            fixed (byte* addrBufPtr = addrBuf)
            {
                Raw.UtilsFfiResultBoxSocketAddrBoxIronRdpError result = Raw.SocketAddr.FromFfiStr(addrBufPtr, addrBufLength);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
                Raw.SocketAddr* retVal = result.Ok;
                return new SocketAddr(retVal);
            }
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.SocketAddr* AsFFI()
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

            Raw.SocketAddr.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~SocketAddr()
    {
        Dispose();
    }
}

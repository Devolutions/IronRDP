// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Interop.Diplomat;
#pragma warning restore 0105

namespace Interop;

#nullable enable

public partial class ClientConnectorState: IDisposable
{
    private unsafe Raw.ClientConnectorState* _inner;

    /// <summary>
    /// Creates a managed <c>ClientConnectorState</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe ClientConnectorState(Raw.ClientConnectorState* handle)
    {
        _inner = handle;
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.ClientConnectorState* AsFFI()
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

            Raw.ClientConnectorState.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~ClientConnectorState()
    {
        Dispose();
    }
}

// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class MousePosition: IDisposable
{
    private unsafe Raw.MousePosition* _inner;

    /// <summary>
    /// Creates a managed <c>MousePosition</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe MousePosition(Raw.MousePosition* handle)
    {
        _inner = handle;
    }

    /// <returns>
    /// A <c>MousePosition</c> allocated on Rust side.
    /// </returns>
    public static MousePosition New(ushort x, ushort y)
    {
        unsafe
        {
            Raw.MousePosition* retVal = Raw.MousePosition.New(x, y);
            return new MousePosition(retVal);
        }
    }

    /// <returns>
    /// A <c>Operation</c> allocated on Rust side.
    /// </returns>
    public Operation AsOperation()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("MousePosition");
            }
            Raw.Operation* retVal = Raw.MousePosition.AsOperation(_inner);
            return new Operation(retVal);
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.MousePosition* AsFFI()
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

            Raw.MousePosition.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~MousePosition()
    {
        Dispose();
    }
}

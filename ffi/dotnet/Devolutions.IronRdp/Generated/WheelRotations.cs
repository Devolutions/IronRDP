// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class WheelRotations: IDisposable
{
    private unsafe Raw.WheelRotations* _inner;

    /// <summary>
    /// Creates a managed <c>WheelRotations</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe WheelRotations(Raw.WheelRotations* handle)
    {
        _inner = handle;
    }

    /// <returns>
    /// A <c>WheelRotations</c> allocated on Rust side.
    /// </returns>
    public static WheelRotations New(bool isVertical, short rotationUnits)
    {
        unsafe
        {
            Raw.WheelRotations* retVal = Raw.WheelRotations.New(isVertical, rotationUnits);
            return new WheelRotations(retVal);
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
                throw new ObjectDisposedException("WheelRotations");
            }
            Raw.Operation* retVal = Raw.WheelRotations.AsOperation(_inner);
            return new Operation(retVal);
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.WheelRotations* AsFFI()
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

            Raw.WheelRotations.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~WheelRotations()
    {
        Dispose();
    }
}

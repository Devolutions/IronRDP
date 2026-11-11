using System;

namespace Devolutions.IronRdp;

/// <summary>
/// If <c>IronRdpError</c> is an opaque error that borrows from an opaque
/// parameter or the receiver, that source handle is held by <c>Inner</c>'s
/// managed lifetime edge rather than by this exception class.
/// </summary>
public class IronRdpException : Exception
{
    public IronRdpError Inner { get; }

    public IronRdpException(IronRdpError inner) : base(
        inner.ToDisplay()
    )
    {
        Inner = inner;
    }
}
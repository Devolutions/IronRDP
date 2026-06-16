using System;

namespace Devolutions.IronRdp;

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
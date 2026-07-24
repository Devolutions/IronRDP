using System;

namespace Devolutions.IronRdp;

public class IronRdpException : Exception
{
    public IronRdpError Inner { get; }
    private readonly object[] _edges;

    public IronRdpException(IronRdpError inner, params object[] edges) : base(
        inner.ToDisplay()
    )
    {
        Inner = inner;
        _edges = edges;
    }
}
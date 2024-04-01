// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class ConfigBuilder: IDisposable
{
    private unsafe Raw.ConfigBuilder* _inner;

    public bool Autologon
    {
        set
        {
            SetAutologon(value);
        }
    }

    public uint ClientBuild
    {
        set
        {
            SetClientBuild(value);
        }
    }

    public string ClientDir
    {
        set
        {
            SetClientDir(value);
        }
    }

    public string ClientName
    {
        set
        {
            SetClientName(value);
        }
    }

    public string DigProductId
    {
        set
        {
            SetDigProductId(value);
        }
    }

    public string Domain
    {
        set
        {
            SetDomain(value);
        }
    }

    public bool EnableCredssp
    {
        set
        {
            SetEnableCredssp(value);
        }
    }

    public bool EnableTls
    {
        set
        {
            SetEnableTls(value);
        }
    }

    public string ImeFileName
    {
        set
        {
            SetImeFileName(value);
        }
    }

    public uint KeyboardFunctionalKeysCount
    {
        set
        {
            SetKeyboardFunctionalKeysCount(value);
        }
    }

    public uint KeyboardSubtype
    {
        set
        {
            SetKeyboardSubtype(value);
        }
    }

    public KeyboardType KeyboardType
    {
        set
        {
            SetKeyboardType(value);
        }
    }

    public bool NoServerPointer
    {
        set
        {
            SetNoServerPointer(value);
        }
    }

    public PerformanceFlags PerformanceFlags
    {
        set
        {
            SetPerformanceFlags(value);
        }
    }

    public bool PointerSoftwareRendering
    {
        set
        {
            SetPointerSoftwareRendering(value);
        }
    }

    /// <summary>
    /// Creates a managed <c>ConfigBuilder</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe ConfigBuilder(Raw.ConfigBuilder* handle)
    {
        _inner = handle;
    }

    /// <returns>
    /// A <c>ConfigBuilder</c> allocated on Rust side.
    /// </returns>
    public static ConfigBuilder New()
    {
        unsafe
        {
            Raw.ConfigBuilder* retVal = Raw.ConfigBuilder.New();
            return new ConfigBuilder(retVal);
        }
    }

    public void WithUsernameAndPasswrord(string username, string password)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] usernameBuf = DiplomatUtils.StringToUtf8(username);
            byte[] passwordBuf = DiplomatUtils.StringToUtf8(password);
            nuint usernameBufLength = (nuint)usernameBuf.Length;
            nuint passwordBufLength = (nuint)passwordBuf.Length;
            fixed (byte* usernameBufPtr = usernameBuf)
            {
                fixed (byte* passwordBufPtr = passwordBuf)
                {
                    Raw.ConfigBuilder.WithUsernameAndPasswrord(_inner, usernameBufPtr, usernameBufLength, passwordBufPtr, passwordBufLength);
                }
            }
        }
    }

    public void SetDomain(string domain)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] domainBuf = DiplomatUtils.StringToUtf8(domain);
            nuint domainBufLength = (nuint)domainBuf.Length;
            fixed (byte* domainBufPtr = domainBuf)
            {
                Raw.ConfigBuilder.SetDomain(_inner, domainBufPtr, domainBufLength);
            }
        }
    }

    public void SetEnableTls(bool enableTls)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetEnableTls(_inner, enableTls);
        }
    }

    public void SetEnableCredssp(bool enableCredssp)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetEnableCredssp(_inner, enableCredssp);
        }
    }

    public void SetKeyboardType(KeyboardType keyboardType)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.KeyboardType keyboardTypeRaw;
            keyboardTypeRaw = (Raw.KeyboardType)keyboardType;
            Raw.ConfigBuilder.SetKeyboardType(_inner, keyboardTypeRaw);
        }
    }

    public void SetKeyboardSubtype(uint keyboardSubtype)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetKeyboardSubtype(_inner, keyboardSubtype);
        }
    }

    public void SetKeyboardFunctionalKeysCount(uint keyboardFunctionalKeysCount)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetKeyboardFunctionalKeysCount(_inner, keyboardFunctionalKeysCount);
        }
    }

    public void SetImeFileName(string imeFileName)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] imeFileNameBuf = DiplomatUtils.StringToUtf8(imeFileName);
            nuint imeFileNameBufLength = (nuint)imeFileNameBuf.Length;
            fixed (byte* imeFileNameBufPtr = imeFileNameBuf)
            {
                Raw.ConfigBuilder.SetImeFileName(_inner, imeFileNameBufPtr, imeFileNameBufLength);
            }
        }
    }

    public void SetDigProductId(string digProductId)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] digProductIdBuf = DiplomatUtils.StringToUtf8(digProductId);
            nuint digProductIdBufLength = (nuint)digProductIdBuf.Length;
            fixed (byte* digProductIdBufPtr = digProductIdBuf)
            {
                Raw.ConfigBuilder.SetDigProductId(_inner, digProductIdBufPtr, digProductIdBufLength);
            }
        }
    }

    public void SetDesktopSize(ushort height, ushort width)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetDesktopSize(_inner, height, width);
        }
    }

    public void SetPerformanceFlags(PerformanceFlags performanceFlags)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.PerformanceFlags* performanceFlagsRaw;
            performanceFlagsRaw = performanceFlags.AsFFI();
            if (performanceFlagsRaw == null)
            {
                throw new ObjectDisposedException("PerformanceFlags");
            }
            Raw.ConfigBuilder.SetPerformanceFlags(_inner, performanceFlagsRaw);
        }
    }

    public void SetClientBuild(uint clientBuild)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetClientBuild(_inner, clientBuild);
        }
    }

    public void SetClientName(string clientName)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] clientNameBuf = DiplomatUtils.StringToUtf8(clientName);
            nuint clientNameBufLength = (nuint)clientNameBuf.Length;
            fixed (byte* clientNameBufPtr = clientNameBuf)
            {
                Raw.ConfigBuilder.SetClientName(_inner, clientNameBufPtr, clientNameBufLength);
            }
        }
    }

    public void SetClientDir(string clientDir)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            byte[] clientDirBuf = DiplomatUtils.StringToUtf8(clientDir);
            nuint clientDirBufLength = (nuint)clientDirBuf.Length;
            fixed (byte* clientDirBufPtr = clientDirBuf)
            {
                Raw.ConfigBuilder.SetClientDir(_inner, clientDirBufPtr, clientDirBufLength);
            }
        }
    }

    public void SetNoServerPointer(bool noServerPointer)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetNoServerPointer(_inner, noServerPointer);
        }
    }

    public void SetAutologon(bool autologon)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetAutologon(_inner, autologon);
        }
    }

    public void SetPointerSoftwareRendering(bool pointerSoftwareRendering)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConfigBuilder.SetPointerSoftwareRendering(_inner, pointerSoftwareRendering);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>Config</c> allocated on Rust side.
    /// </returns>
    public Config Build()
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ConfigBuilder");
            }
            Raw.ConnectorConfigFfiResultBoxConfigBoxIronRdpError result = Raw.ConfigBuilder.Build(_inner);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.Config* retVal = result.Ok;
            return new Config(retVal);
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.ConfigBuilder* AsFFI()
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

            Raw.ConfigBuilder.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~ConfigBuilder()
    {
        Dispose();
    }
}

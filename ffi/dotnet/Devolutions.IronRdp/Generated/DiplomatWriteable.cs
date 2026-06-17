using System;
using System.Runtime.InteropServices;

#if __IOS__
using ObjCRuntime;
#endif

namespace Devolutions.IronRdp.Diplomat;

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void WriteableFlush(IntPtr self);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
internal delegate bool WriteableGrow(IntPtr self, nuint capacity);

/// <summary>
/// Caller-provided writer that Rust appends UTF-8 bytes into.
/// Use <see cref="ToUnicode"/> after a Rust call returns to extract the
/// written content as a managed string. Dispose to free the underlying
/// unmanaged buffer.
/// </summary>
// Mirrors the Rust `DiplomatWrite` repr(C) layout. The `byte growFailed`
// field corresponds to Rust's `grow_failed: bool`; Sequential layout pads
// it to the next 8-byte boundary so `flush`/`grow` land at offsets 40/48.
[StructLayout(LayoutKind.Sequential)]
internal struct DiplomatWriteable : IDisposable
{
    IntPtr context;
    IntPtr buf;
    nuint len;
    nuint cap;
    byte growFailed;
    readonly IntPtr flush;
    readonly IntPtr grow;

    public DiplomatWriteable()
    {
        WriteableFlush flushFunc = Flush;
        WriteableGrow growFunc = Grow;

        IntPtr flushFuncPtr = Marshal.GetFunctionPointerForDelegate(flushFunc);
        IntPtr growFuncPtr = Marshal.GetFunctionPointerForDelegate(growFunc);

        // Hold the delegate objects alive for as long as the function-pointer
        // form is in use. Without this, the GC could free the thunks and
        // Rust's callback into them would crash.
        DiplomatWriteableContext ctx = new DiplomatWriteableContext
        {
            flushFunc = flushFunc,
            growFunc = growFunc,
        };
        GCHandle ctxHandle = GCHandle.Alloc(ctx);

        context = GCHandle.ToIntPtr(ctxHandle);
        buf = Marshal.AllocHGlobal(64);
        len = 0;
        cap = 64;
        growFailed = 0;
        flush = flushFuncPtr;
        grow = growFuncPtr;
    }

    public byte[] ToUtf8Bytes()
    {
        if (len > int.MaxValue)
        {
            throw new IndexOutOfRangeException("DiplomatWriteable buffer is too big");
        }
        byte[] managedArray = new byte[(int)len];
        Marshal.Copy(buf, managedArray, 0, (int)len);
        return managedArray;
    }

    public string ToUnicode()
    {
        if (len > int.MaxValue)
        {
            throw new IndexOutOfRangeException("DiplomatWriteable buffer is too big");
        }
#if NET6_0_OR_GREATER
        return Marshal.PtrToStringUTF8(buf, (int)len) ?? string.Empty;
#else
        // `Marshal.PtrToStringUTF8` doesn't exist on netstandard2.0 / .NET
        // Framework. Fall back to copying the bytes out and decoding via
        // `Encoding.UTF8`, both of which are available there.
        return System.Text.Encoding.UTF8.GetString(ToUtf8Bytes());
#endif
    }

    public void Dispose()
    {
        if (buf != IntPtr.Zero)
        {
            Marshal.FreeHGlobal(buf);
            buf = IntPtr.Zero;
        }

        if (context != IntPtr.Zero)
        {
            GCHandle.FromIntPtr(context).Free();
            context = IntPtr.Zero;
        }
    }

#if __IOS__
    [MonoPInvokeCallback(typeof(WriteableFlush))]
#endif
    private static void Flush(IntPtr self)
    {
        // Nothing to do — Rust signals "I'm done writing" but we don't
        // need to act on it. The caller will read via ToUnicode() when
        // it's ready.
    }

#if __IOS__
    [MonoPInvokeCallback(typeof(WriteableGrow))]
#endif
    [return: MarshalAs(UnmanagedType.U1)]
    private unsafe static bool Grow(IntPtr writeable, nuint capacity)
    {
        if (writeable == IntPtr.Zero)
        {
            return false;
        }
        DiplomatWriteable* self = (DiplomatWriteable*)writeable;

        nuint newCap = capacity;
        if (newCap > int.MaxValue)
        {
            return false;
        }

        IntPtr newBuf;
        try
        {
            newBuf = Marshal.AllocHGlobal((int)newCap);
        }
        catch (OutOfMemoryException)
        {
            return false;
        }

        Buffer.MemoryCopy((void*)self->buf, (void*)newBuf, newCap, self->len);
        Marshal.FreeHGlobal(self->buf);
        self->buf = newBuf;
        self->cap = newCap;

        return true;
    }
}

internal struct DiplomatWriteableContext
{
    internal WriteableFlush flushFunc;
    internal WriteableGrow growFunc;
}
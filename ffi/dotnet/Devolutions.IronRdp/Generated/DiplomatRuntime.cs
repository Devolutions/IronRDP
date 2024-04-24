// Automatically generated by Diplomat

using System;
using System.Collections;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace Devolutions.IronRdp.Diplomat;

#nullable enable

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
delegate void WriteableFlush(IntPtr self);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
[return: MarshalAs(UnmanagedType.U1)]
delegate bool WriteableGrow(IntPtr self, nuint capacity);

[Serializable]
[StructLayout(LayoutKind.Sequential)]
public struct DiplomatWriteable : IDisposable
{
    // About the current approach:
    // Ideally DiplomatWriteable should wrap the native string type and grows/writes directly into the internal buffer.
    // However, there is no native string type backed by UTF-8 in dotnet frameworks.
    // Alternative could be to provide Diplomat's own `Utf8String` type, but this is not trivial and mostly useless
    // on its own because all other dotnet/C# APIs are expecting the standard UTF-16 encoded string/String type.
    // API is expected to become overall less ergonomic by forcing users to explicitly convert between UTF-16 and UTF-8 anyway in such case.
    // It could be useful for copy-free interactions between Diplomat-generated API though.
    // Also, there is no way to re-use the unmanaged buffer as-is to get a managed byte[]
    // (hence `ToUtf8Bytes` is copying the internal buffer from unmanaged to managed memory).
    // It's encouraged to enable memoization when applicable (#129).

    IntPtr context;
    IntPtr buf;
    nuint len;
    nuint cap;
    readonly IntPtr flush;
    readonly IntPtr grow;

    public DiplomatWriteable()
    {
        WriteableFlush flushFunc = Flush;
        WriteableGrow growFunc = Grow;

        IntPtr flushFuncPtr = Marshal.GetFunctionPointerForDelegate(flushFunc);
        IntPtr growFuncPtr = Marshal.GetFunctionPointerForDelegate(growFunc);
        
        // flushFunc and growFunc are managed objects and might be disposed of by the garbage collector.
        // To prevent this, we make the context hold the references and protect the context itself
        // for automatic disposal by moving it behind a GCHandle.
        DiplomatWriteableContext ctx = new DiplomatWriteableContext();        
        ctx.flushFunc = flushFunc;
        ctx.growFunc = growFunc;
        GCHandle ctxHandle = GCHandle.Alloc(ctx);

        context = GCHandle.ToIntPtr(ctxHandle);
        buf = Marshal.AllocHGlobal(64);
        len = 0;
        cap = 64;
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
#if NET6_0_OR_GREATER
        if (len > int.MaxValue)
        {
            throw new IndexOutOfRangeException("DiplomatWriteable buffer is too big");
        }
        return Marshal.PtrToStringUTF8(buf, (int) len);
#else
        byte[] utf8 = ToUtf8Bytes();
        return DiplomatUtils.Utf8ToString(utf8);
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

    private static void Flush(IntPtr self)
    {
        // Nothing to do
    }

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

        Buffer.MemoryCopy((void*)self->buf, (void*)newBuf, newCap, self->cap);
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

internal static class DiplomatUtils
{
    internal static byte[] StringToUtf8(string s)
    {
        int size = Encoding.UTF8.GetByteCount(s);
        byte[] buf = new byte[size];
        Encoding.UTF8.GetBytes(s, 0, s.Length, buf, 0);
        return buf;
    }

    internal static string Utf8ToString(byte[] utf8)
    {
        char[] chars = new char[utf8.Length];
        Encoding.UTF8.GetChars(utf8, 0, utf8.Length, chars, 0);
        return new string(chars);
    }
}

public class DiplomatOpaqueException : Exception
{
    public DiplomatOpaqueException() : base("The FFI function failed with an opaque error") { }
}

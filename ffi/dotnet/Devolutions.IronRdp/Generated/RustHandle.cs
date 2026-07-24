using System;

namespace Devolutions.IronRdp.Diplomat;

#nullable enable

/// <summary>
/// Frees a Rust-owned <typeparamref name="T"/> by calling its native
/// destructor. Held by an owned <see cref="RustHandle{T}"/> so the handle can
/// release the pointer without knowing the concrete type.
/// </summary>
internal unsafe delegate void RustDestructor<T>(T* ptr) where T : unmanaged;

/// <summary>
/// A raw pointer that remembers who's responsible for freeing it. An owned
/// handle carries the Rust destructor and runs it when you release the handle;
/// a borrowed handle carries none, so releasing it does nothing — Rust still
/// owns the memory and will free it itself. That's the whole trick: the
/// pointer itself says "free me" or "leave me alone", so the wrapper doesn't
/// need a separate flag bolted onto the class.
/// </summary>
internal readonly unsafe struct RustHandle<T> where T : unmanaged
{
    private readonly T* _ptr;
    private readonly RustDestructor<T>? _free;

    private RustHandle(T* ptr, RustDestructor<T>? free)
    {
        _ptr = ptr;
        _free = free;
    }

    /// <summary>The C# side owns the pointer: <see cref="Release"/> frees it.</summary>
    public static RustHandle<T> Owned(T* ptr, RustDestructor<T> free) => new RustHandle<T>(ptr, free);

    /// <summary>Rust still owns the pointer: <see cref="Release"/> is a no-op.</summary>
    public static RustHandle<T> Borrowed(T* ptr) => new RustHandle<T>(ptr, null);

    public T* Ptr => _ptr;

    public bool IsNull => _ptr == null;

    public void Release()
    {
        if (_free != null && _ptr != null)
        {
            _free(_ptr);
        }
    }
}
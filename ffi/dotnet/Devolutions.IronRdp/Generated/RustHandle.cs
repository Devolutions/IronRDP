using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

namespace Devolutions.IronRdp.Diplomat;

#nullable enable

internal unsafe delegate void RustDestructor<T>(T* ptr) where T : unmanaged;

internal interface ILifetimeEdge
{
    void Release();
}

internal static class LifetimeEdge
{
    internal static ILifetimeEdge? Move<T>(ref BorrowLease<T>? lease) where T : unmanaged
    {
        BorrowLease<T>? moved = Interlocked.Exchange(ref lease, null);
        moved?.TransferToPersistent();
        return moved;
    }
}

internal enum BorrowKind
{
    Shared,
    Exclusive,
}

internal enum WrapperKind
{
    Owned,
    SharedView,
    ExclusiveView,
}

internal readonly struct MutationVersion
{
    private readonly long _value;

    internal MutationVersion(long value)
    {
        _value = value;
    }

    internal bool Equals(MutationVersion other) => _value == other._value;
}

internal struct LifetimeEdges
{
    private ILifetimeEdge?[] _edges;

    internal LifetimeEdges(WrapperKind kind, ILifetimeEdge?[] edges)
    {
        _edges = edges;
        for (int i = 0; i < _edges.Length; i++)
        {
            if (_edges[i] is IBorrowLease lease && IsReadView(kind, lease))
            {
                _edges[i] = lease.IntoVersionedEdge();
            }
        }
    }

    // A value born from an exclusive borrow is its source's only writer until released, so
    // it keeps the borrow; every other dependent is a read view that only remembers a version.
    private static bool IsReadView(WrapperKind kind, IBorrowLease lease) =>
        kind == WrapperKind.SharedView
        || (kind == WrapperKind.Owned && lease.Kind == BorrowKind.Shared);

    internal ILifetimeEdge[] HoldDependenciesForCall()
    {
        List<ILifetimeEdge>? held = null;
        try
        {
            foreach (ILifetimeEdge? edge in Volatile.Read(ref _edges))
            {
                ILifetimeEdge? operation = edge switch
                {
                    IVersionedReference versioned => versioned.LeaseForOperation(),
                    IBorrowLease borrow => borrow.HoldForCall(),
                    _ => null,
                };
                if (operation is not null)
                {
                    (held ??= new List<ILifetimeEdge>()).Add(operation);
                }
            }

            return held?.ToArray() ?? Array.Empty<ILifetimeEdge>();
        }
        catch
        {
            if (held is not null)
            {
                ReleaseNoThrow(held.ToArray());
            }
            throw;
        }
    }

    internal void Release()
    {
        ILifetimeEdge?[] edges = Interlocked.Exchange(
            ref _edges,
            Array.Empty<ILifetimeEdge?>());
        ReleaseNoThrow(edges);
    }

    internal static void ReleaseLeases(ILifetimeEdge[] leases)
    {
        for (int i = leases.Length - 1; i >= 0; i--)
        {
            leases[i].Release();
        }
    }

    internal static void ReleaseNoThrow(ILifetimeEdge?[] edges)
    {
        for (int i = edges.Length - 1; i >= 0; i--)
        {
            try
            {
                edges[i]?.Release();
            }
            catch
            {
            }
        }
    }
}

internal struct BorrowLedger
{
    private const int Free = 0;
    private const int Exclusive = -1;

    private int _state;
    private MutationClock _mutations;
    private int _scopeEnded;

    internal bool IsScopeOpen() => Volatile.Read(ref _scopeEnded) == 0;

    internal bool IsVersionAccessible(MutationVersion mutationVersion) =>
        IsScopeOpen()
        && Volatile.Read(ref _state) != Exclusive
        && _mutations.Read().Equals(mutationVersion);

    internal bool EndScope() => Interlocked.Exchange(ref _scopeEnded, 1) == 0;

    internal void Enter(BorrowKind kind)
    {
        if (kind == BorrowKind.Exclusive)
        {
            if (Interlocked.CompareExchange(ref _state, Exclusive, Free) != Free)
            {
                throw new InvalidOperationException("Another borrow is already active.");
            }

            return;
        }

        while (true)
        {
            int current = Volatile.Read(ref _state);
            if (current < Free)
            {
                throw new InvalidOperationException("An exclusive borrow is already active.");
            }

            if (Interlocked.CompareExchange(ref _state, current + 1, current) == current)
            {
                return;
            }
        }
    }

    internal MutationVersion Exit(BorrowKind kind)
    {
        if (kind == BorrowKind.Shared)
        {
            MutationVersion version = _mutations.Read();
            Interlocked.Decrement(ref _state);
            return version;
        }

        MutationVersion nextVersion = _mutations.Advance();
        Volatile.Write(ref _state, Free);
        return nextVersion;
    }

    private struct MutationClock
    {
        private long _version;

        internal MutationVersion Read() => new MutationVersion(Volatile.Read(ref _version));

        internal MutationVersion Advance() =>
            new MutationVersion(Interlocked.Increment(ref _version));
    }
}

internal sealed unsafe class DependencyOperationLease<T> : ILifetimeEdge where T : unmanaged
{
    private RustHandle<T>? _owner;
    private ILifetimeEdge[] _dependencies;

    internal DependencyOperationLease(RustHandle<T> owner, ILifetimeEdge[] dependencies)
    {
        _owner = owner;
        _dependencies = dependencies;
    }

    public void Release()
    {
        RustHandle<T>? owner = Interlocked.Exchange(ref _owner, null);
        if (owner is null)
        {
            return;
        }

        ILifetimeEdge[] dependencies = Interlocked.Exchange(
            ref _dependencies,
            Array.Empty<ILifetimeEdge>());
        try
        {
            LifetimeEdges.ReleaseLeases(dependencies);
        }
        finally
        {
            GC.KeepAlive(owner);
        }
    }
}

internal sealed unsafe class OperationLease<T> : ILifetimeEdge where T : unmanaged
{
    private RustHandle<T>? _owner;
    private ILifetimeEdge[] _dependencies;

    internal OperationLease(RustHandle<T> owner, ILifetimeEdge[] dependencies)
    {
        _owner = owner;
        _dependencies = dependencies;
    }

    public void Release()
    {
        RustHandle<T>? owner = Interlocked.Exchange(ref _owner, null);
        if (owner is null)
        {
            return;
        }

        ILifetimeEdge[] dependencies = Interlocked.Exchange(
            ref _dependencies,
            Array.Empty<ILifetimeEdge>());
        try
        {
            LifetimeEdges.ReleaseLeases(dependencies);
        }
        finally
        {
            owner.ExitOperation();
        }
    }
}

// The SafeHandle count protects only a native call made directly on this value. Source edges
// keep managed handles reachable and validate borrow state without taking SafeHandle claims.
internal sealed unsafe class RustHandle<T> : SafeHandle where T : unmanaged
{
    private readonly RustDestructor<T>? _destructor;
    private readonly WrapperKind _wrapperKind;
    private LifetimeEdges _edges;
    private BorrowLedger _borrows;

    private RustHandle(
        T* ptr,
        RustDestructor<T>? destructor,
        WrapperKind kind,
        ILifetimeEdge?[] edges)
        : base(IntPtr.Zero, ownsHandle: true)
    {
        _destructor = destructor;
        _wrapperKind = kind;
        _edges = new LifetimeEdges(kind, edges);
        SetHandle((IntPtr)ptr);
    }

    internal static RustHandle<T> Owned(T* ptr, RustDestructor<T> destructor) =>
        Create(ptr, destructor, WrapperKind.Owned, Array.Empty<ILifetimeEdge?>());

    internal static RustHandle<T> Owned(
        T* ptr,
        RustDestructor<T> destructor,
        params ILifetimeEdge?[] edges) =>
        Create(ptr, destructor, WrapperKind.Owned, edges);

    internal static RustHandle<T> Borrowed(
        T* ptr,
        WrapperKind kind,
        params ILifetimeEdge?[] edges) =>
        Create(ptr, null, kind, edges);

    private static RustHandle<T> Create(
        T* ptr,
        RustDestructor<T>? destructor,
        WrapperKind kind,
        ILifetimeEdge?[] edges)
    {
        try
        {
            return new RustHandle<T>(ptr, destructor, kind, edges);
        }
        catch
        {
            try
            {
                if (ptr != null && destructor is not null)
                {
                    destructor(ptr);
                }
            }
            finally
            {
                LifetimeEdges.ReleaseNoThrow(edges);
            }

            throw;
        }
    }

    public override bool IsInvalid => handle == IntPtr.Zero;

    internal T* Ptr => (T*)handle;

    internal bool IsVersionAccessible(MutationVersion mutationVersion) =>
        !IsClosed && _borrows.IsVersionAccessible(mutationVersion);

    internal BorrowLease<T> Lease(BorrowKind kind)
    {
        if (IsClosed)
        {
            throw new ObjectDisposedException(typeof(T).Name);
        }

        if (kind == BorrowKind.Exclusive && _wrapperKind == WrapperKind.SharedView)
        {
            throw new InvalidOperationException("This wrapper only carries a shared borrow.");
        }

        _borrows.Enter(kind);
        try
        {
            OperationLease<T> operation = StartCall();
            return new BorrowLease<T>(this, kind, Ptr, operation);
        }
        catch
        {
            _borrows.Exit(kind);
            throw;
        }
    }

    internal BorrowLease<T> LeaseVersionForOperation(MutationVersion mutationVersion)
    {
        if (!IsVersionAccessible(mutationVersion))
        {
            throw new InvalidOperationException(
                "This borrowed view was invalidated by disposal or mutation of its source.");
        }

        _borrows.Enter(BorrowKind.Shared);
        ILifetimeEdge? operation = null;
        try
        {
            operation = HoldForCall();
            if (_borrows.IsVersionAccessible(mutationVersion) && !IsClosed)
            {
                return new BorrowLease<T>(
                    this,
                    BorrowKind.Shared,
                    Ptr,
                    operation);
            }

            throw new InvalidOperationException(
                "This borrowed view was invalidated by disposal or mutation of its source.");
        }
        catch
        {
            operation?.Release();
            _borrows.Exit(BorrowKind.Shared);
            throw;
        }
    }

    internal OperationLease<T> StartCall()
    {
        bool success = false;
        DangerousAddRef(ref success);
        try
        {
            return new OperationLease<T>(this, _edges.HoldDependenciesForCall());
        }
        catch
        {
            DangerousRelease();
            throw;
        }
    }

    internal DependencyOperationLease<T> HoldForCall()
    {
        if (IsClosed)
        {
            throw new InvalidOperationException(
                "The source of this borrowed value was disposed.");
        }

        ILifetimeEdge[] dependencies = _edges.HoldDependenciesForCall();
        if (!IsClosed)
        {
            return new DependencyOperationLease<T>(this, dependencies);
        }

        LifetimeEdges.ReleaseNoThrow(dependencies);
        throw new InvalidOperationException(
            "The source of this borrowed value was disposed.");
    }

    internal void ExitBorrow(BorrowKind kind) => _borrows.Exit(kind);

    internal MutationVersion ExitBorrowKeepingReference(BorrowKind kind) => _borrows.Exit(kind);

    internal void ExitOperation() => DangerousRelease();

    internal void ReleaseWrapper()
    {
        if (_wrapperKind == WrapperKind.ExclusiveView)
        {
            _borrows.EndScope();
        }

        Dispose();
    }

    protected override bool ReleaseHandle()
    {
        try
        {
            if (_destructor is not null)
            {
                _destructor((T*)handle);
            }
        }
        finally
        {
            _edges.Release();
        }

        return true;
    }
}
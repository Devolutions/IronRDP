using System;
using System.Threading;

namespace Devolutions.IronRdp.Diplomat;

#nullable enable

internal interface IBorrowLease : ILifetimeEdge
{
    BorrowKind Kind { get; }

    ILifetimeEdge HoldForCall();

    ILifetimeEdge IntoVersionedEdge();
}

internal interface IVersionedReference : ILifetimeEdge
{
    IBorrowLease LeaseForOperation();
}

internal sealed unsafe class BorrowLease<T> : IBorrowLease where T : unmanaged
{
    private RustHandle<T>? _owner;
    private readonly BorrowKind _kind;
    private ILifetimeEdge? _operation;

    internal BorrowLease(
        RustHandle<T> owner,
        BorrowKind kind,
        T* ptr,
        ILifetimeEdge operation)
    {
        _owner = owner;
        _kind = kind;
        Ptr = ptr;
        _operation = operation;
    }

    internal T* Ptr { get; }

    public BorrowKind Kind => _kind;

    public ILifetimeEdge HoldForCall()
    {
        RustHandle<T>? owner = Volatile.Read(ref _owner);
        if (owner is null)
        {
            throw new InvalidOperationException(
                "The source of this borrowed value is no longer available.");
        }

        return owner.HoldForCall();
    }

    public ILifetimeEdge IntoVersionedEdge()
    {
        RustHandle<T> owner = TakeOwner();
        MutationVersion version = owner.ExitBorrowKeepingReference(_kind);
        TakeOperation()?.Release();
        return new VersionedReference(owner, version);
    }

    internal void TransferToPersistent()
    {
        TakeOperation()?.Release();
    }

    public void Release()
    {
        RustHandle<T>? owner = Interlocked.Exchange(ref _owner, null);
        if (owner is null)
        {
            return;
        }

        try
        {
            TakeOperation()?.Release();
        }
        finally
        {
            owner.ExitBorrow(_kind);
        }
    }

    private RustHandle<T> TakeOwner()
    {
        RustHandle<T>? owner = Interlocked.Exchange(ref _owner, null);
        if (owner is null)
        {
            throw new ObjectDisposedException(nameof(BorrowLease<T>));
        }

        return owner;
    }

    private ILifetimeEdge? TakeOperation() => Interlocked.Exchange(ref _operation, null);

    private sealed class VersionedReference : IVersionedReference
    {
        private RustHandle<T>? _owner;
        private readonly MutationVersion _version;

        internal VersionedReference(RustHandle<T> owner, MutationVersion version)
        {
            _owner = owner;
            _version = version;
        }

        public IBorrowLease LeaseForOperation()
        {
            RustHandle<T>? owner = Volatile.Read(ref _owner);
            if (owner is null)
            {
                throw new InvalidOperationException(
                    "The source of this borrowed value is no longer available.");
            }

            return owner.LeaseVersionForOperation(_version);
        }

        public void Release()
        {
            Interlocked.Exchange(ref _owner, null);
        }
    }
}
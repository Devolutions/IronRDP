// <auto-generated/> by Diplomat

#pragma warning disable 0105
using System;
using System.Runtime.InteropServices;

using Devolutions.IronRdp.Diplomat;
#pragma warning restore 0105

namespace Devolutions.IronRdp;

#nullable enable

public partial class ActiveStage: IDisposable
{
    private unsafe Raw.ActiveStage* _inner;

    /// <summary>
    /// Creates a managed <c>ActiveStage</c> from a raw handle.
    /// </summary>
    /// <remarks>
    /// Safety: you should not build two managed objects using the same raw handle (may causes use-after-free and double-free).
    /// <br/>
    /// This constructor assumes the raw struct is allocated on Rust side.
    /// If implemented, the custom Drop implementation on Rust side WILL run on destruction.
    /// </remarks>
    public unsafe ActiveStage(Raw.ActiveStage* handle)
    {
        _inner = handle;
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>ActiveStage</c> allocated on Rust side.
    /// </returns>
    public static ActiveStage New(ConnectionResult connectionResult)
    {
        unsafe
        {
            Raw.ConnectionResult* connectionResultRaw;
            connectionResultRaw = connectionResult.AsFFI();
            if (connectionResultRaw == null)
            {
                throw new ObjectDisposedException("ConnectionResult");
            }
            Raw.SessionFfiResultBoxActiveStageBoxIronRdpError result = Raw.ActiveStage.New(connectionResultRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.ActiveStage* retVal = result.Ok;
            return new ActiveStage(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>ActiveStageOutputIterator</c> allocated on Rust side.
    /// </returns>
    public ActiveStageOutputIterator Process(DecodedImage image, Action action, byte[] payload)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ActiveStage");
            }
            nuint payloadLength = (nuint)payload.Length;
            Raw.DecodedImage* imageRaw;
            imageRaw = image.AsFFI();
            if (imageRaw == null)
            {
                throw new ObjectDisposedException("DecodedImage");
            }
            Raw.Action* actionRaw;
            actionRaw = action.AsFFI();
            if (actionRaw == null)
            {
                throw new ObjectDisposedException("Action");
            }
            fixed (byte* payloadPtr = payload)
            {
                Raw.SessionFfiResultBoxActiveStageOutputIteratorBoxIronRdpError result = Raw.ActiveStage.Process(_inner, imageRaw, actionRaw, payloadPtr, payloadLength);
                if (!result.isOk)
                {
                    throw new IronRdpException(new IronRdpError(result.Err));
                }
                Raw.ActiveStageOutputIterator* retVal = result.Ok;
                return new ActiveStageOutputIterator(retVal);
            }
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>ActiveStageOutputIterator</c> allocated on Rust side.
    /// </returns>
    public ActiveStageOutputIterator ProcessFastpathInput(DecodedImage image, FastPathInputEventIterator fastpathInput)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ActiveStage");
            }
            Raw.DecodedImage* imageRaw;
            imageRaw = image.AsFFI();
            if (imageRaw == null)
            {
                throw new ObjectDisposedException("DecodedImage");
            }
            Raw.FastPathInputEventIterator* fastpathInputRaw;
            fastpathInputRaw = fastpathInput.AsFFI();
            if (fastpathInputRaw == null)
            {
                throw new ObjectDisposedException("FastPathInputEventIterator");
            }
            Raw.SessionFfiResultBoxActiveStageOutputIteratorBoxIronRdpError result = Raw.ActiveStage.ProcessFastpathInput(_inner, imageRaw, fastpathInputRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.ActiveStageOutputIterator* retVal = result.Ok;
            return new ActiveStageOutputIterator(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>VecU8</c> allocated on Rust side.
    /// </returns>
    public VecU8 InitiateClipboardCopy(ClipboardFormatIterator formats)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ActiveStage");
            }
            Raw.ClipboardFormatIterator* formatsRaw;
            formatsRaw = formats.AsFFI();
            if (formatsRaw == null)
            {
                throw new ObjectDisposedException("ClipboardFormatIterator");
            }
            Raw.SessionFfiResultBoxVecU8BoxIronRdpError result = Raw.ActiveStage.InitiateClipboardCopy(_inner, formatsRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.VecU8* retVal = result.Ok;
            return new VecU8(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>VecU8</c> allocated on Rust side.
    /// </returns>
    public VecU8 InitiateClipboardPaste(ClipboardFormatId formatId)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ActiveStage");
            }
            Raw.ClipboardFormatId* formatIdRaw;
            formatIdRaw = formatId.AsFFI();
            if (formatIdRaw == null)
            {
                throw new ObjectDisposedException("ClipboardFormatId");
            }
            Raw.SessionFfiResultBoxVecU8BoxIronRdpError result = Raw.ActiveStage.InitiateClipboardPaste(_inner, formatIdRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.VecU8* retVal = result.Ok;
            return new VecU8(retVal);
        }
    }

    /// <exception cref="IronRdpException"></exception>
    /// <returns>
    /// A <c>VecU8</c> allocated on Rust side.
    /// </returns>
    public VecU8 SubmitClipboardFormatData(FormatDataResponse formatDataResponse)
    {
        unsafe
        {
            if (_inner == null)
            {
                throw new ObjectDisposedException("ActiveStage");
            }
            Raw.FormatDataResponse* formatDataResponseRaw;
            formatDataResponseRaw = formatDataResponse.AsFFI();
            if (formatDataResponseRaw == null)
            {
                throw new ObjectDisposedException("FormatDataResponse");
            }
            Raw.SessionFfiResultBoxVecU8BoxIronRdpError result = Raw.ActiveStage.SubmitClipboardFormatData(_inner, formatDataResponseRaw);
            if (!result.isOk)
            {
                throw new IronRdpException(new IronRdpError(result.Err));
            }
            Raw.VecU8* retVal = result.Ok;
            return new VecU8(retVal);
        }
    }

    /// <summary>
    /// Returns the underlying raw handle.
    /// </summary>
    public unsafe Raw.ActiveStage* AsFFI()
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

            Raw.ActiveStage.Destroy(_inner);
            _inner = null;

            GC.SuppressFinalize(this);
        }
    }

    ~ActiveStage()
    {
        Dispose();
    }
}

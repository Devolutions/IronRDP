using NowProto.Types;
using System.Threading;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_SESSION_MSGBOX_REQ_MSG is used to show a message box in the user session, similar to
    /// what the [WTSSendMessage function](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtssendmessagew)
    /// does.
    ///
    /// NOW_PROTO: NOW_SESSION_MSGBOX_REQ_MSG
    /// </summary>
    public class NowMsgSessionMessageBoxReq(uint requestId, string message) : INowSerialize
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassSession;
        public static byte TypeMessageKind => 0x03; // NOW-PROTO: NOW_SESSION_MSGBOX_REQ_MSG_ID

        public byte MessageClass => NowMessage.ClassSession;
        public byte MessageKind => 0x03;

        // -- INowSerialize --

        public ushort Flags { get; private set; }
        public uint BodySize => FixedPartSize + NowVarStr.LengthOf(_title) + NowVarStr.LengthOf(Message);

        public void SerializeBody(NowWriteCursor cursor)
        {
            cursor.EnsureEnoughBytes(FixedPartSize);
            cursor.WriteUint32Le(RequestId);
            cursor.WriteUint32Le(_style);
            cursor.WriteUint32Le(_timeout);
            cursor.WriteVarStr(_title);
            cursor.WriteVarStr(Message);
        }

        // -- impl --

        private const uint FixedPartSize = 12; // u32 requestId + u32 style + u32 timeout

        // The title field contains non-default value.
        //
        // NOW_PROTO: NOW_SESSION_MSGBOX_FLAG_TITLE
        private const ushort TitleFlag = 0x0001;

        // The style field contains non-default value.
        //
        // NOW_PROTO: NOW_SESSION_MSGBOX_FLAG_STYLE
        private const ushort StyleFlag = 0x0002;

        // The timeout field contains non-default value.
        //
        // NOW_PROTO: NOW_SESSION_MSGBOX_FLAG_TIMEOUT
        private const ushort TimeoutFlag = 0x0004;

        // A response message is expected (don't fire and forget)
        //
        // NOW_PROTO: NOW_SESSION_MSGBOX_FLAG_RESPONSE
        private const ushort ResponseFlag = 0x0008;


        public enum MessageBoxStyle : uint
        {
            Ok = 0x00000000,
            OkCancel = 0x00000001,
            AbortRetryIgnore = 0x00000002,
            YesNoCancel = 0x00000003,
            YesNo = 0x00000004,
            RetryCancel = 0x00000005,
            CancelTryContinue = 0x00000006,
            Help = 0x00004000,
        }


        public uint RequestId { get; set; } = requestId;

        public MessageBoxStyle? Style
        {
            get
            {
                if ((Flags & StyleFlag) == 0)
                {
                    return null;
                }

                return (MessageBoxStyle)_style;
            }
            set
            {
                if (value == null)
                {
                    Flags &= unchecked((ushort)~StyleFlag);
                    _style = 0;
                    return;
                }

                Flags |= StyleFlag;
                _style = (uint)value;
            }
        }
        public uint? Timeout
        {
            get
            {
                if ((Flags & TimeoutFlag) == 0)
                {
                    return null;
                }

                return _timeout;
            }
            set
            {
                if (value is null or 0)
                {
                    Flags &= unchecked((ushort)~TimeoutFlag);
                    _timeout = 0;
                    return;
                }

                Flags |= TimeoutFlag;
                _timeout = (uint)value;
            }
        }
        public string? Title
        {
            get => (Flags & TitleFlag) == 0 ? null : _title;
            set
            {
                if (string.IsNullOrEmpty(value))
                {
                    Flags &= unchecked((ushort)~TitleFlag);
                    _title = "";
                    return;
                }

                Flags |= TitleFlag;
                _title = value;
            }
        }
        public string Message { get; set; } = message;

        public bool WaitForResponse
        {
            get => (Flags & ResponseFlag) != 0;
            set
            {
                if (value)
                {
                    Flags |= ResponseFlag;
                }
                else
                {
                    Flags &= unchecked((ushort)~ResponseFlag);
                }
            }
        }


        private uint _style = 0;
        private uint _timeout = 0;
        private string _title = "";
    }
}

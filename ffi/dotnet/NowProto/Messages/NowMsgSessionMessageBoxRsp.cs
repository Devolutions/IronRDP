using System.Threading.Channels;
using System.Threading;

namespace NowProto.Messages
{
    /// <summary>
    /// The NOW_SESSION_MSGBOX_RSP_MSG is a message sent in response to NOW_SESSION_MSGBOX_REQ_MSG if
    /// the NOW_MSGBOX_FLAG_RESPONSE has been set, and contains the result from the message box dialog.
    ///
    /// NOW_PROTO: NOW_SESSION_MSGBOX_RSP_MSG
    /// </summary>
    public class NowMsgSessionMessageBoxRsp(uint requestId, NowMsgSessionMessageBoxRsp.MessageBoxResponse response)
        : INowDeserialize<NowMsgSessionMessageBoxRsp>
    {
        // -- INowMessage --

        public static byte TypeMessageClass => NowMessage.ClassSession;
        public static byte TypeMessageKind => 0x04; // NOW-PROTO: NOW_SESSION_MSGBOX_RSP_MSG_ID

        public byte MessageClass => NowMessage.ClassSession;
        public byte MessageKind => 0x04;

        // -- INowDeserialize --

        public static NowMsgSessionMessageBoxRsp Deserialize(ushort flags, NowReadCursor cursor)
        {
            cursor.EnsureEnoughBytes(FixedPartSize);
            var requestId = cursor.ReadUInt32Le();
            var response = (MessageBoxResponse)cursor.ReadUInt32Le();

            return new NowMsgSessionMessageBoxRsp(requestId, response);
        }

        // -- impl --

        private const uint FixedPartSize = 8; // u32 requestId + u32 response

        public enum MessageBoxResponse: uint
        {
            /// <summary>
            /// OK
            ///
            /// NOW_PROTO: IDOK
            /// </summary>
            Ok = 1,

            /// <summary>
            /// Cancel
            ///
            /// NOW_PROTO: IDCANCEL
            /// </summary>
            Cancel = 2,

            /// <summary>
            /// Abort
            ///
            /// NOW_PROTO: IDABORT
            /// </summary>
            Abort = 3,

            /// <summary>
            /// Retry
            ///
            /// NOW_PROTO: IDRETRY
            /// </summary>
            Retry = 4,

            /// <summary>
            /// Ignore
            ///
            /// NOW_PROTO: IDIGNORE
            /// </summary>
            Ignore = 5,

            /// <summary>
            /// Yes
            ///
            /// NOW_PROTO: IDYES
            /// </summary>
            Yes = 6,

            /// <summary>
            /// No
            ///
            /// NOW_PROTO: IDNO
            /// </summary>
            No = 7,

            /// <summary>
            /// Try Again
            ///
            /// NOW_PROTO: IDTRYAGAIN
            /// </summary>
            TryAgain = 10,

            /// <summary>
            /// Continue
            ///
            /// NOW_PROTO: IDCONTINUE
            /// </summary>
            Continue = 11,

            /// <summary>
            /// Timeout
            ///
            /// NOW_PROTO: IDTIMEOUT
            /// </summary>
            Timeout = 32000
        }

        public uint RequestId { get; set; } = requestId;
        public MessageBoxResponse Response { get; set; } = response;
    }
}

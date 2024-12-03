using System.Text;

namespace NowProto.Types
{
    /// <summary>
    /// String value up to 2^31 bytes long (Length has compact variable length encoding).
    /// Implemented as class extensions for NowReadCursor and NowWriteCursor.
    ///
    /// NOW-PROTO: NOW_VARSTR
    /// </summary>
    public static class NowVarStr
    {
        public static uint LengthOf(string str)
        {
            var encodedStr = Encoding.UTF8.GetBytes(str, 0, str.Length);
            var headerLen = NowVarU32.LengthOf((uint)encodedStr.Length);

            return headerLen + (uint)encodedStr.Length + 1;
        }

        public static string ReadVarStr(this NowReadCursor cursor)
        {
            var length = cursor.ReadVarU32();
            var data = cursor.ReadBytes(length);
            cursor.ReadByte(); // null terminator

            return Encoding.UTF8.GetString(data);
        }

        public static void WriteVarStr(this NowWriteCursor cursor, string str)
        {
            var encodedStr = Encoding.UTF8.GetBytes(str, 0, str.Length);
            cursor.WriteVarU32((uint)encodedStr.Length);
            cursor.WriteBytes(encodedStr);
            cursor.WriteByte(0);
        }
    }
}

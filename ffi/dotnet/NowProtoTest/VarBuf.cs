using NowProto;
using NowProto.Types;

namespace NowProtoTest
{
    [TestClass]
    public class VarBuf
    {
        [DataTestMethod]
        [DataRow(new byte[] { 1, 2, 3, 4, 5 }, new byte[] { 0x05, 1, 2, 3, 4, 5 })]
        [DataRow(new byte[] {}, new byte[] { 0x00 })]
        public void VarBufRoundtrip(byte[] value, byte[] expectedEncoded)
        {
            var actualEncoded = new byte[NowVarBuf.LengthOf(value)];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                cursor.WriteVarBuf(value);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }

            var actualDecoded = new byte[NowVarBuf.LengthOf(value)];
            {
                var cursor = new NowReadCursor(actualEncoded);
                var actualValue = cursor.ReadVarBuf().ToArray();
                CollectionAssert.AreEqual(value, actualValue);
            }
        }
    }
}

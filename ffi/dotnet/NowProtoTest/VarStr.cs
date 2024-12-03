using NowProto;
using NowProto.Types;

namespace NowProtoTest
{
    [TestClass]
    public class VarStr
    {

        [DataTestMethod]
        [DataRow("hello", new byte[] { 0x05, 0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x00 })]
        [DataRow("", new byte[] { 0x00, 0x00 })]
        public void VarStrRoundtrip(string value, byte[] expectedEncoded)
        {
            var actualEncoded = new byte[NowVarStr.LengthOf(value)];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                cursor.WriteVarStr(value);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }

            var actualDecoded = new byte[NowVarStr.LengthOf(value)];
            {
                var cursor = new NowReadCursor(actualEncoded);
                var actualValue = cursor.ReadVarStr();
                Assert.AreEqual(value, actualValue);
            }
        }
    }
}

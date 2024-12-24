using NowProto;
using NowProto.Types;

namespace NowProtoTest
{
    [TestClass]
    public class VarNum
    {
        [DataTestMethod]
        [DataRow((uint)0x00, new byte[] { 0x00 })]
        [DataRow((uint)0x3F, new byte[] { 0x3F })]
        [DataRow((uint)0x40, new byte[] { 0x40, 0x40 })]
        [DataRow((uint)0x14000, new byte[] { 0x81, 0x40, 0x00 })]
        [DataRow((uint)0x3FFFFFFF, new byte[] { 0xFF, 0xFF, 0xFF, 0xFF })]
        public void VarU32Roundtrip(uint value, byte[] expectedEncoded)
        {
            var actualEncoded = new byte[NowVarU32.LengthOf(value)];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                cursor.WriteVarU32(value);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }

            var actualDecoded = new byte[NowVarU32.LengthOf(value)];
            {

                var cursor = new NowReadCursor(actualEncoded);
                var actualValue = cursor.ReadVarU32();
                Assert.AreEqual(value, actualValue);
            }
        }
    }
}
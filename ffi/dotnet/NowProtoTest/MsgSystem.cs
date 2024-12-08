using NowProto;
using NowProto.Messages;

namespace NowProtoTest
{
    [TestClass]
    public class MsgSystem
    {
        [TestMethod]
        public void MsgShutdownRoundtrip()
        {
            var msg = new NowMsgSystemShutdown("hello", 0x12345678) {
                Force = true,
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
            }

            var expectedEncoded = new byte[]
            {
                0x0B, 0x00, 0x00, 0x00, 0x11, 0x03, 0x01, 0x00, 0x78, 0x56,
                0x34, 0x12, 0x05, 0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x00
            };

            CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
        }
    }
}

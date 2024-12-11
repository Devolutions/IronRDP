using NowProto;
using NowProto.Messages;

namespace NowProtoTest
{
    [TestClass]
    public class MsgSession
    {
        [TestMethod]
        public void MsgLockRoundtrip()
        {
            var msg = new NowMsgSessionLock();

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
            }

            var expectedEncoded = new byte[]
            {
                0x00, 0x00, 0x00, 0x00, 0x12, 0x01, 0x00, 0x00,
            };

            CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
        }

        [TestMethod]
        public void MsgLogoff()
        {
            var msg = new NowMsgSessionLogoff();

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
            }

            var expectedEncoded = new byte[]
            {
                0x00, 0x00, 0x00, 0x00, 0x12, 0x02, 0x00, 0x00,
            };

            CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
        }

        [TestMethod]
        public void MsgMessageBoxReq()
        {
            var msg = new NowMsgSessionMessageBoxReq(0x76543210, "hello")
            {
                WaitForResponse = true,
                Style = NowMsgSessionMessageBoxReq.MessageBoxStyle.AbortRetryIgnore,
                Title = "world",
                Timeout = 3,
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
            }

            var expectedEncoded = new byte[]
            {
                0x1A, 0x00, 0x00, 0x00, 0x12, 0x03, 0x0F, 0x00,
                0x10, 0x32, 0x54, 0x76, 0x02, 0x00, 0x00, 0x00,
                0x03, 0x00, 0x00, 0x00, 0x05, 0x77, 0x6F, 0x72,
                0x6C, 0x64, 0x00, 0x05, 0x68, 0x65, 0x6C, 0x6C,
                0x6F, 0x00,
            };

            CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
        }

        [TestMethod]
        public void MsgMessageBoxRsp()
        {
            var encoded = new byte[]
            {
                0x08, 0x00, 0x00, 0x00, 0x12, 0x04, 0x00, 0x00,
                0x67, 0x45, 0x23, 0x01, 0x04, 0x00, 0x00, 0x00,
            };

            var msg = NowMessage
                .Read(new NowReadCursor(encoded))
                .Deserialize<NowMsgSessionMessageBoxRsp>();

            Assert.AreEqual((uint)0x01234567, msg.RequestId);
            Assert.AreEqual(NowMsgSessionMessageBoxRsp.MessageBoxResponse.Retry, msg.Response);
        }
    }
}

using NowProto.Messages;
using NowProto;

namespace NowProtoTest
{
    [TestClass]
    public class NowExecStyles
    {
        [TestMethod]
        public void MsgRun()
        {
            var msg = new NowMsgExecRun(0x1234567, "hello");

            var expectedEncoded = new byte[] {
                0x0B, 0x00, 0x00, 0x00, 0x13, 0x10, 0x00, 0x00,
                0x67, 0x45, 0x23, 0x01, 0x05, 0x68, 0x65, 0x6C,
                0x6C, 0x6F, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgProcess()
        {
            var msg = new NowMsgExecProcess(0x12345678, "a")
            {
                Parameters = "b",
                Directory = "c",
            };

            var expectedEncoded = new byte[] {
                0x0D, 0x00, 0x00, 0x00, 0x13, 0x12, 0x00, 0x00,
                0x78, 0x56, 0x34, 0x12, 0x01, 0x61, 0x00, 0x01,
                0x62, 0x00, 0x01, 0x63, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgShell()
        {
            var msg = new NowMsgExecShell(0x12345678, "a", "b");

            var expectedEncoded = new byte[] {
                0x0A, 0x00, 0x00, 0x00, 0x13, 0x13, 0x00, 0x00,
                0x78, 0x56, 0x34, 0x12, 0x01, 0x61, 0x00, 0x01,
                0x62, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgBatch()
        {
            var msg = new NowMsgExecBatch(0x12345678, "a");

            var expectedEncoded = new byte[] {
                0x07, 0x00, 0x00, 0x00, 0x13, 0x14, 0x00, 0x00,
                0x78, 0x56, 0x34, 0x12, 0x01, 0x61, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgWinPs()
        {
            var msg = new NowMsgExecWinPs(0x12345678, "a")
            {
                NoProfile = true,
                ExecutionPolicy = "b",
                ConfigurationName = "c",
            };

            var expectedEncoded = new byte[] {
                0x0D, 0x00, 0x00, 0x00, 0x13, 0x15, 0xD0, 0x00,
                0x78, 0x56, 0x34, 0x12, 0x01, 0x61, 0x00, 0x01,
                0x62, 0x00, 0x01, 0x63, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgPwsh()
        {
            var msg = new NowMsgExecPwsh(0x12345678, "a")
            {
                NoProfile = true,
                ExecutionPolicy = "b",
                ConfigurationName = "c",
            };

            var expectedEncoded = new byte[] {
                0x0D, 0x00, 0x00, 0x00, 0x13, 0x16, 0xD0, 0x00,
                0x78, 0x56, 0x34, 0x12, 0x01, 0x61, 0x00, 0x01,
                0x62, 0x00, 0x01, 0x63, 0x00
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }
    }
}

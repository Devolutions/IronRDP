using NowProto;
using NowProto.Messages;
using NowProto.Types;

namespace NowProtoTest
{
    [TestClass]
    public class MsgExecGeneral
    {
        [TestMethod]
        public void MsgAbort()
        {
            var msg = new NowMsgExecAbort(
                0x12345678,
                new NowStatus(NowStatus.StatusSeverity.Fatal, 0, (ushort)NowStatus.KnownStatusCode.Failure)
            );

            var expectedEncoded = new byte[] {
                0x08, 0x00, 0x00, 0x00, 0x13, 0x01, 0x00, 0x00,
                0x78, 0x56, 0x34, 0x12, 0xC0, 0x00, 0xFF, 0xFF
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgCapabilities()
        {
            var msg = new NowMsgExecCaps()
            {
                CapabilityBatch = true,
                CapabilityCmd = true,
                CapabilityRun = true,
                CapabilityProcess = true,
                CapabilityAppleScript = true,
                CapabilityPwsh = true,
                CapabilityShell = true,
                CapabilityWinPs = true,
            };

            var expectedEncoded = new byte[]
            {
                0x00, 0x00, 0x00, 0x00, 0x13, 0x00, 0xFF, 0x00
            };


            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }

            {
                var cursor = new NowReadCursor(actualEncoded);
                var actualMsg = NowMessage.Read(cursor).Deserialize<NowMsgExecCaps>();
                Assert.AreEqual(msg.CapabilityBatch, actualMsg.CapabilityBatch);
                Assert.AreEqual(msg.CapabilityCmd, actualMsg.CapabilityCmd);
                Assert.AreEqual(msg.CapabilityRun, actualMsg.CapabilityRun);
                Assert.AreEqual(msg.CapabilityProcess, actualMsg.CapabilityProcess);
                Assert.AreEqual(msg.CapabilityAppleScript, actualMsg.CapabilityAppleScript);
                Assert.AreEqual(msg.CapabilityPwsh, actualMsg.CapabilityPwsh);
                Assert.AreEqual(msg.CapabilityShell, actualMsg.CapabilityShell);
                Assert.AreEqual(msg.CapabilityWinPs, actualMsg.CapabilityWinPs);
            }
        }

        [TestMethod]
        public void MsgCancelReq()
        {
            var msg = new NowMsgExecCancelReq(0x12345678);

            var expectedEncoded = new byte[] {
                0x04, 0x00, 0x00, 0x00, 0x13, 0x02, 0x00, 0x00,
                0x78, 0x56, 0x34, 0x12
            };

            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }
        }

        [TestMethod]
        public void MsgCancelRsp()
        {
            var msg = new NowMsgExecCancelRsp(
                0x12345678,
                new NowStatus(NowStatus.StatusSeverity.Error, 0, (ushort)NowStatus.KnownStatusCode.Failure)
            );

            var encoded = new byte[] {
                0x08, 0x00, 0x00, 0x00, 0x13, 0x03, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x80, 0x00, 0xFF, 0xFF
            };

            {
                var cursor = new NowReadCursor(encoded);
                var actualMsg = NowMessage.Read(cursor).Deserialize<NowMsgExecCancelRsp>();
                Assert.AreEqual(msg.SessionId, actualMsg.SessionId);
                Assert.AreEqual(msg.Status.Severity, actualMsg.Status.Severity);
                Assert.AreEqual(msg.Status.Kind, actualMsg.Status.Kind);
                Assert.AreEqual(msg.Status.Status, actualMsg.Status.Status);
            }
        }

        [TestMethod]
        public void MsgResult()
        {
            var msg = new NowMsgExecResult(
                0x12345678,
                new NowStatus(NowStatus.StatusSeverity.Error, 0, (ushort)NowStatus.KnownStatusCode.Failure)
            );

            var encoded = new byte[] {
                0x08, 0x00, 0x00, 0x00, 0x13, 0x04, 0x00, 0x00, 0x78, 0x56, 0x34, 0x12, 0x80, 0x00, 0xFF, 0xFF
            };

            {
                var cursor = new NowReadCursor(encoded);
                var actualMsg = NowMessage.Read(cursor).Deserialize<NowMsgExecResult>();
                Assert.AreEqual(msg.SessionId, actualMsg.SessionId);
                Assert.AreEqual(msg.Status.Severity, actualMsg.Status.Severity);
                Assert.AreEqual(msg.Status.Kind, actualMsg.Status.Kind);
                Assert.AreEqual(msg.Status.Status, actualMsg.Status.Status);
            }
        }

        [TestMethod]
        public void MsgData()
        {
            var msg = new NowMsgExecData(
                0x12345678,
                NowMsgExecData.StreamKind.Stdin,
                new byte[] { 0x01, 0x02, 0x03 }
            ) { Fist = true };

            var expectedEncoded = new byte[]
            {
                0x08, 0x00, 0x00, 0x00, 0x13, 0x05, 0x05, 0x00, 0x78, 0x56, 0x34, 0x12, 0x03, 0x01, 0x02, 0x03
            };


            var actualEncoded = new byte[(msg as INowSerialize).Size];
            {
                var cursor = new NowWriteCursor(actualEncoded);
                (msg as INowSerialize).Serialize(cursor);
                CollectionAssert.AreEqual(expectedEncoded, actualEncoded);
            }

            {
                var cursor = new NowReadCursor(actualEncoded);
                var actualMsg = NowMessage.Read(cursor).Deserialize<NowMsgExecData>();
                Assert.AreEqual(msg.Fist, actualMsg.Fist);
                Assert.AreEqual(msg.Last, actualMsg.Last);
                Assert.AreEqual(msg.Stream, actualMsg.Stream);
                Assert.AreEqual(msg.SessionId, actualMsg.SessionId);
                CollectionAssert.AreEqual(msg.Data.ToArray(), actualMsg.Data.ToArray());
            }
        }
    }
}

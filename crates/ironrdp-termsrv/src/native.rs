use core::time::Duration;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context as _, anyhow};
use ironrdp_core::{Encode, EncodeResult, WriteCursor, impl_as_any};
use ironrdp_dvc::{DvcEncode, DvcMessage, DvcMessageBatch, DvcProcessor, DvcServerProcessor, encode_dvc_messages};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_server::ServerEvent;
use ironrdp_svc::{CHANNEL_CHUNK_LENGTH, ChannelFlags};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::native_pipe::NativePipe;
use crate::windows_main::generate_capture_helper_token;

pub(crate) const NOW_CHANNEL: &str = "Devolutions::Now::Agent";
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
const MAX_PENDING_BYTES: usize = 4 * 1024 * 1024;
const MAX_PENDING_MESSAGES: usize = 1024;
const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct NativeChannels {
    sender: StdMutex<Option<mpsc::UnboundedSender<ServerEvent>>>,
    session: watch::Sender<Option<u32>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl NativeChannels {
    pub(crate) fn attach(
        self: &Arc<Self>,
        sender: mpsc::UnboundedSender<ServerEvent>,
    ) -> anyhow::Result<ConnectionGuard> {
        *self
            .sender
            .lock()
            .map_err(|_| anyhow!("native channel sender lock poisoned"))? = Some(sender);
        Ok(ConnectionGuard(Arc::clone(self)))
    }

    pub(crate) fn set_session(&self, session_id: u32) {
        self.session.send_if_modified(|current| {
            if *current == Some(session_id) {
                false
            } else {
                *current = Some(session_id);
                true
            }
        });
    }

    pub(crate) async fn open(&self, session_id: u32) -> anyhow::Result<String> {
        let mut task = self
            .task
            .try_lock()
            .context("native channel open is already in progress")?;
        anyhow::ensure!(
            task.as_ref().is_none_or(JoinHandle::is_finished),
            "native channel is already open"
        );
        let mut binding = self.session.subscribe();
        anyhow::ensure!(
            session_id != 0 && session_id != u32::MAX && *binding.borrow_and_update() == Some(session_id),
            "native channel session does not match the connection"
        );
        let sender = self
            .sender
            .lock()
            .map_err(|_| anyhow!("native channel sender lock poisoned"))?
            .clone()
            .context("native channel transport is unavailable")?;

        let (incoming, incoming_rx) = mpsc::channel(MAX_PENDING_MESSAGES);
        let (opened, opened_rx) = oneshot::channel();
        let (closed, closed_rx) = oneshot::channel();
        let processor = NativeDvc {
            incoming,
            budget: Arc::new(Semaphore::new(MAX_PENDING_BYTES)),
            opened: Some(opened),
            closed: Some(closed),
            failed: false,
        };
        let (created, created_rx) = oneshot::channel();
        let channel_sender = sender.clone();
        sender
            .send(ServerEvent::CreateDynamicChannel(Box::new(move |id| {
                let _ = created.send(ChannelGuard {
                    id,
                    sender: channel_sender,
                });
                Box::new(processor)
            })))
            .map_err(|_| anyhow!("native channel event queue closed"))?;
        let pending = async {
            let channel = created_rx.await.context("native channel creation was rejected")?;
            opened_rx.await.context("native channel peer declined creation")?;
            let pipe_name = format!(r"\\.\pipe\IronRdpNativeDvc-{}", generate_capture_helper_token()?);
            let pipe = NativePipe::create(&pipe_name)?;
            anyhow::Ok((channel, pipe_name, pipe))
        };
        let (channel, pipe_name, pipe) = tokio::select! {
            biased;
            _ = binding.changed() => anyhow::bail!("native channel session changed during creation"),
            result = timeout(OPEN_TIMEOUT, pending) => result.context("native channel creation timed out")??,
        };
        *task = Some(tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = binding.changed() => Err(anyhow!("native channel session binding changed")),
                _ = closed_rx => Ok(()),
                result = pump(&channel, &pipe, incoming_rx) => result,
            };
            if let Err(error) = result {
                warn!(session_id, channel_id = channel.id, %error, "Native dynamic channel stopped");
            } else {
                debug!(session_id, channel_id = channel.id, "Native dynamic channel closed");
            }
        }));
        Ok(pipe_name)
    }
}

pub(crate) struct ConnectionGuard(Arc<NativeChannels>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.session.send_replace(None);
        match self.0.sender.lock() {
            Ok(mut sender) => *sender = None,
            Err(error) => warn!(%error, "Native channel sender lock poisoned during disconnect"),
        }
    }
}

struct ChannelGuard {
    id: u32,
    sender: mpsc::UnboundedSender<ServerEvent>,
}

impl Drop for ChannelGuard {
    fn drop(&mut self) {
        if self.sender.send(ServerEvent::CloseDynamicChannel(self.id)).is_err() {
            debug!(channel_id = self.id, "Native channel transport already disconnected");
        }
    }
}

struct NativeDvc {
    incoming: mpsc::Sender<IncomingMessage>,
    budget: Arc<Semaphore>,
    opened: Option<oneshot::Sender<()>>,
    closed: Option<oneshot::Sender<()>>,
    failed: bool,
}

struct IncomingMessage {
    payload: Vec<u8>,
    _budget: OwnedSemaphorePermit,
}

impl_as_any!(NativeDvc);

impl DvcProcessor for NativeDvc {
    fn channel_name(&self) -> &str {
        NOW_CHANNEL
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if let Some(opened) = self.opened.take() {
            let _ = opened.send(());
        }
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.failed {
            return Ok(Vec::new());
        }
        let budget = u32::try_from(payload.len().max(1))
            .ok()
            .filter(|_| payload.len() <= MAX_MESSAGE_SIZE)
            .and_then(|size| Arc::clone(&self.budget).try_acquire_many_owned(size).ok());
        let queued = budget.is_some_and(|budget| {
            self.incoming
                .try_send(IncomingMessage {
                    payload: payload.to_vec(),
                    _budget: budget,
                })
                .is_ok()
        });
        if !queued {
            warn!(
                channel_id,
                "Native channel receive limit exceeded or consumer disconnected"
            );
            self.failed = true;
            self.closed.take();
        }
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for NativeDvc {
    fn maximum_message_size(&self) -> usize {
        MAX_MESSAGE_SIZE
    }
}

async fn pump(
    channel: &ChannelGuard,
    pipe: &NativePipe,
    mut incoming: mpsc::Receiver<IncomingMessage>,
) -> anyhow::Result<()> {
    timeout(OPEN_TIMEOUT, pipe.connect())
        .await
        .context("native channel pipe connection timed out")??;
    let receive = async {
        while let Some(message) = incoming.recv().await {
            let payload = &message.payload;
            let length = u32::try_from(payload.len()).context("native channel payload length overflow")?;
            let mut offset = 0;
            loop {
                let end = (offset + CHANNEL_CHUNK_LENGTH).min(payload.len());
                let mut flags = ChannelControlFlags::empty();
                if offset == 0 {
                    flags |= ChannelControlFlags::FLAG_FIRST;
                }
                if end == payload.len() {
                    flags |= ChannelControlFlags::FLAG_LAST;
                }
                let mut record = ironrdp_core::encode_vec(&ChannelPduHeader { length, flags })?;
                record.extend_from_slice(&payload[offset..end]);
                pipe.write(record).await?;
                offset = end;
                if end == payload.len() {
                    break;
                }
            }
        }
        anyhow::Ok(())
    };
    let send = async {
        loop {
            let payload = pipe
                .read(MAX_MESSAGE_SIZE)
                .await
                .context("native pipe read worker failed")??;
            let messages = encode_dvc_messages(
                channel.id,
                vec![Box::new(Payload(payload))],
                ChannelFlags::SHOW_PROTOCOL,
            )?;
            let batch = DvcMessageBatch::try_new(channel.id, messages)?;
            let (written, completion) = oneshot::channel();
            channel
                .sender
                .send(ServerEvent::DynamicChannelWithCompletion(batch, written))
                .map_err(|_| anyhow!("native dynamic channel transport closed"))?;
            timeout(IO_TIMEOUT, completion)
                .await
                .context("native channel transport write timed out")?
                .context("native channel transport write failed")?;
        }
    };
    tokio::select! {
        result = receive => result,
        result = send => result,
    }
}

struct Payload(Vec<u8>);

impl DvcEncode for Payload {}

impl Encode for Payload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.0.encode(dst)
    }

    fn name(&self) -> &'static str {
        "NativeChannelPayload"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::io::{Read as _, Write as _};
    use std::os::windows::io::AsRawHandle as _;
    use windows::Win32::Foundation::HANDLE;

    use ironrdp_core::decode;
    use ironrdp_dvc::pdu::{DrdynvcDataPdu, DrdynvcServerPdu};

    use super::*;

    // oracle: spec WTSVirtualChannelRead remarks and CHANNEL_PDU_HEADER; independently chosen bytes.
    #[tokio::test]
    async fn native_reads_receive_chunk_headers_and_preserve_empty_messages() {
        let (_task, mut processor, _events, mut pipe) = pipe_fixture();
        processor.process(7, &vec![0xa5; 1601]).unwrap();
        processor.process(7, &[]).unwrap();
        let read = tokio::task::spawn_blocking(move || {
            let mut buffer = [0; 2048];
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(length, 1608);
            assert_eq!(&buffer[..8], &[0x41, 0x06, 0, 0, 1, 0, 0, 0]);
            assert!(buffer[8..length].iter().all(|byte| *byte == 0xa5));
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(&buffer[..length], &[0x41, 0x06, 0, 0, 2, 0, 0, 0, 0xa5]);
            let length = pipe.read(&mut buffer).unwrap();
            assert_eq!(&buffer[..length], &[0, 0, 0, 0, 3, 0, 0, 0]);
        });
        timeout(Duration::from_secs(5), read).await.unwrap().unwrap();
    }

    // oracle: mutant counting small transport chunks as full-sized messages rejects ordinary streamed stdin.
    #[tokio::test]
    async fn native_reads_accept_a_megabyte_split_into_small_messages() {
        let (_task, mut processor, _events, mut pipe) = pipe_fixture();
        for index in 0..256u16 {
            processor.process(7, &vec![index.to_le_bytes()[0]; 4096]).unwrap();
        }
        let read = tokio::task::spawn_blocking(move || {
            let mut record = [0; 1608];
            for index in 0..256u16 {
                let mut payload = Vec::new();
                while payload.len() < 4096 {
                    let count = pipe.read(&mut record).unwrap();
                    assert_eq!(&record[..4], &[0, 16, 0, 0]);
                    payload.extend_from_slice(&record[8..count]);
                }
                assert_eq!(payload, vec![index.to_le_bytes()[0]; 4096]);
            }
        });
        timeout(Duration::from_secs(5), read).await.unwrap().unwrap();
    }

    // oracle: mutant a blocked native reader also stalls the independent native-to-DVC direction.
    #[tokio::test]
    async fn native_output_progresses_while_native_input_is_blocked() {
        let (_task, mut processor, mut events, mut pipe) = pipe_fixture();
        processor.process(7, &vec![0xa5; 128 * 1024]).unwrap();
        let writer = tokio::task::spawn_blocking(move || {
            let mut record = [0; 1608];
            assert_eq!(pipe.read(&mut record).unwrap(), record.len());
            pipe.write_all(b"independent output").unwrap();
            pipe
        });
        let _pipe = timeout(Duration::from_secs(5), writer).await.unwrap().unwrap();
        let event = timeout(Duration::from_secs(2), events.recv()).await.unwrap().unwrap();
        let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
            panic!("expected native output while the peer is not reading input");
        };
        let pdu: DrdynvcServerPdu = decode(&batch.messages()[0].encode_unframed_pdu().unwrap()).unwrap();
        let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) = pdu else {
            panic!("expected an unfragmented output message");
        };
        assert_eq!(data.data(), b"independent output");
        completion.send(()).unwrap();
    }

    // oracle: mutant removing the 4 MiB receive bound allows arbitrarily large queued payloads.
    #[tokio::test]
    async fn excessive_pending_bytes_close_the_native_channel() {
        let (_native, _connection, mut processor, mut events) = open_fixture().await;
        for _ in 0..4 {
            processor.process(7, &vec![0; 1024 * 1024]).unwrap();
        }
        processor.process(7, &[1]).unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(7))
        ));
    }

    // oracle: mutant zero-length messages bypass the payload bound and grow the queue without limit.
    #[tokio::test]
    async fn excessive_pending_empty_messages_close_the_native_channel() {
        let (_native, _connection, mut processor, mut events) = open_fixture().await;
        for _ in 0..1025 {
            processor.process(7, &[]).unwrap();
        }
        assert!(matches!(
            timeout(Duration::from_secs(1), events.recv()).await.unwrap(),
            Some(ServerEvent::CloseDynamicChannel(7))
        ));
    }

    // oracle: mutant treating a zero-byte native message as EOF or repeating it instead of reading the next message.
    #[tokio::test]
    async fn native_writes_preserve_empty_messages_between_nonempty_messages() {
        let (_task, _processor, mut events, mut pipe) = pipe_fixture();
        let writes = tokio::task::spawn_blocking(move || {
            for payload in [b"first".as_slice(), b"", b"last"] {
                assert_eq!(pipe.write(payload).unwrap(), payload.len());
            }
            pipe
        });
        for expected in [b"first".as_slice(), b"", b"last"] {
            let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
            let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
                panic!("expected a native data message");
            };
            assert_eq!(batch.channel_id(), 7);
            assert_eq!(batch.messages().len(), 1);
            let pdu: DrdynvcServerPdu = decode(&batch.messages()[0].encode_unframed_pdu().unwrap()).unwrap();
            let DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) = pdu else {
                panic!("expected a short data PDU");
            };
            assert_eq!(data.data(), expected);
            completion.send(()).unwrap();
        }
        let _pipe = writes.await.unwrap();
    }

    // oracle: mutant truncating a native write at an internal pipe buffer boundary.
    #[tokio::test]
    async fn a_large_native_write_remains_one_complete_dvc_message() {
        let (_task, _processor, mut events, mut pipe) = pipe_fixture();
        let writes = tokio::task::spawn_blocking(move || {
            assert_eq!(pipe.write(&vec![0x6b; 1024 * 1024]).unwrap(), 1024 * 1024);
            pipe
        });
        let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        let ServerEvent::DynamicChannelWithCompletion(batch, completion) = event else {
            panic!("expected a native data message");
        };
        let mut payload = Vec::new();
        for (index, message) in batch.messages().iter().enumerate() {
            let pdu: DrdynvcServerPdu = decode(&message.encode_unframed_pdu().unwrap()).unwrap();
            match pdu {
                DrdynvcServerPdu::Data(DrdynvcDataPdu::DataFirst(data)) => {
                    assert_eq!(index, 0);
                    assert_eq!(data.length(), 1024 * 1024);
                    payload.extend_from_slice(data.data());
                }
                DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(data)) => payload.extend_from_slice(data.data()),
                _ => panic!("expected uncompressed data"),
            }
        }
        assert_eq!(payload, vec![0x6b; 1024 * 1024]);
        completion.send(()).unwrap();
        let _pipe = writes.await.unwrap();
    }

    // oracle: mutant keeping a native process connected after reassignment to a different desktop.
    #[tokio::test]
    async fn rebinding_closes_the_native_channel_and_rejects_the_old_session() {
        let (native, _connection, _processor, mut events) = open_fixture().await;
        native.set_session(8);
        let event = timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        assert!(matches!(event, ServerEvent::CloseDynamicChannel(7)));
        assert!(native.open(5).await.is_err());
    }

    // oracle: mutant retaining an allocated DVC after the control caller abandons its open request.
    #[tokio::test]
    async fn abandoned_open_releases_the_reserved_channel() {
        let native = Arc::new(NativeChannels::default());
        native.set_session(5);
        let (sender, mut events) = mpsc::unbounded_channel();
        let _connection = native.attach(sender).unwrap();
        let pending = tokio::spawn({
            let native = Arc::clone(&native);
            async move { native.open(5).await }
        });
        let ServerEvent::CreateDynamicChannel(build) = events.recv().await.unwrap() else {
            panic!("expected a creation request");
        };
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        let _processor = build(7);
        assert!(matches!(events.recv().await, Some(ServerEvent::CloseDynamicChannel(7))));
    }

    async fn open_fixture() -> (
        Arc<NativeChannels>,
        ConnectionGuard,
        Box<dyn DvcServerProcessor>,
        mpsc::UnboundedReceiver<ServerEvent>,
    ) {
        let native = Arc::new(NativeChannels::default());
        native.set_session(5);
        let (sender, mut events) = mpsc::unbounded_channel();
        let connection = native.attach(sender).unwrap();
        let pending = tokio::spawn({
            let native = Arc::clone(&native);
            async move { native.open(5).await }
        });
        let ServerEvent::CreateDynamicChannel(build) = events.recv().await.unwrap() else {
            panic!("expected a creation request");
        };
        let mut processor = build(7);
        processor.start(7).unwrap();
        pending.await.unwrap().unwrap();
        (native, connection, processor, events)
    }

    fn pipe_fixture() -> (AbortOnDrop, NativeDvc, mpsc::UnboundedReceiver<ServerEvent>, File) {
        let name = format!(
            r"\\.\pipe\IronRdpNativeTest-{}",
            generate_capture_helper_token().unwrap()
        );
        let server = NativePipe::for_test(&name).unwrap();
        let pipe = OpenOptions::new().read(true).write(true).open(name).unwrap();
        // SAFETY: This is a live message-pipe handle, exclusively owned by the test.
        unsafe {
            windows::Win32::System::Pipes::SetNamedPipeHandleState(
                HANDLE(pipe.as_raw_handle()),
                Some(&windows::Win32::System::Pipes::PIPE_READMODE_MESSAGE),
                None,
                None,
            )
        }
        .unwrap();
        let (sender, events) = mpsc::unbounded_channel();
        let channel = ChannelGuard { id: 7, sender };
        let (incoming, receiver) = mpsc::channel(MAX_PENDING_MESSAGES);
        let processor = NativeDvc {
            incoming,
            budget: Arc::new(Semaphore::new(MAX_PENDING_BYTES)),
            opened: None,
            closed: None,
            failed: false,
        };
        let task = tokio::spawn(async move {
            pump(&channel, &server, receiver).await.unwrap();
        });
        (AbortOnDrop(task), processor, events, pipe)
    }

    struct AbortOnDrop(JoinHandle<()>);

    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
}

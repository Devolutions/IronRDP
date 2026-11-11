use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use clap::Parser;
use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_server::{
    ConnectionHandler, DesktopSize, DisplayContext, DisplayUpdate, PostConnectionAction, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpdrServerBackend, RdpdrServerFactory, RdpdrServerMessage, ServerError, ServerEvent,
    ServerEventSender, ServerResult,
};
use tokio::sync::mpsc::UnboundedSender;

const TIMEOUT: Duration = Duration::from_secs(30);
const PAYLOAD: &[u8] = b"RDPDR wire policy\n";

#[derive(Parser)]
struct Args {
    #[arg(long)]
    artifacts: PathBuf,
}

#[derive(Debug)]
enum Reply {
    Device(u32),
    Create(NtStatus, u32, u8),
    Read(NtStatus, usize),
    Write(NtStatus, u32),
    Flush(NtStatus),
    SetInformation(NtStatus),
    Directory(NtStatus, bool),
    DeviceControl(NtStatus, bool),
    Close(NtStatus),
}

struct Factory {
    sender: Arc<Mutex<Option<UnboundedSender<ServerEvent>>>>,
    replies: mpsc::Sender<Reply>,
}

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        *self.sender.lock().unwrap() = Some(sender);
    }
}

impl RdpdrServerFactory for Factory {
    fn build_backend(&self) -> Box<dyn RdpdrServerBackend> {
        Box::new(Backend(self.replies.clone()))
    }
}

#[derive(Debug)]
struct Backend(mpsc::Sender<Reply>);

impl_as_any!(Backend);

impl Backend {
    fn reply(&self, reply: Reply) -> PduResult<()> {
        self.0
            .send(reply)
            .map_err(|_| ironrdp_pdu::pdu_other_err!("policy fixture receiver closed"))
    }
}

macro_rules! unexpected_completions {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(fn $method(&mut self, _: &$ty) -> PduResult<()> {
            Err(ironrdp_pdu::pdu_other_err!("unexpected policy fixture completion"))
        })*
    };
}

impl RdpdrServerBackend for Backend {
    fn on_device_announce(&mut self, devices: &[DeviceAnnounceHeader]) -> Vec<(u32, bool)> {
        devices
            .iter()
            .map(|device| {
                let accepted = device.device_type() == DeviceType::Filesystem && device.preferred_dos_name() == "DL";
                if accepted && self.0.send(Reply::Device(device.device_id())).is_err() {
                    eprintln!("policy fixture receiver closed before device announcement");
                    return (device.device_id(), false);
                }
                (device.device_id(), accepted)
            })
            .collect()
    }

    fn on_device_remove(&mut self, _: &[u32]) {}

    fn on_client_name(&mut self, _: &str) {}

    fn on_create_complete(&mut self, response: &DeviceCreateResponse) -> PduResult<()> {
        self.reply(Reply::Create(
            response.device_io_reply.io_status,
            response.file_id,
            response.information.bits(),
        ))
    }

    fn on_read_complete(&mut self, response: &DeviceReadResponse) -> PduResult<()> {
        self.reply(Reply::Read(
            response.device_io_reply.io_status,
            response.read_data.len(),
        ))
    }

    fn on_write_complete(&mut self, response: &DeviceWriteResponse) -> PduResult<()> {
        self.reply(Reply::Write(response.device_io_reply.io_status, response.length))
    }

    fn on_flush_buffers_complete(&mut self, response: &DeviceFlushBuffersResponse) -> PduResult<()> {
        self.reply(Reply::Flush(response.device_io_response.io_status))
    }

    fn on_set_information_complete(&mut self, response: &ClientDriveSetInformationResponse) -> PduResult<()> {
        self.reply(Reply::SetInformation(response.device_io_reply().io_status))
    }

    fn on_query_directory_complete(&mut self, response: &ClientDriveQueryDirectoryResponse) -> PduResult<()> {
        self.reply(Reply::Directory(
            response.device_io_reply.io_status,
            response.buffer.is_none(),
        ))
    }

    fn on_close_complete(&mut self, response: &DeviceCloseResponse) -> PduResult<()> {
        self.reply(Reply::Close(response.device_io_response.io_status))
    }

    fn on_device_control_complete(&mut self, response: &DeviceControlResponse) -> PduResult<()> {
        self.reply(Reply::DeviceControl(
            response.device_io_reply.io_status,
            response.output_buffer.is_none(),
        ))
    }

    unexpected_completions! {
        on_query_information_complete(ClientDriveQueryInformationResponse),
        on_notify_change_directory_complete(ClientDriveNotifyChangeDirectoryResponse),
        on_query_volume_information_complete(ClientDriveQueryVolumeInformationResponse),
        on_lock_control_complete(ClientDriveLockControlResponse),
        on_query_security_complete(ClientDriveQuerySecurityResponse),
        on_set_security_complete(ClientDriveSetSecurityResponse),
    }
}

struct Display;

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1280,
            height: 800,
        }
    }

    async fn updates(&mut self, _: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(Updates))
    }
}

struct Updates;

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        std::future::pending().await
    }
}

struct StopOnDisconnect;

impl ConnectionHandler for StopOnDisconnect {
    fn on_disconnected(&mut self, _: SocketAddr, _: Duration, _: Option<&ServerError>) -> PostConnectionAction {
        PostConnectionAction::Stop
    }
}

fn request(
    sender: &UnboundedSender<ServerEvent>,
    replies: &mpsc::Receiver<Reply>,
    message: RdpdrServerMessage,
) -> anyhow::Result<Reply> {
    sender
        .send(ServerEvent::Rdpdr(message))
        .map_err(|_| anyhow::anyhow!("policy fixture request channel closed"))?;
    replies.recv_timeout(TIMEOUT).context("receive policy fixture response")
}

fn create(device_id: u32, path: &str, access: DesiredAccess, disposition: CreateDisposition) -> RdpdrServerMessage {
    RdpdrServerMessage::Create {
        device_id,
        path: path.to_owned(),
        desired_access: access,
        create_disposition: disposition,
        create_options: CreateOptions::FILE_NON_DIRECTORY_FILE,
    }
}

fn rename(device_id: u32, file_id: u32, name: &str, replace: Boolean) -> RdpdrServerMessage {
    RdpdrServerMessage::SetInformation {
        device_id,
        file_id,
        set_buffer: FileInformationClass::Rename(FileRenameInformation {
            replace_if_exists: replace,
            file_name: name.to_owned(),
        }),
    }
}

fn check(label: &str, valid: bool, response: &Reply) -> anyhow::Result<()> {
    anyhow::ensure!(valid, "{label}: unexpected client response {response:?}");
    println!("PASS {label}");
    Ok(())
}

fn probe(
    artifacts: &Path,
    sender: Arc<Mutex<Option<UnboundedSender<ServerEvent>>>>,
    replies: mpsc::Receiver<Reply>,
) -> anyhow::Result<()> {
    let device_id = match replies.recv_timeout(TIMEOUT).context("wait for download drive")? {
        Reply::Device(id) => id,
        reply => anyhow::bail!("unexpected drive announcement: {reply:?}"),
    };
    let deadline = Instant::now() + TIMEOUT;
    while !artifacts.join("go").exists() {
        anyhow::ensure!(Instant::now() < deadline, "timed out waiting for policy test start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let sender = sender.lock().unwrap().clone().context("missing RDPDR event sender")?;
    let write = DesiredAccess::GENERIC_WRITE | DesiredAccess::DELETE;

    // oracle: mutant an RDP server can open a pre-existing client file for reading or mutation.
    for (label, access, disposition) in [
        (
            "existing-read",
            DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
            CreateDisposition::FILE_OPEN,
        ),
        (
            "existing-overwrite",
            write.clone(),
            CreateDisposition::FILE_OVERWRITE_IF,
        ),
        ("existing-open-write", write.clone(), CreateDisposition::FILE_OPEN),
    ] {
        let response = request(
            &sender,
            &replies,
            create(device_id, "existing.bin", access, disposition),
        )?;
        check(
            label,
            matches!(response, Reply::Create(NtStatus::ACCESS_DENIED, _, _)),
            &response,
        )?;
    }

    // oracle: mutant a filesystem request escapes the approved directory or follows an existing link.
    let response = request(
        &sender,
        &replies,
        create(
            device_id,
            r"..\outside.bin",
            write.clone(),
            CreateDisposition::FILE_CREATE,
        ),
    )?;
    check(
        "create-traversal",
        matches!(response, Reply::Create(NtStatus::OBJECT_NAME_INVALID, _, _)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        create(device_id, "link.bin", write.clone(), CreateDisposition::FILE_CREATE),
    )?;
    check(
        "create-link",
        matches!(response, Reply::Create(NtStatus::OBJECT_NAME_COLLISION, _, _)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::Create {
            device_id,
            path: "directory".to_owned(),
            desired_access: write.clone(),
            create_disposition: CreateDisposition::FILE_CREATE,
            create_options: CreateOptions::FILE_DIRECTORY_FILE,
        },
    )?;
    check(
        "create-directory",
        matches!(response, Reply::Create(NtStatus::ACCESS_DENIED, _, _)),
        &response,
    )?;

    let response = request(
        &sender,
        &replies,
        create(device_id, "wire.part", write, CreateDisposition::FILE_CREATE),
    )?;
    let file_id = match &response {
        Reply::Create(NtStatus::SUCCESS, id, information) => {
            // oracle: spec MS-RDPEFS §2.2.3.4.1
            check("create-information", *information == 0, &response)?;
            *id
        }
        _ => anyhow::bail!("exclusive temporary create failed: {response:?}"),
    };
    println!("PASS create-temporary");

    // oracle: mutant a writable transfer handle permits reads or directory enumeration.
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::Read {
            device_id,
            file_id,
            length: 64,
            offset: 0,
        },
    )?;
    check(
        "read-denied",
        matches!(response, Reply::Read(NtStatus::ACCESS_DENIED, 0)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::QueryDirectory {
            device_id,
            file_id,
            info_class: FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
            path: "*".to_owned(),
            initial_query: true,
        },
    )?;
    check(
        "listing-denied",
        matches!(response, Reply::Directory(NtStatus::ACCESS_DENIED, true)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::DeviceControl {
            device_id,
            file_id,
            io_control_code: 0,
            input_buffer: Vec::new(),
            output_buffer_length: 0,
        },
    )?;
    check(
        "device-control-denied",
        matches!(response, Reply::DeviceControl(NtStatus::ACCESS_DENIED, true)),
        &response,
    )?;

    // oracle: mutant publication replaces an existing client file or escapes the approved directory.
    for (label, name, replace, status) in [
        ("replace-denied", "existing.bin", Boolean::True, NtStatus::ACCESS_DENIED),
        (
            "collision-denied",
            "existing.bin",
            Boolean::False,
            NtStatus::OBJECT_NAME_COLLISION,
        ),
        (
            "rename-link",
            "link.bin",
            Boolean::False,
            NtStatus::OBJECT_NAME_COLLISION,
        ),
        (
            "rename-traversal",
            r"..\outside.bin",
            Boolean::False,
            NtStatus::OBJECT_NAME_INVALID,
        ),
    ] {
        let response = request(&sender, &replies, rename(device_id, file_id, name, replace))?;
        check(
            label,
            matches!(response, Reply::SetInformation(actual) if actual == status),
            &response,
        )?;
    }
    let response = request(
        &sender,
        &replies,
        rename(device_id, u32::MAX, "existing.bin", Boolean::False),
    )?;
    check(
        "unowned-handle-denied",
        matches!(
            response,
            Reply::SetInformation(NtStatus::NO_SUCH_FILE | NtStatus::INVALID_HANDLE)
        ),
        &response,
    )?;

    // oracle: spec MS-RDPEFS §3.1.5.2 (an unknown FileId cannot identify an open device).
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::Close {
            device_id,
            file_id: u32::MAX,
        },
    )?;
    check(
        "unowned-close-denied",
        matches!(response, Reply::Close(status) if status != NtStatus::SUCCESS),
        &response,
    )?;

    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::Write {
            device_id,
            file_id,
            data: PAYLOAD.to_vec(),
            offset: 0,
        },
    )?;
    check(
        "write-temporary",
        matches!(response, Reply::Write(NtStatus::SUCCESS, size) if size as usize == PAYLOAD.len()),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::FlushBuffers { device_id, file_id },
    )?;
    check(
        "flush-temporary",
        matches!(response, Reply::Flush(NtStatus::SUCCESS)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        rename(device_id, file_id, "delivered.bin", Boolean::False),
    )?;
    check(
        "publish",
        matches!(response, Reply::SetInformation(NtStatus::SUCCESS)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::Write {
            device_id,
            file_id,
            data: vec![1],
            offset: 0,
        },
    )?;
    check(
        "published-write-denied",
        matches!(response, Reply::Write(NtStatus::ACCESS_DENIED, 0)),
        &response,
    )?;
    let response = request(
        &sender,
        &replies,
        RdpdrServerMessage::SetInformation {
            device_id,
            file_id,
            set_buffer: FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
        },
    )?;
    check(
        "published-delete-denied",
        matches!(response, Reply::SetInformation(NtStatus::ACCESS_DENIED)),
        &response,
    )?;
    let response = request(&sender, &replies, RdpdrServerMessage::Close { device_id, file_id })?;
    check("close", matches!(response, Reply::Close(NtStatus::SUCCESS)), &response)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    runtime.block_on(async move {
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])?;
        std::fs::write(args.artifacts.join("server.pem"), identity.cert.pem())?;
        let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.cert.der().clone()],
            tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
        )?;
        let sender = Arc::new(Mutex::new(None));
        let (replies_tx, replies_rx) = mpsc::channel();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let probe_sender = Arc::clone(&sender);
        let artifacts = args.artifacts.clone();
        std::thread::spawn(move || {
            let result = probe(&artifacts, probe_sender, replies_rx);
            if done_tx.send(result).is_err() {
                eprintln!("policy fixture controller closed");
            }
        });
        let mut server = RdpServer::builder()
            .with_addr(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .with_tls(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
            .with_no_input()
            .with_display_handler(Display)
            .with_connection_handler(Some(Box::new(StopOnDisconnect)))
            .with_rdpdr_factory(Some(Box::new(Factory {
                sender,
                replies: replies_tx,
            })))
            .build();
        let ready_handle = server.handle().clone();
        let stop_handle = server.handle().clone();
        let ready = async {
            let addr = ready_handle
                .local_addr()
                .await?
                .context("RDP listener has no address")?;
            std::fs::write(args.artifacts.join("server.addr"), addr.to_string())?;
            Ok::<(), anyhow::Error>(())
        };
        let control = async {
            let result = tokio::time::timeout(TIMEOUT * 2, done_rx)
                .await
                .context("policy fixture timed out")?
                .context("policy fixture worker stopped")?;
            stop_handle.quit("policy fixture completed")?;
            result
        };
        let serving = async { server.run().await.map_err(anyhow::Error::from) };
        tokio::try_join!(ready, control, serving)?;
        Ok(())
    })
}

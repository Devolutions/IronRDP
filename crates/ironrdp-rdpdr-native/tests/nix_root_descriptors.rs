#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::ops::Range;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use ironrdp_core as _;
use ironrdp_pdu as _;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceIoRequest,
    FileAttributes, FileInformationClass, FileRenameInformation, MajorFunction, MinorFunction, NtStatus,
    ServerDriveIoRequest, ServerDriveSetInformationRequest, SharedAccess,
};
use ironrdp_rdpdr::{RdpdrBackend, RdpdrBackendFactory};
use ironrdp_rdpdr_native::backend::NixRdpdrBackendFactory;
use ironrdp_svc::SvcMessage;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::resource::{Resource, getrlimit, rlim_t, setrlimit};
use nix::sys::stat::Mode;
use rstest as _;
use tracing as _;

/// The devices given the enclosing root and the root inside it.
const OUTER: u32 = 1;
const INNER: u32 = 2;

/// The soft descriptor limit while the test runs, low enough to fill in a few dozen opens.
const DESCRIPTOR_LIMIT: rlim_t = 64;

/// A backend built while the process has too few descriptors to resolve every root does not serve a root through a link that a server placed on its path.
/// A walk that runs out of descriptors leaves the directory of its root undetermined, so the path of the other root cannot be checked against it.
/// The root at `a/b` then reaches a directory outside both roots through the link, and a create through its drive must not land there, however many descriptors were left and in whichever order the roots are given.
///
/// This is the only test of its file, so it has a process to itself and nothing else opens or closes a descriptor while it counts them.
// oracle: mutant a root reached through a link served while the walk of another root ran out of descriptors
#[test]
fn a_root_whose_path_a_server_led_through_a_link_serves_nothing_outside_when_descriptors_run_out() {
    let (soft_limit, hard_limit) = getrlimit(Resource::RLIMIT_NOFILE).expect("read the descriptor limit");
    setrlimit(Resource::RLIMIT_NOFILE, soft_limit.min(DESCRIPTOR_LIMIT), hard_limit)
        .expect("lower the descriptor limit");
    let sandbox = Sandbox::new();

    for free_descriptors in 0..=8 {
        for order in [[OUTER, INNER], [INNER, OUTER]] {
            let tree = sandbox.0.join(format!("{free_descriptors}-free-{}-first", order[0]));
            let (outer, outside) = (tree.join("outer"), tree.join("outside"));
            for directory in [&outer.join("a").join("b"), &outer.join("d"), &outside] {
                std::fs::create_dir_all(directory).expect("create directory");
            }
            std::os::unix::fs::symlink(&outside, outer.join("d").join("b")).expect("link directory");
            let factory = order
                .into_iter()
                .fold(NixRdpdrBackendFactory::default(), |factory, device_id| {
                    if device_id == OUTER {
                        factory.with_drive_root(OUTER, "outer".to_owned(), utf8(&outer))
                    } else {
                        factory.with_drive_root(INNER, "inner".to_owned(), utf8(&outer.join("a").join("b")))
                    }
                });
            let (mut earlier, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
            rename_through_the_outer_drive(earlier.as_mut(), r"\a", r"\a_old");
            rename_through_the_outer_drive(earlier.as_mut(), r"\d", r"\a");
            drop(earlier);

            let fillers = fill_descriptors_leaving(free_descriptors);
            let (mut later, _) = factory.build_rdpdr_backend().expect("valid configuration").into_parts();
            drop(fillers);
            let _ = create(later.as_mut(), INNER, r"\new.txt", CreateDisposition::FILE_CREATE);

            assert_eq!(
                std::fs::read_dir(&outside).expect("list directory").count(),
                0,
                "{free_descriptors} descriptors left, roots given in the order {order:?}"
            );
        }
    }

    setrlimit(Resource::RLIMIT_NOFILE, soft_limit, hard_limit).expect("restore the descriptor limit");
}

/// Opens `/dev/null` until the process has no descriptor left, then closes `free` of those descriptors and returns the others.
fn fill_descriptors_leaving(free: usize) -> Vec<OwnedFd> {
    let mut fillers = Vec::new();
    loop {
        match nix::fcntl::open("/dev/null", OFlag::O_RDONLY | OFlag::O_CLOEXEC, Mode::empty()) {
            Ok(filler) => fillers.push(filler),
            Err(error) => {
                assert_eq!(error, Errno::EMFILE, "open /dev/null");
                break;
            }
        }
    }
    let kept = fillers
        .len()
        .checked_sub(free)
        .expect("more fillers than descriptors to free");
    fillers.truncate(kept);
    fillers
}

/// Renames `from` to `to` through the outer drive, with a handle opened for the rename and closed after it.
fn rename_through_the_outer_drive(backend: &mut dyn RdpdrBackend, from: &str, to: &str) {
    let opened = create(backend, OUTER, from, CreateDisposition::FILE_OPEN);
    assert_eq!(response_status(&opened), NtStatus::SUCCESS, "open {from}");
    let file_id = read_u32(&opened, CREATE_FILE_ID);
    let renamed = backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(
            ServerDriveSetInformationRequest {
                device_io_request: header(OUTER, file_id, MajorFunction::SetInformation),
                set_buffer: FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: to.to_owned(),
                }),
            },
        ))
        .expect("complete rename");
    assert_eq!(response_status(&renamed), NtStatus::SUCCESS, "rename {from} to {to}");
    backend
        .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
            device_io_request: header(OUTER, file_id, MajorFunction::Close),
        }))
        .expect("complete close");
}

/// Sends a create of `path` through `device_id`, with the access a rename needs.
fn create(
    backend: &mut dyn RdpdrBackend,
    device_id: u32,
    path: &str,
    create_disposition: CreateDisposition,
) -> Vec<SvcMessage> {
    let request = DeviceCreateRequest {
        device_io_request: header(device_id, 0, MajorFunction::Create),
        desired_access: DesiredAccess::GENERIC_ALL,
        allocation_size: 0,
        file_attributes: FileAttributes::empty(),
        shared_access: SharedAccess::empty(),
        create_disposition,
        create_options: CreateOptions::empty(),
        path: path.to_owned(),
    };
    backend
        .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
        .expect("complete create")
}

fn header(device_id: u32, file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id,
        file_id,
        completion_id: 0,
        major_function,
        minor_function: MinorFunction::from(0),
    }
}

// Every response starts with an RDPDR_HEADER, Component and PacketId, followed by DeviceId, CompletionId and IoStatus,
// all four bytes ([MS-RDPEFS] 2.2.1.1 Shared Header (RDPDR_HEADER) and 2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)).
const IO_STATUS: Range<usize> = 12..16;
// [MS-RDPEFS] 2.2.1.5.1 Device Create Response (DR_CREATE_RSP) continues with FileId.
const CREATE_FILE_ID: Range<usize> = 16..20;

fn response_status(messages: &[SvcMessage]) -> NtStatus {
    NtStatus::from(read_u32(messages, IO_STATUS))
}

fn read_u32(messages: &[SvcMessage], field: Range<usize>) -> u32 {
    let response = messages[0].encode_unframed_pdu().expect("encode RDPDR response");
    u32::from_le_bytes(response[field].try_into().expect("four-byte field"))
}

fn utf8(path: &Path) -> String {
    path.to_str().expect("UTF-8 temporary directory").to_owned()
}

/// A temporary directory for the trees of the test, removed when dropped.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let sandbox = std::env::temp_dir().join(format!("rdpdr-native-nix-descriptors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sandbox);
        Self(sandbox)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

//! Browser-side drive redirection for RDPDR.
//!
//! A browser cannot open local files, so the backend hands every filesystem
//! IRP that needs them to JavaScript and answers the server once JavaScript
//! replies:
//!
//! * [`WasmDriveBackend`] lives inside the RDPDR SVC processor (`Send`). It
//!   decodes the IRP into a [`DriveRequest`], queues it to the session event
//!   loop and remembers how to encode the completion. IRPs that need no local
//!   state (flush, locks, device control, volume attributes) are answered at
//!   once.
//! * The event loop calls the JavaScript `drive_request_callback` with each
//!   request (see [`DriveRequest::to_js`]). JavaScript answers through
//!   `Session.invokeExtension(driveReply({...}))`; the reply goes back to the
//!   backend through [`WasmDriveBackend::complete`], which returns the
//!   completion PDUs.
//!
//! Requests carry an ID of their own rather than the server's completion ID,
//! so a reply that arrives after the channel was reset is recognized as stale
//! and dropped. Directory listings are fetched whole on the initial query and
//! served from memory afterwards, so enumerating a directory costs one round
//! trip instead of one per entry. The metadata that a `create` reply carries
//! answers information queries until something changes the drive, and closing
//! a handle that changed nothing completes at once: JavaScript gets the close
//! as a notification (ID 0) and must handle requests in order.
//!
//! Times cross to JavaScript as Unix milliseconds, which stay exact in a
//! JavaScript number, unlike Windows FILETIME values.

use std::collections::{BTreeMap, HashMap, VecDeque};

use futures_channel::mpsc;
use ironrdp::rdpdr::pdu::RdpdrPdu;
use ironrdp::rdpdr::pdu::efs::{
    Boolean, Characteristics, ClientDriveLockControlResponse, ClientDriveQueryDirectoryResponse,
    ClientDriveQueryInformationResponse, ClientDriveQuerySecurityResponse, ClientDriveQueryVolumeInformationResponse,
    ClientDriveSetInformationResponse, ClientDriveSetSecurityResponse, DeviceCloseResponse, DeviceControlResponse,
    DeviceCreateResponse, DeviceFlushBuffersResponse, DeviceIoRequest, DeviceIoResponse, DeviceReadResponse,
    DeviceWriteResponse, FileAttributeTagInformation, FileAttributes, FileBasicInformation,
    FileBothDirectoryInformation, FileDirectoryInformation, FileFsAttributeInformation, FileFsDeviceInformation,
    FileFsFullSizeInformation, FileFsSizeInformation, FileFsVolumeInformation, FileFullDirectoryInformation,
    FileInformationClass, FileInformationClassLevel, FileNamesInformation, FileStandardInformation,
    FileSystemAttributes, FileSystemInformationClass, FileSystemInformationClassLevel, Information, NtStatus,
    ServerDriveIoRequest, ServerDriveSetInformationRequest,
};
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::SvcMessage;
use tracing::{debug, warn};
use wasm_bindgen::prelude::*;

use crate::session::RdpInputEvent;

/// Most IRPs waiting for JavaScript at once; later ones fail with
/// STATUS_INSUFFICIENT_RESOURCES.
const MAX_PENDING: usize = 4096;
/// Most entries kept for one directory listing.
const MAX_LISTING: usize = 100_000;
/// Largest read answered; the server asks for far less in practice.
const MAX_READ: u32 = 16 * 1024 * 1024;

/// FILETIME of the Unix epoch, in 100 ns units since 1601.
const FILETIME_UNIX_EPOCH: i64 = 116_444_736_000_000_000;
const FILETIME_PER_MS: i64 = 10_000;
/// FILE_DEVICE_DISK ([MS-FSCC] 2.5.10).
const FILE_DEVICE_DISK: u32 = 0x0000_0007;
/// Reported allocation unit: 8 sectors of 512 bytes.
const BYTES_PER_SECTOR: u32 = 512;
const SECTORS_PER_UNIT: u32 = 8;
/// FILE_DELETE_ON_CLOSE create option.
const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;

fn ms_to_filetime(ms: f64) -> i64 {
    if !ms.is_finite() || ms <= 0.0 {
        return 0;
    }
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "clamped below")]
    let ms = ms.min(1e15) as i64;
    ms.saturating_mul(FILETIME_PER_MS).saturating_add(FILETIME_UNIX_EPOCH)
}

/// Converts a FILETIME from a set-information request. 0 and -1 mean "leave
/// unchanged" ([MS-FSCC] 2.4.7), which is reported as 0.
fn filetime_to_ms(filetime: i64) -> f64 {
    if filetime <= FILETIME_UNIX_EPOCH {
        return 0.0;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_precision_loss,
        reason = "milliseconds fit in f64"
    )]
    let ms = ((filetime - FILETIME_UNIX_EPOCH) / FILETIME_PER_MS) as f64;
    ms
}

/// One redirected folder, as announced to the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DriveSpec {
    pub(crate) device_id: u32,
    pub(crate) name: String,
}

/// What JavaScript is asked to do with the local filesystem.
#[derive(Debug, PartialEq)]
pub(crate) enum DriveOp {
    /// Opens or creates `path` (`\`-separated, relative to the drive root).
    /// The numbers are the raw MS-SMB2 create fields.
    Create {
        path: String,
        desired_access: u32,
        disposition: u32,
        options: u32,
        attributes: u32,
        share_access: u32,
    },
    Close,
    Read {
        offset: u64,
        length: u32,
    },
    Write {
        offset: u64,
        data: Vec<u8>,
    },
    /// Metadata of the open file.
    Stat,
    /// Every entry of the directory in `path` matching its last component
    /// (`*` and `?` wildcards); `path` is the directory itself for no pattern.
    List {
        path: String,
    },
    /// Size and free space of the drive.
    Volume,
    /// Times in Unix milliseconds (0 = unchanged) and attributes (0 = unchanged).
    SetBasic {
        created: f64,
        accessed: f64,
        written: f64,
        changed: f64,
        attributes: u32,
    },
    SetSize {
        size: i64,
    },
    SetDelete {
        delete: bool,
    },
    Rename {
        to: String,
        replace: bool,
    },
    /// The server answered the drive's announcement.
    Announced {
        accepted: bool,
        result_code: u32,
    },
    /// The channel restarted: every handle is gone and pending replies are dropped.
    Reset,
}

impl DriveOp {
    fn name(&self) -> &'static str {
        match self {
            Self::Create { .. } => "create",
            Self::Close => "close",
            Self::Read { .. } => "read",
            Self::Write { .. } => "write",
            Self::Stat => "stat",
            Self::List { .. } => "list",
            Self::Volume => "volume",
            Self::SetBasic { .. } => "setBasic",
            Self::SetSize { .. } => "setSize",
            Self::SetDelete { .. } => "setDelete",
            Self::Rename { .. } => "rename",
            Self::Announced { .. } => "announced",
            Self::Reset => "reset",
        }
    }
}

/// A request for JavaScript. `id` is 0 for notifications that take no reply
/// (`announced`, `reset`).
#[derive(Debug, PartialEq)]
pub(crate) struct DriveRequest {
    pub(crate) id: u32,
    pub(crate) device_id: u32,
    pub(crate) file_id: u32,
    pub(crate) op: DriveOp,
}

impl DriveRequest {
    /// Builds `{ id, op, deviceId, fileId, ...fields }`.
    pub(crate) fn to_js(&self) -> JsValue {
        let obj = js_sys::Object::new();
        let set = |key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(&obj, &JsValue::from_str(key), &value);
        };
        set("id", self.id.into());
        set("op", self.op.name().into());
        set("deviceId", self.device_id.into());
        set("fileId", self.file_id.into());
        match &self.op {
            DriveOp::Create {
                path,
                desired_access,
                disposition,
                options,
                attributes,
                share_access,
            } => {
                set("path", path.into());
                set("desiredAccess", (*desired_access).into());
                set("disposition", (*disposition).into());
                set("options", (*options).into());
                set("attributes", (*attributes).into());
                set("shareAccess", (*share_access).into());
            }
            DriveOp::Read { offset, length } => {
                #[expect(clippy::as_conversions, clippy::cast_precision_loss, reason = "offsets fit in f64")]
                set("offset", (*offset as f64).into());
                set("length", (*length).into());
            }
            DriveOp::Write { offset, data } => {
                #[expect(clippy::as_conversions, clippy::cast_precision_loss, reason = "offsets fit in f64")]
                set("offset", (*offset as f64).into());
                set("data", js_sys::Uint8Array::from(data.as_slice()).into());
            }
            DriveOp::List { path } => set("path", path.into()),
            DriveOp::SetBasic {
                created,
                accessed,
                written,
                changed,
                attributes,
            } => {
                set("created", (*created).into());
                set("accessed", (*accessed).into());
                set("written", (*written).into());
                set("changed", (*changed).into());
                set("attributes", (*attributes).into());
            }
            DriveOp::SetSize { size } => {
                #[expect(clippy::as_conversions, clippy::cast_precision_loss, reason = "sizes fit in f64")]
                set("size", (*size as f64).into());
            }
            DriveOp::SetDelete { delete } => set("delete", (*delete).into()),
            DriveOp::Rename { to, replace } => {
                set("to", to.into());
                set("replace", (*replace).into());
            }
            DriveOp::Announced { accepted, result_code } => {
                set("accepted", (*accepted).into());
                set("resultCode", (*result_code).into());
            }
            DriveOp::Close | DriveOp::Stat | DriveOp::Volume | DriveOp::Reset => {}
        }
        obj.into()
    }
}

/// File or directory metadata from JavaScript.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DriveStat {
    pub(crate) name: String,
    pub(crate) directory: bool,
    pub(crate) size: i64,
    pub(crate) created: f64,
    pub(crate) accessed: f64,
    pub(crate) written: f64,
    pub(crate) changed: f64,
    /// FILE_ATTRIBUTE_* bits; directory and archive bits are filled in when missing.
    pub(crate) attributes: u32,
    pub(crate) delete_pending: bool,
}

impl DriveStat {
    fn attributes(&self) -> FileAttributes {
        let mut attributes = FileAttributes::from_bits_retain(self.attributes);
        if self.directory {
            attributes |= FileAttributes::FILE_ATTRIBUTE_DIRECTORY;
        } else if attributes.is_empty() {
            attributes = FileAttributes::FILE_ATTRIBUTE_ARCHIVE;
        }
        attributes
    }

    fn size(&self) -> i64 {
        if self.directory { 0 } else { self.size.max(0) }
    }
}

/// JavaScript's answer to a [`DriveRequest`].
#[derive(Debug, Default, PartialEq)]
pub(crate) struct DriveReply {
    pub(crate) id: u32,
    /// NTSTATUS of the operation.
    pub(crate) status: u32,
    /// `create`: the new handle.
    pub(crate) file_id: u32,
    /// `create`: FILE_SUPERSEDED / OPENED / CREATED / OVERWRITTEN (0–3).
    pub(crate) information: u8,
    /// `read`: the bytes read.
    pub(crate) data: Vec<u8>,
    /// `write`: bytes written.
    pub(crate) length: u32,
    /// `stat`: the file.
    pub(crate) stat: Option<DriveStat>,
    /// `list`: the matching entries.
    pub(crate) entries: Vec<DriveStat>,
    /// `volume`: total and free bytes.
    pub(crate) total_bytes: f64,
    pub(crate) free_bytes: f64,
}

impl DriveReply {
    /// Parses `{ id, status, fileId?, information?, data?, length?, stat?, entries?, total?, free? }`.
    pub(crate) fn from_js(value: JsValue) -> anyhow::Result<Self> {
        let obj = value
            .dyn_into::<js_sys::Object>()
            .map_err(|_| anyhow::anyhow!("drive reply must be an object"))?;
        let mut reply = Self {
            id: u32_field(&obj, "id")?.ok_or_else(|| anyhow::anyhow!("drive reply without `id`"))?,
            status: u32_field(&obj, "status")?.unwrap_or(0),
            file_id: u32_field(&obj, "fileId")?.unwrap_or(0),
            information: u8::try_from(u32_field(&obj, "information")?.unwrap_or(0)).unwrap_or(0),
            length: u32_field(&obj, "length")?.unwrap_or(0),
            total_bytes: f64_field(&obj, "total")?.unwrap_or(0.0),
            free_bytes: f64_field(&obj, "free")?.unwrap_or(0.0),
            ..Self::default()
        };
        let data = get(&obj, "data")?;
        if !data.is_undefined() && !data.is_null() {
            reply.data = data
                .dyn_into::<js_sys::Uint8Array>()
                .map_err(|_| anyhow::anyhow!("drive reply `data` must be a Uint8Array"))?
                .to_vec();
        }
        let stat = get(&obj, "stat")?;
        if !stat.is_undefined() && !stat.is_null() {
            reply.stat = Some(parse_stat(stat)?);
        }
        let entries = get(&obj, "entries")?;
        if !entries.is_undefined() && !entries.is_null() {
            let entries = entries
                .dyn_into::<js_sys::Array>()
                .map_err(|_| anyhow::anyhow!("drive reply `entries` must be an array"))?;
            for entry in entries.iter().take(MAX_LISTING) {
                reply.entries.push(parse_stat(entry)?);
            }
        }
        Ok(reply)
    }
}

fn get(obj: &js_sys::Object, key: &str) -> anyhow::Result<JsValue> {
    js_sys::Reflect::get(obj, &JsValue::from_str(key)).map_err(|e| anyhow::anyhow!("get `{key}`: {e:?}"))
}

fn f64_field(obj: &js_sys::Object, key: &str) -> anyhow::Result<Option<f64>> {
    let value = get(obj, key)?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let number = value
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| anyhow::anyhow!("`{key}` must be a number"))?;
    Ok(Some(number))
}

fn u32_field(obj: &js_sys::Object, key: &str) -> anyhow::Result<Option<u32>> {
    match f64_field(obj, key)? {
        None => Ok(None),
        Some(n) if (0.0..=f64::from(u32::MAX)).contains(&n) && n.fract() == 0.0 =>
        {
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "range checked"
            )]
            Ok(Some(n as u32))
        }
        Some(n) => anyhow::bail!("`{key}` is not a 32-bit unsigned integer: {n}"),
    }
}

fn parse_stat(value: JsValue) -> anyhow::Result<DriveStat> {
    let obj = value
        .dyn_into::<js_sys::Object>()
        .map_err(|_| anyhow::anyhow!("file metadata must be an object"))?;
    let size = f64_field(&obj, "size")?.unwrap_or(0.0).clamp(0.0, 9.0e15);
    Ok(DriveStat {
        name: get(&obj, "name")?.as_string().unwrap_or_default(),
        directory: get(&obj, "dir")?.as_bool().unwrap_or(false),
        #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "clamped above")]
        size: size as i64,
        created: f64_field(&obj, "created")?.unwrap_or(0.0),
        accessed: f64_field(&obj, "accessed")?.unwrap_or(0.0),
        written: f64_field(&obj, "written")?.unwrap_or(0.0),
        changed: f64_field(&obj, "changed")?.unwrap_or(0.0),
        attributes: u32_field(&obj, "attributes")?.unwrap_or(0),
        delete_pending: get(&obj, "deletePending")?.as_bool().unwrap_or(false),
    })
}

/// How to complete an IRP once JavaScript answers.
#[derive(Debug)]
enum Pending {
    /// The flag is FILE_DELETE_ON_CLOSE.
    Create(DeviceIoRequest, bool),
    Close(DeviceIoRequest),
    Read(DeviceIoRequest),
    Write(DeviceIoRequest),
    Information(DeviceIoRequest, FileInformationClassLevel),
    List(DeviceIoRequest, FileInformationClassLevel),
    Volume(DeviceIoRequest, FileSystemInformationClassLevel),
    Set(ServerDriveSetInformationRequest),
}

/// Sends drive requests to the session event loop.
#[derive(Debug, Clone)]
pub(crate) struct WasmDriveProxy {
    tx: mpsc::UnboundedSender<RdpInputEvent>,
}

impl WasmDriveProxy {
    pub(crate) fn new(tx: mpsc::UnboundedSender<RdpInputEvent>) -> Self {
        Self { tx }
    }

    fn send(&self, request: DriveRequest) -> bool {
        self.tx.unbounded_send(RdpInputEvent::DriveRequest(request)).is_ok()
    }
}

/// Filesystem side of the browser RDPDR backend; see the module docs.
#[derive(Debug)]
pub(crate) struct WasmDriveBackend {
    drives: Vec<DriveSpec>,
    proxy: WasmDriveProxy,
    /// Deferred IRPs by request ID, with their place in the completion order.
    pending: HashMap<u32, (u64, Pending)>,
    next_id: u32,
    /// Completions go out in the order the IRPs arrived; see [`Self::handle`].
    next_seq: u64,
    next_out: u64,
    ready: BTreeMap<u64, Vec<SvcMessage>>,
    /// The sequence number of the IRP being handled, and whether it was deferred.
    current_seq: u64,
    deferred: bool,
    /// Remaining entries of each directory being enumerated, by (device, file).
    listings: HashMap<(u32, u32), VecDeque<DriveStat>>,
    /// Open handles, by (device, file).
    files: HashMap<(u32, u32), OpenFile>,
}

/// What the backend knows about an open handle.
#[derive(Debug, Default)]
struct OpenFile {
    /// Metadata answering information queries without a round trip. Every
    /// change on the drive drops the metadata of all its handles.
    stat: Option<DriveStat>,
    /// Whether the handle changed anything (or deletes on close), so its
    /// close must wait for JavaScript; other closes complete at once.
    dirty: bool,
}

impl WasmDriveBackend {
    pub(crate) fn new(drives: Vec<DriveSpec>, proxy: WasmDriveProxy) -> Self {
        Self {
            drives,
            proxy,
            pending: HashMap::new(),
            next_id: 1,
            next_seq: 0,
            next_out: 0,
            ready: BTreeMap::new(),
            current_seq: 0,
            deferred: false,
            listings: HashMap::new(),
            files: HashMap::new(),
        }
    }

    pub(crate) fn drives(&self) -> &[DriveSpec] {
        &self.drives
    }

    pub(crate) fn has_drive(&self, device_id: u32) -> bool {
        self.drives.iter().any(|drive| drive.device_id == device_id)
    }

    fn drive_name(&self, device_id: u32) -> &str {
        self.drives
            .iter()
            .find(|drive| drive.device_id == device_id)
            .map_or("Drive", |drive| drive.name.as_str())
    }

    /// Drops every pending IRP and listing; JavaScript closes every handle.
    pub(crate) fn reset(&mut self) {
        self.pending.clear();
        self.ready.clear();
        self.next_out = self.next_seq;
        self.listings.clear();
        self.files.clear();
        let _ = self.proxy.send(DriveRequest {
            id: 0,
            device_id: 0,
            file_id: 0,
            op: DriveOp::Reset,
        });
    }

    pub(crate) fn announced(&mut self, device_id: u32, result_code: NtStatus) {
        if result_code == NtStatus::SUCCESS {
            debug!(device_id, "Drive announce accepted by server");
        } else {
            warn!(device_id, ?result_code, "Drive announce rejected by server");
        }
        let _ = self.proxy.send(DriveRequest {
            id: 0,
            device_id,
            file_id: 0,
            op: DriveOp::Announced {
                accepted: result_code == NtStatus::SUCCESS,
                result_code: u32::from(result_code),
            },
        });
    }

    /// Records that the handle `key` changes the drive: its close must wait,
    /// and the cached metadata of every handle on the drive is stale.
    fn touch(&mut self, key: (u32, u32)) {
        self.forget_stats(key.0);
        self.files.entry(key).or_default().dirty = true;
    }

    fn forget_stats(&mut self, device_id: u32) {
        for (_, file) in self.files.iter_mut().filter(|((device, _), _)| *device == device_id) {
            file.stat = None;
        }
    }

    /// Queues `op` for JavaScript, or fails the IRP when that is impossible.
    fn defer(&mut self, io: &DeviceIoRequest, op: DriveOp, pending: Pending) -> PduResult<Vec<SvcMessage>> {
        if self.pending.len() >= MAX_PENDING {
            warn!(completion_id = io.completion_id, "Too many pending drive requests");
            return fail(pending, NtStatus::INSUFFICIENT_RESOURCES);
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let request = DriveRequest {
            id,
            device_id: io.device_id,
            file_id: io.file_id,
            op,
        };
        if !self.proxy.send(request) {
            warn!("Drive request could not be queued, the event loop is gone");
            return fail(pending, NtStatus::UNSUCCESSFUL);
        }
        self.pending.insert(id, (self.current_seq, pending));
        self.deferred = true;
        Ok(Vec::new())
    }

    /// Handles a filesystem IRP and returns the completions that may go out now.
    ///
    /// Completions are sent in the order the IRPs arrived, even when a later
    /// one is answered first (from the metadata cache, or because JavaScript
    /// answered it sooner). The server reads the channel on one thread: a
    /// completion that overtakes an earlier one can block that thread on a
    /// lock held by the thread waiting for the earlier completion, which then
    /// never gets read, and the whole session stops until a timeout. Change
    /// notifications are never completed, so they take no place in the order.
    pub(crate) fn handle(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        if matches!(req, ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_)) {
            return Ok(Vec::new());
        }
        self.current_seq = self.next_seq;
        self.next_seq += 1;
        self.deferred = false;
        let out = self.handle_irp(req)?;
        if !self.deferred {
            self.ready.insert(self.current_seq, out);
        }
        Ok(self.flush())
    }

    /// Takes the completions that are next in order.
    fn flush(&mut self) -> Vec<SvcMessage> {
        let mut out = Vec::new();
        while let Some(messages) = self.ready.remove(&self.next_out) {
            out.extend(messages);
            self.next_out += 1;
        }
        out
    }

    fn handle_irp(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(create) => {
                let io = create.device_io_request;
                let delete_on_close = create.create_options.bits() & FILE_DELETE_ON_CLOSE != 0;
                let op = DriveOp::Create {
                    path: create.path,
                    desired_access: create.desired_access.bits(),
                    disposition: create.create_disposition.into(),
                    options: create.create_options.bits(),
                    attributes: create.file_attributes.bits(),
                    share_access: create.shared_access.bits(),
                };
                self.defer(&io.clone(), op, Pending::Create(io, delete_on_close))
            }
            ServerDriveIoRequest::DeviceCloseRequest(close) => {
                let io = close.device_io_request;
                let key = (io.device_id, io.file_id);
                self.listings.remove(&key);
                if self.files.remove(&key).is_some_and(|file| !file.dirty) {
                    // Nothing waits on this close: JavaScript handles requests in
                    // order, so the handle is gone before the next one runs.
                    let _ = self.proxy.send(DriveRequest {
                        id: 0,
                        device_id: io.device_id,
                        file_id: io.file_id,
                        op: DriveOp::Close,
                    });
                    return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
                        DeviceCloseResponse {
                            device_io_response: DeviceIoResponse::new(io, NtStatus::SUCCESS),
                        },
                    ))]);
                }
                self.defer(&io.clone(), DriveOp::Close, Pending::Close(io))
            }
            ServerDriveIoRequest::DeviceReadRequest(read) => {
                let io = read.device_io_request;
                if read.length > MAX_READ {
                    return fail(Pending::Read(io), NtStatus::INVALID_PARAMETER);
                }
                let op = DriveOp::Read {
                    offset: read.offset,
                    length: read.length,
                };
                self.defer(&io.clone(), op, Pending::Read(io))
            }
            ServerDriveIoRequest::DeviceWriteRequest(write) => {
                let io = write.device_io_request;
                self.touch((io.device_id, io.file_id));
                let op = DriveOp::Write {
                    offset: write.offset,
                    data: write.write_data,
                };
                self.defer(&io.clone(), op, Pending::Write(io))
            }
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(query) => {
                let io = query.device_io_request;
                let class = query.file_info_class_lvl;
                if !matches!(
                    class,
                    FileInformationClassLevel::FILE_BASIC_INFORMATION
                        | FileInformationClassLevel::FILE_STANDARD_INFORMATION
                        | FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION
                ) {
                    return fail(Pending::Information(io, class), NtStatus::NOT_SUPPORTED);
                }
                if let Some(stat) = self
                    .files
                    .get(&(io.device_id, io.file_id))
                    .and_then(|file| file.stat.as_ref())
                {
                    let info = file_information(class, stat);
                    return Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
                        ClientDriveQueryInformationResponse {
                            device_io_response: DeviceIoResponse::new(io, NtStatus::SUCCESS),
                            buffer: Some(info),
                        },
                    ))]);
                }
                self.defer(&io.clone(), DriveOp::Stat, Pending::Information(io, class))
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(query) => {
                let io = query.device_io_request;
                let class = query.file_info_class_lvl;
                let key = (io.device_id, io.file_id);
                if query.initial_query == 0 {
                    let entry = self.listings.get_mut(&key).and_then(VecDeque::pop_front);
                    if self.listings.get(&key).is_some_and(VecDeque::is_empty) {
                        self.listings.remove(&key);
                    }
                    return directory_response(io, class, entry.ok_or(NtStatus::NO_MORE_FILES));
                }
                // An initial query restarts the enumeration.
                self.listings.remove(&key);
                self.defer(
                    &io.clone(),
                    DriveOp::List { path: query.path },
                    Pending::List(io, class),
                )
            }
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(query) => {
                let io = query.device_io_request;
                let class = query.fs_info_class_lvl;
                match class {
                    FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION
                    | FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => {
                        self.defer(&io.clone(), DriveOp::Volume, Pending::Volume(io, class))
                    }
                    _ => {
                        let info = local_volume_information(class, self.drive_name(io.device_id), io.device_id);
                        Ok(vec![volume_response(io, info)])
                    }
                }
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(set) => {
                let op = match &set.set_buffer {
                    FileInformationClass::Basic(info) => DriveOp::SetBasic {
                        created: filetime_to_ms(info.creation_time),
                        accessed: filetime_to_ms(info.last_access_time),
                        written: filetime_to_ms(info.last_write_time),
                        changed: filetime_to_ms(info.change_time),
                        attributes: info.file_attributes.bits(),
                    },
                    FileInformationClass::EndOfFile(info) => DriveOp::SetSize { size: info.end_of_file },
                    FileInformationClass::Disposition(info) => DriveOp::SetDelete {
                        delete: info.delete_pending != 0,
                    },
                    FileInformationClass::Rename(info) => DriveOp::Rename {
                        to: info.file_name.clone(),
                        replace: info.replace_if_exists == Boolean::True,
                    },
                    // The allocation size is only a hint.
                    FileInformationClass::Allocation(_) => {
                        return set_response(&set, NtStatus::SUCCESS);
                    }
                    _ => return set_response(&set, NtStatus::NOT_SUPPORTED),
                };
                let io = set.device_io_request.clone();
                self.touch((io.device_id, io.file_id));
                self.defer(&io, op, Pending::Set(set))
            }
            // Writes reach the file directly, so there is nothing to flush.
            ServerDriveIoRequest::DeviceFlushBuffersRequest(flush) => Ok(vec![SvcMessage::from(
                RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                    device_io_response: DeviceIoResponse::new(flush.device_io_request, NtStatus::SUCCESS),
                }),
            )]),
            // Byte-range locks are granted without enforcement, like FreeRDP does.
            ServerDriveIoRequest::ServerDriveLockControlRequest(lock) => {
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
                    ClientDriveLockControlResponse::new(lock.device_io_request, NtStatus::SUCCESS),
                ))])
            }
            // Change notifications are never completed: there is no watcher.
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => Ok(Vec::new()),
            ServerDriveIoRequest::DeviceControlRequest(control) => {
                let status = fsctl_status(control.io_control_code.0);
                Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
                    DeviceControlResponse {
                        device_io_reply: DeviceIoResponse::new(control.header, status),
                        output_buffer: None,
                    },
                ))])
            }
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(query) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                    device_io_response: DeviceIoResponse::new(query.device_io_request, NtStatus::NOT_SUPPORTED),
                    security_descriptor: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(set) => {
                let response =
                    ClientDriveSetSecurityResponse::new(&set, NtStatus::NOT_SUPPORTED).map_err(|e| encode_err!(e))?;
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetSecurityResponse(
                    response,
                ))])
            }
        }
    }

    /// Completes the IRP JavaScript answered. A reply to an unknown or stale
    /// request is ignored.
    pub(crate) fn complete(&mut self, reply: DriveReply) -> PduResult<Vec<SvcMessage>> {
        let Some((seq, pending)) = self.pending.remove(&reply.id) else {
            debug!(id = reply.id, "Ignoring a drive reply without a pending request");
            return Ok(Vec::new());
        };
        let messages = self.completion(pending, reply)?;
        self.ready.insert(seq, messages);
        Ok(self.flush())
    }

    /// Builds the completion of a deferred IRP from JavaScript's reply.
    fn completion(&mut self, pending: Pending, reply: DriveReply) -> PduResult<Vec<SvcMessage>> {
        let status = NtStatus::from(reply.status);
        let ok = status == NtStatus::SUCCESS;
        let message = match pending {
            Pending::Create(io, delete_on_close) => {
                if ok {
                    // A create that made or emptied a file changes the drive.
                    if reply.information != 1 {
                        self.forget_stats(io.device_id);
                    }
                    self.files.insert(
                        (io.device_id, reply.file_id),
                        OpenFile {
                            stat: reply.stat,
                            dirty: delete_on_close,
                        },
                    );
                }
                SvcMessage::from(RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                    device_io_reply: DeviceIoResponse::new(io, status),
                    file_id: if ok { reply.file_id } else { 0 },
                    information: if ok {
                        Information::from_bits_retain(reply.information.min(3))
                    } else {
                        Information::empty()
                    },
                }))
            }
            Pending::Close(io) => SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                // A close always succeeds ([MS-FSA] 2.1.5.5 defines no failure).
                device_io_response: DeviceIoResponse::new(io, NtStatus::SUCCESS),
            })),
            Pending::Read(io) => SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(io, status),
                read_data: if ok { reply.data } else { Vec::new() },
            })),
            Pending::Write(io) => SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(io, status),
                length: if ok { reply.length } else { 0 },
            })),
            Pending::Information(io, class) => {
                let buffer = match (ok, reply.stat) {
                    (true, Some(stat)) => {
                        let info = file_information(class, &stat);
                        if let Some(file) = self.files.get_mut(&(io.device_id, io.file_id)) {
                            file.stat = Some(stat);
                        }
                        Some(info)
                    }
                    (true, None) => return fail(Pending::Information(io, class), NtStatus::UNSUCCESSFUL),
                    (false, _) => None,
                };
                SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
                    ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(io, status),
                        buffer,
                    },
                ))
            }
            Pending::List(io, class) => {
                if !ok {
                    return directory_response(io, class, Err(status));
                }
                let mut entries: VecDeque<DriveStat> = reply.entries.into();
                // No match on an initial query is STATUS_NO_SUCH_FILE ([MS-FSA] 2.1.5.6.3).
                let first = entries.pop_front().ok_or(NtStatus::NO_SUCH_FILE);
                if first.is_ok() && !entries.is_empty() {
                    self.listings.insert((io.device_id, io.file_id), entries);
                }
                return directory_response(io, class, first);
            }
            Pending::Volume(io, class) => {
                if !ok {
                    return fail(Pending::Volume(io, class), status);
                }
                let info = size_information(class, reply.total_bytes, reply.free_bytes);
                volume_response(io, info)
            }
            Pending::Set(set) => return set_response(&set, status),
        };
        Ok(vec![message])
    }
}

/// FSCTL_IS_VOLUME_MOUNTED.
const FSCTL_IS_VOLUME_MOUNTED: u32 = 0x0009_0028;
/// FSCTL_GET_REPARSE_POINT.
const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
/// STATUS_NOT_A_REPARSE_POINT.
const STATUS_NOT_A_REPARSE_POINT: u32 = 0xC000_0275;

/// Answers a filesystem control request without output, as a file system
/// without reparse points or object IDs does. A success without the output
/// the caller asked for would claim, for example, that every file is a link:
/// Explorer asks FSCTL_GET_REPARSE_POINT for each file and then tries to
/// resolve it, and link tracking waits on FSCTL_CREATE_OR_GET_OBJECT_ID.
fn fsctl_status(code: u32) -> NtStatus {
    match code {
        FSCTL_IS_VOLUME_MOUNTED => NtStatus::SUCCESS,
        FSCTL_GET_REPARSE_POINT => NtStatus::from(STATUS_NOT_A_REPARSE_POINT),
        _ => NtStatus::INVALID_DEVICE_REQUEST,
    }
}

fn file_information(class: FileInformationClassLevel, stat: &DriveStat) -> FileInformationClass {
    match class {
        FileInformationClassLevel::FILE_STANDARD_INFORMATION => {
            FileInformationClass::Standard(FileStandardInformation {
                allocation_size: allocation(stat.size()),
                end_of_file: stat.size(),
                number_of_links: 1,
                delete_pending: if stat.delete_pending {
                    Boolean::True
                } else {
                    Boolean::False
                },
                directory: if stat.directory { Boolean::True } else { Boolean::False },
            })
        }
        FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => {
            FileInformationClass::AttributeTag(FileAttributeTagInformation {
                file_attributes: stat.attributes(),
                reparse_tag: 0,
            })
        }
        _ => FileInformationClass::Basic(FileBasicInformation {
            creation_time: ms_to_filetime(stat.created),
            last_access_time: ms_to_filetime(stat.accessed),
            last_write_time: ms_to_filetime(stat.written),
            change_time: ms_to_filetime(stat.changed),
            file_attributes: stat.attributes(),
        }),
    }
}

/// Rounds a size up to the reported allocation unit.
fn allocation(size: i64) -> i64 {
    let unit = i64::from(BYTES_PER_SECTOR * SECTORS_PER_UNIT);
    size.saturating_add(unit - 1) / unit * unit
}

fn directory_response(
    io: DeviceIoRequest,
    class: FileInformationClassLevel,
    entry: Result<DriveStat, NtStatus>,
) -> PduResult<Vec<SvcMessage>> {
    let (status, buffer) = match entry {
        Err(status) => (status, None),
        Ok(stat) => {
            let created = ms_to_filetime(stat.created);
            let accessed = ms_to_filetime(stat.accessed);
            let written = ms_to_filetime(stat.written);
            let changed = ms_to_filetime(stat.changed);
            let size = stat.size();
            let attributes = stat.attributes();
            let name = stat.name;
            let info = match class {
                FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => {
                    let mut info =
                        FileBothDirectoryInformation::new(created, accessed, written, changed, size, attributes, name);
                    info.allocation_size = allocation(size);
                    Some(FileInformationClass::BothDirectory(info))
                }
                FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => {
                    let mut info =
                        FileFullDirectoryInformation::new(created, accessed, written, changed, size, attributes, name);
                    info.allocation_size = allocation(size);
                    Some(FileInformationClass::FullDirectory(info))
                }
                FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => {
                    let mut info =
                        FileDirectoryInformation::new(created, accessed, written, changed, size, attributes, name);
                    info.allocation_size = allocation(size);
                    Some(FileInformationClass::Directory(info))
                }
                FileInformationClassLevel::FILE_NAMES_INFORMATION => {
                    Some(FileInformationClass::Names(FileNamesInformation::new(name)))
                }
                _ => None,
            };
            match info {
                Some(info) => (NtStatus::SUCCESS, Some(info)),
                None => (NtStatus::NOT_SUPPORTED, None),
            }
        }
    };
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
        ClientDriveQueryDirectoryResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            buffer,
        },
    ))])
}

/// Volume information that needs no local filesystem.
fn local_volume_information(
    class: FileSystemInformationClassLevel,
    label: &str,
    device_id: u32,
) -> Option<FileSystemInformationClass> {
    match class {
        FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => Some(
            FileSystemInformationClass::FileFsVolumeInformation(FileFsVolumeInformation {
                volume_creation_time: 0,
                volume_serial_number: 0x7E4D_0000 | (device_id & 0xFFFF),
                supports_objects: Boolean::False,
                volume_label: label.chars().take(32).collect(),
            }),
        ),
        // NTFS, so Windows does not refuse files over 4 GiB as it does for FAT32.
        FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION => Some(
            FileSystemInformationClass::FileFsAttributeInformation(FileFsAttributeInformation {
                file_system_attributes: FileSystemAttributes::FILE_CASE_PRESERVED_NAMES
                    | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                max_component_name_len: 255,
                file_system_name: "NTFS".to_owned(),
            }),
        ),
        FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION => Some(
            FileSystemInformationClass::FileFsDeviceInformation(FileFsDeviceInformation {
                device_type: FILE_DEVICE_DISK,
                characteristics: Characteristics::empty(),
            }),
        ),
        _ => None,
    }
}

fn size_information(
    class: FileSystemInformationClassLevel,
    total: f64,
    free: f64,
) -> Option<FileSystemInformationClass> {
    let unit = f64::from(BYTES_PER_SECTOR * SECTORS_PER_UNIT);
    #[expect(clippy::as_conversions, clippy::cast_possible_truncation, reason = "clamped to i64")]
    let units = |bytes: f64| (bytes.max(0.0) / unit).min(9.0e15) as i64;
    let total_units = units(total);
    let free_units = units(free).min(total_units);
    match class {
        FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => Some(
            FileSystemInformationClass::FileFsSizeInformation(FileFsSizeInformation {
                total_alloc_units: total_units,
                available_alloc_units: free_units,
                sectors_per_alloc_unit: SECTORS_PER_UNIT,
                bytes_per_sector: BYTES_PER_SECTOR,
            }),
        ),
        FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => Some(
            FileSystemInformationClass::FileFsFullSizeInformation(FileFsFullSizeInformation {
                total_alloc_units: total_units,
                caller_available_alloc_units: free_units,
                actual_available_alloc_units: free_units,
                sectors_per_alloc_unit: SECTORS_PER_UNIT,
                bytes_per_sector: BYTES_PER_SECTOR,
            }),
        ),
        _ => None,
    }
}

fn volume_response(io: DeviceIoRequest, info: Option<FileSystemInformationClass>) -> SvcMessage {
    let status = if info.is_some() {
        NtStatus::SUCCESS
    } else {
        NtStatus::NOT_SUPPORTED
    };
    SvcMessage::from(RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
        ClientDriveQueryVolumeInformationResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            buffer: info,
        },
    ))
}

fn set_response(set: &ServerDriveSetInformationRequest, status: NtStatus) -> PduResult<Vec<SvcMessage>> {
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        ClientDriveSetInformationResponse::new(set, status).map_err(|e| encode_err!(e))?,
    ))])
}

/// Completes a deferred IRP with an error and no data.
fn fail(pending: Pending, status: NtStatus) -> PduResult<Vec<SvcMessage>> {
    let message = match pending {
        Pending::Create(io, _) => SvcMessage::from(RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            file_id: 0,
            information: Information::empty(),
        })),
        Pending::Close(io) => SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
            device_io_response: DeviceIoResponse::new(io, status),
        })),
        Pending::Read(io) => SvcMessage::from(RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            read_data: Vec::new(),
        })),
        Pending::Write(io) => SvcMessage::from(RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            length: 0,
        })),
        Pending::Information(io, _) => SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
            ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(io, status),
                buffer: None,
            },
        )),
        Pending::List(io, class) => return directory_response(io, class, Err(status)),
        Pending::Volume(io, _) => volume_response_with_status(io, status),
        Pending::Set(set) => return set_response(&set, status),
    };
    Ok(vec![message])
}

fn volume_response_with_status(io: DeviceIoRequest, status: NtStatus) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
        ClientDriveQueryVolumeInformationResponse {
            device_io_reply: DeviceIoResponse::new(io, status),
            buffer: None,
        },
    ))
}

#[cfg(test)]
mod tests {
    use ironrdp::rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCreateRequest, DeviceReadRequest,
        MajorFunction, MinorFunction, ServerDriveQueryDirectoryRequest, SharedAccess,
    };

    use super::*;

    fn backend() -> (WasmDriveBackend, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (tx, rx) = mpsc::unbounded();
        let drives = vec![DriveSpec {
            device_id: 100,
            name: "Work".to_owned(),
        }];
        (WasmDriveBackend::new(drives, WasmDriveProxy::new(tx)), rx)
    }

    fn io(file_id: u32, completion_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 100,
            file_id,
            completion_id,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    fn next_request(rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>) -> DriveRequest {
        match rx.try_recv() {
            Ok(RdpInputEvent::DriveRequest(request)) => request,
            other => panic!("expected a drive request, got {other:?}"),
        }
    }

    #[test]
    fn create_is_deferred_and_completed_with_the_reply() {
        let (mut backend, mut rx) = backend();
        let out = backend
            .handle(ServerDriveIoRequest::ServerCreateDriveRequest(DeviceCreateRequest {
                device_io_request: io(0, 7, MajorFunction::Create),
                desired_access: DesiredAccess::GENERIC_READ,
                allocation_size: 0,
                file_attributes: FileAttributes::empty(),
                shared_access: SharedAccess::FILE_SHARE_READ,
                create_disposition: CreateDisposition::FILE_OPEN,
                create_options: CreateOptions::empty(),
                path: "\\docs\\a.txt".to_owned(),
            }))
            .unwrap();
        assert!(out.is_empty());
        let request = next_request(&mut rx);
        assert_eq!(request.device_id, 100);
        assert!(matches!(&request.op, DriveOp::Create { path, disposition: 1, .. } if path == "\\docs\\a.txt"));

        let out = backend
            .complete(DriveReply {
                id: request.id,
                file_id: 5,
                information: 1,
                ..DriveReply::default()
            })
            .unwrap();
        assert_eq!(out.len(), 1);
        // A second reply to the same request is ignored.
        assert!(
            backend
                .complete(DriveReply {
                    id: request.id,
                    ..DriveReply::default()
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn reset_drops_pending_requests() {
        let (mut backend, mut rx) = backend();
        backend
            .handle(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
                device_io_request: io(5, 8, MajorFunction::Read),
                length: 10,
                offset: 0,
            }))
            .unwrap();
        let request = next_request(&mut rx);
        backend.reset();
        assert_eq!(next_request(&mut rx).op, DriveOp::Reset);
        assert!(
            backend
                .complete(DriveReply {
                    id: request.id,
                    data: vec![1, 2, 3],
                    ..DriveReply::default()
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn oversized_reads_fail_without_a_round_trip() {
        let (mut backend, mut rx) = backend();
        let out = backend
            .handle(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
                device_io_request: io(5, 8, MajorFunction::Read),
                length: MAX_READ + 1,
                offset: 0,
            }))
            .unwrap();
        assert_eq!(out.len(), 1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_listing_is_fetched_once_and_served_from_memory() {
        let (mut backend, mut rx) = backend();
        let query = |file_id, completion_id, initial_query| {
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(ServerDriveQueryDirectoryRequest {
                device_io_request: io(file_id, completion_id, MajorFunction::DirectoryControl),
                file_info_class_lvl: FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
                initial_query,
                path: "\\docs\\*".to_owned(),
            })
        };
        assert!(backend.handle(query(3, 1, 1)).unwrap().is_empty());
        let request = next_request(&mut rx);
        assert_eq!(
            request.op,
            DriveOp::List {
                path: "\\docs\\*".to_owned()
            }
        );
        let entry = |name: &str| DriveStat {
            name: name.to_owned(),
            ..DriveStat::default()
        };
        let out = backend
            .complete(DriveReply {
                id: request.id,
                entries: vec![entry("."), entry(".."), entry("a.txt")],
                ..DriveReply::default()
            })
            .unwrap();
        assert_eq!(out.len(), 1);
        // The next two entries come without a request, then the listing ends.
        for completion_id in 2..=4 {
            assert_eq!(backend.handle(query(3, completion_id, 0)).unwrap().len(), 1);
        }
        assert!(rx.try_recv().is_err());
        assert!(backend.listings.is_empty());

        // Closing the handle forgets a listing that was not finished.
        backend.handle(query(3, 5, 1)).unwrap();
        let request = next_request(&mut rx);
        backend
            .complete(DriveReply {
                id: request.id,
                entries: vec![entry("."), entry("..")],
                ..DriveReply::default()
            })
            .unwrap();
        assert_eq!(backend.listings.len(), 1);
        backend
            .handle(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: io(3, 6, MajorFunction::Close),
            }))
            .unwrap();
        assert!(backend.listings.is_empty());
    }

    fn open(backend: &mut WasmDriveBackend, rx: &mut mpsc::UnboundedReceiver<RdpInputEvent>, completion_id: u32) {
        backend
            .handle(ServerDriveIoRequest::ServerCreateDriveRequest(DeviceCreateRequest {
                device_io_request: io(0, completion_id, MajorFunction::Create),
                desired_access: DesiredAccess::GENERIC_READ,
                allocation_size: 0,
                file_attributes: FileAttributes::empty(),
                shared_access: SharedAccess::FILE_SHARE_READ,
                create_disposition: CreateDisposition::FILE_OPEN,
                create_options: CreateOptions::empty(),
                path: "\\a.txt".to_owned(),
            }))
            .unwrap();
        let request = next_request(rx);
        backend
            .complete(DriveReply {
                id: request.id,
                file_id: 5,
                information: 1,
                stat: Some(DriveStat {
                    size: 3,
                    ..DriveStat::default()
                }),
                ..DriveReply::default()
            })
            .unwrap();
    }

    fn query_information(backend: &mut WasmDriveBackend, completion_id: u32) -> usize {
        backend
            .handle(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
                ironrdp::rdpdr::pdu::efs::ServerDriveQueryInformationRequest {
                    device_io_request: io(5, completion_id, MajorFunction::QueryInformation),
                    file_info_class_lvl: FileInformationClassLevel::FILE_STANDARD_INFORMATION,
                },
            ))
            .unwrap()
            .len()
    }

    fn close(backend: &mut WasmDriveBackend, completion_id: u32) -> usize {
        backend
            .handle(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: io(5, completion_id, MajorFunction::Close),
            }))
            .unwrap()
            .len()
    }

    #[test]
    fn a_handle_that_changed_nothing_is_answered_locally() {
        let (mut backend, mut rx) = backend();
        open(&mut backend, &mut rx, 1);
        // The metadata came with the create and the close needs no reply.
        assert_eq!(query_information(&mut backend, 2), 1);
        assert!(rx.try_recv().is_err());
        assert_eq!(close(&mut backend, 3), 1);
        let request = next_request(&mut rx);
        assert_eq!((request.id, request.op), (0, DriveOp::Close));
    }

    #[test]
    fn a_write_makes_metadata_and_close_go_to_javascript() {
        let (mut backend, mut rx) = backend();
        open(&mut backend, &mut rx, 1);
        backend
            .handle(ServerDriveIoRequest::DeviceWriteRequest(
                ironrdp::rdpdr::pdu::efs::DeviceWriteRequest {
                    device_io_request: io(5, 2, MajorFunction::Write),
                    offset: 0,
                    write_data: vec![1],
                },
            ))
            .unwrap();
        next_request(&mut rx);
        assert_eq!(query_information(&mut backend, 3), 0);
        assert_eq!(next_request(&mut rx).op, DriveOp::Stat);
        assert_eq!(close(&mut backend, 4), 0);
        let request = next_request(&mut rx);
        assert!(request.id != 0 && request.op == DriveOp::Close);
    }

    #[test]
    fn completions_go_out_in_the_order_the_irps_arrived() {
        let (mut backend, mut rx) = backend();
        open(&mut backend, &mut rx, 1);
        // A read goes to JavaScript; a later metadata query is answered from the cache.
        let read = backend
            .handle(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
                device_io_request: io(5, 2, MajorFunction::Read),
                length: 3,
                offset: 0,
            }))
            .unwrap();
        assert!(read.is_empty());
        let request = next_request(&mut rx);
        // The cached answer waits for the read.
        assert_eq!(query_information(&mut backend, 3), 0);
        // A change notification is never answered and holds nothing up.
        backend
            .handle(ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(
                ironrdp::rdpdr::pdu::efs::ServerDriveNotifyChangeDirectoryRequest {
                    device_io_request: io(5, 4, MajorFunction::DirectoryControl),
                    watch_tree: 0,
                    completion_filter: 1,
                },
            ))
            .unwrap();
        let out = backend
            .complete(DriveReply {
                id: request.id,
                data: vec![1, 2, 3],
                ..DriveReply::default()
            })
            .unwrap();
        assert_eq!(out.len(), 2, "the read and then the query");
        assert_eq!(query_information(&mut backend, 5), 1);
    }

    #[test]
    fn filesystem_controls_report_no_reparse_points() {
        assert_eq!(
            fsctl_status(FSCTL_GET_REPARSE_POINT),
            NtStatus::from(STATUS_NOT_A_REPARSE_POINT)
        );
        assert_eq!(fsctl_status(FSCTL_IS_VOLUME_MOUNTED), NtStatus::SUCCESS);
        // FSCTL_CREATE_OR_GET_OBJECT_ID
        assert_eq!(fsctl_status(0x0009_00C0), NtStatus::INVALID_DEVICE_REQUEST);
    }

    #[test]
    fn times_round_trip_through_milliseconds() {
        let ms = 1_700_000_000_123.0;
        assert_eq!(filetime_to_ms(ms_to_filetime(ms)), ms);
        assert_eq!(filetime_to_ms(0), 0.0);
        assert_eq!(filetime_to_ms(-1), 0.0);
        assert_eq!(ms_to_filetime(0.0), 0);
    }

    #[test]
    fn sizes_round_up_to_the_allocation_unit() {
        assert_eq!(allocation(0), 0);
        assert_eq!(allocation(1), 4096);
        assert_eq!(allocation(4096), 4096);
    }
}

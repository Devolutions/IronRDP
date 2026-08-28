//! Server-side [\[MS-RDPEFS\]] orchestration: the half of RDPDR that initiates the
//! handshake, tracks outstanding I/O requests by `CompletionId`, and routes client
//! completions back to an application-supplied [`RdpdrServerBackend`].
//!
//! This module only builds and interprets PDUs; it knows nothing about where a
//! `drive_read` completion's bytes end up or which local file a `drive_create`
//! corresponds to. That is the backend's job, matching the layering already used
//! by `crate::backend` on the client side and by `ironrdp-rdpeusb`'s
//! `UrbdrcDeviceServerBackend` on the server side of a different channel.
//!
//! [\[MS-RDPEFS\]]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

use core::fmt;
use std::collections::HashMap;

use ironrdp_core::{AsAny, EncodeResult, ReadCursor, WriteCursor, impl_as_any};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcMessage, SvcProcessor, SvcServerProcessor};
use tracing::{trace, warn};

use crate::pdu::efs::{
    AnyIoCtlCode, CapabilityMessage, ClientDeviceListAnnounce, ClientDeviceListRemove, ClientDriveLockControlResponse,
    ClientDriveNotifyChangeDirectoryResponse, ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse,
    ClientDriveQuerySecurityResponse, ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse,
    ClientDriveSetSecurityResponse, ClientNameRequest, CoreCapability, CoreCapabilityKind, CreateDisposition,
    CreateOptions, DesiredAccess, DeviceAnnounceHeader, DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest,
    DeviceControlResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceFlushBuffersRequest,
    DeviceFlushBuffersResponse, DeviceIoRequest, DeviceIoResponse, DeviceReadRequest, DeviceReadResponse,
    DeviceWriteRequest, DeviceWriteResponse, FileAttributes, FileInformationClass, FileInformationClassLevel,
    FileSystemInformationClassLevel, LockOperation, MajorFunction, MinorFunction, NtStatus, RdpLockInfo,
    SecurityInformation, ServerDeviceAnnounceResponse, ServerDriveLockControlRequest,
    ServerDriveNotifyChangeDirectoryRequest, ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest,
    ServerDriveQuerySecurityRequest, ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest,
    ServerDriveSetSecurityRequest, SharedAccess, VERSION_MAJOR, VERSION_MINOR_13, VersionAndIdPdu, VersionAndIdPduKind,
};
use crate::pdu::{Component, PacketId, RdpdrPdu, SharedHeader};

/// The wire size of [`SharedHeader`] (`Component` + `PacketId`, both `u16`).
///
/// `SharedHeader::encode` itself is private to `crate::pdu`, so a hand-built
/// `PAKID_CORE_DEVICE_IOREQUEST` (see [`RdpdrServer::send_device_io_request`]) writes
/// this prefix directly with the public [`Component`]/[`PacketId`] `Into<u16>` impls
/// instead of going through it.
const SHARED_HEADER_SIZE: usize = 2 // Component
    + 2; // PacketId

/// A drive I/O request awaiting its `DeviceIoCompletion`.
///
/// [`RdpdrPdu::decode_io_completion`] cannot recover the `MajorFunction` (or, for
/// directory control, the `MinorFunction`) a completion answers from the wire: every
/// completion response shares one `PacketId` (see `pdu::RdpdrPdu::header`), and three
/// of them additionally need the `FileInformationClassLevel` or
/// `FileSystemInformationClassLevel` the original request asked for. `PendingIrp`
/// carries exactly that context, keyed by `CompletionId` in [`RdpdrServer`]'s tracker.
///
/// One variant per `MajorFunction` this server can send, except `SetVolumeInformation`
/// (deliberately unsupported end to end, matching `ServerDriveIoRequest::decode` and
/// `RdpdrPdu::decode_io_completion`). `DirectoryControl` splits into `QueryDirectory`
/// and `NotifyChangeDirectory`, since those are the two `MinorFunction`s it carries and
/// each completes with a different response type.
#[derive(Debug, Clone)]
pub enum PendingIrp {
    Create,
    Close,
    Read,
    Write,
    FlushBuffers,
    DeviceControl,
    QueryInformation {
        info_class: FileInformationClassLevel,
    },
    SetInformation,
    QueryDirectory {
        info_class: FileInformationClassLevel,
    },
    NotifyChangeDirectory,
    QueryVolumeInformation {
        volume_info_class: FileSystemInformationClassLevel,
    },
    LockControl,
    QuerySecurity,
    SetSecurity,
}

impl PendingIrp {
    /// The `(MajorFunction, MinorFunction, info_class, volume_info_class)` tuple
    /// [`RdpdrPdu::decode_io_completion`] needs to parse this IRP's completion.
    fn completion_classifiers(
        &self,
    ) -> (
        MajorFunction,
        MinorFunction,
        Option<FileInformationClassLevel>,
        Option<FileSystemInformationClassLevel>,
    ) {
        let no_minor = MinorFunction::from(0);
        match self {
            PendingIrp::Create => (MajorFunction::Create, no_minor, None, None),
            PendingIrp::Close => (MajorFunction::Close, no_minor, None, None),
            PendingIrp::Read => (MajorFunction::Read, no_minor, None, None),
            PendingIrp::Write => (MajorFunction::Write, no_minor, None, None),
            PendingIrp::FlushBuffers => (MajorFunction::FlushBuffers, no_minor, None, None),
            PendingIrp::DeviceControl => (MajorFunction::DeviceControl, no_minor, None, None),
            PendingIrp::QueryInformation { info_class } => (
                MajorFunction::QueryInformation,
                no_minor,
                Some(info_class.clone()),
                None,
            ),
            PendingIrp::SetInformation => (MajorFunction::SetInformation, no_minor, None, None),
            PendingIrp::QueryDirectory { info_class } => (
                MajorFunction::DirectoryControl,
                MinorFunction::IRP_MN_QUERY_DIRECTORY,
                Some(info_class.clone()),
                None,
            ),
            PendingIrp::NotifyChangeDirectory => (
                MajorFunction::DirectoryControl,
                MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
                None,
                None,
            ),
            PendingIrp::QueryVolumeInformation { volume_info_class } => (
                MajorFunction::QueryVolumeInformation,
                no_minor,
                None,
                Some(volume_info_class.clone()),
            ),
            PendingIrp::LockControl => (MajorFunction::LockControl, no_minor, None, None),
            PendingIrp::QuerySecurity => (MajorFunction::QuerySecurity, no_minor, None, None),
            PendingIrp::SetSecurity => (MajorFunction::SetSecurity, no_minor, None, None),
        }
    }
}

/// Tracks drive I/O requests this server has sent to the client but not yet completed.
///
/// Mirrors `ironrdp-rdpeusb`'s `RequestIdAllocator` + `pending_io: BTreeMap<RequestId,
/// Pending>` (the closest merged precedent for a server-side virtual channel that
/// issues async requests and matches completions by ID), adapted to RDPDR's
/// full-width `CompletionId` (`u32`, no reserved high bit).
#[derive(Debug, Default)]
struct IrpTracker {
    next_completion_id: u32,
    pending: HashMap<u32, PendingIrp>,
}

impl IrpTracker {
    fn new() -> Self {
        Self::default()
    }

    /// Allocates a `CompletionId` and registers `irp` under it.
    fn register(&mut self, irp: PendingIrp) -> u32 {
        loop {
            let id = self.next_completion_id;
            self.next_completion_id = self.next_completion_id.wrapping_add(1);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.pending.entry(id) {
                entry.insert(irp);
                return id;
            }
            // `CompletionId` space (2^32) is only exhaustible by ~4 billion simultaneously
            // outstanding requests; this loop is defensive, not expected to iterate.
        }
    }

    /// Looks up and removes the pending IRP for `completion_id`, if any.
    ///
    /// `None` means the completion doesn't match anything this server sent: an
    /// unknown, already-completed, or (per MS-RDPEFS leniency elsewhere in this
    /// crate) simply misbehaving client. The caller warns and ignores rather than
    /// treating it as a protocol error.
    fn complete(&mut self, completion_id: u32) -> Option<PendingIrp> {
        self.pending.remove(&completion_id)
    }

    fn clear(&mut self) {
        self.pending.clear();
    }
}

/// Application-level callbacks for server-side RDPDR.
///
/// The processor (`RdpdrServer`) owns protocol state and PDU framing; the backend
/// owns everything else, matching the three-layer split already used for
/// `ironrdp-rdpeusb`'s server side (`UrbdrcDeviceServerBackend`) and for this crate's
/// own client-side `RdpdrBackend`. One callback per `MajorFunction` this server can
/// complete (all fourteen except the unsupported `SetVolumeInformation`), plus device
/// lifecycle and client-name notifications.
///
/// Completion callbacks return `PduResult<()>` (matching
/// `UrbdrcDeviceServerBackend`'s completion callbacks) so a backend can fail the
/// channel on a response it cannot make sense of, rather than being forced to swallow
/// the error or panic.
pub trait RdpdrServerBackend: AsAny + fmt::Debug + Send {
    /// The client announced `devices` (initial announce or a later hot-plug).
    ///
    /// Returns `(device_id, accepted)` for every device in `devices`. An accepted
    /// device is registered and receives `STATUS_SUCCESS` in its
    /// `ServerDeviceAnnounceResponse`; anything else (including a `device_id` this
    /// returns nothing for) is rejected with `STATUS_ACCESS_DENIED` and never
    /// registered.
    fn on_device_announce(&mut self, devices: &[DeviceAnnounceHeader]) -> Vec<(u32, bool)>;

    /// The client removed (hot-unplugged) the given devices.
    fn on_device_remove(&mut self, device_ids: &[u32]);

    /// The client's computer name arrived during the initialization handshake.
    fn on_client_name(&mut self, computer_name: &str);

    fn on_create_complete(&mut self, response: &DeviceCreateResponse) -> PduResult<()>;
    fn on_close_complete(&mut self, response: &DeviceCloseResponse) -> PduResult<()>;
    fn on_read_complete(&mut self, response: &DeviceReadResponse) -> PduResult<()>;
    fn on_write_complete(&mut self, response: &DeviceWriteResponse) -> PduResult<()>;
    fn on_flush_buffers_complete(&mut self, response: &DeviceFlushBuffersResponse) -> PduResult<()>;
    fn on_device_control_complete(&mut self, response: &DeviceControlResponse) -> PduResult<()>;
    fn on_query_information_complete(&mut self, response: &ClientDriveQueryInformationResponse) -> PduResult<()>;
    fn on_set_information_complete(&mut self, response: &ClientDriveSetInformationResponse) -> PduResult<()>;
    fn on_query_directory_complete(&mut self, response: &ClientDriveQueryDirectoryResponse) -> PduResult<()>;
    fn on_notify_change_directory_complete(
        &mut self,
        response: &ClientDriveNotifyChangeDirectoryResponse,
    ) -> PduResult<()>;
    fn on_query_volume_information_complete(
        &mut self,
        response: &ClientDriveQueryVolumeInformationResponse,
    ) -> PduResult<()>;
    fn on_lock_control_complete(&mut self, response: &ClientDriveLockControlResponse) -> PduResult<()>;
    fn on_query_security_complete(&mut self, response: &ClientDriveQuerySecurityResponse) -> PduResult<()>;
    fn on_set_security_complete(&mut self, response: &ClientDriveSetSecurityResponse) -> PduResult<()>;

    /// Notifies the backend that the RDPDR channel is closing.
    ///
    /// Defaulted to a no-op: this module has no call site for it yet. `SvcProcessor`
    /// (unlike `DvcProcessor`, which the client's dynamic-channel siblings implement)
    /// declares no `close` method, so a static virtual channel has no notification
    /// when it goes away on its own. Calling this is deferred to the `ironrdp-server`
    /// integration that wires `RdpdrServer` in, which does have connection-lifecycle
    /// hooks.
    fn close(&mut self) {}
}

/// Trivial [`RdpdrServerBackend`] that accepts every device and ignores every
/// completion. Used for protocol-level testing of [`RdpdrServer`] itself, matching
/// `ironrdp-rdpeusb`'s and this crate's own `NoopRdpdrBackend` convention.
#[derive(Debug, Default)]
pub struct NoopRdpdrServerBackend;

impl_as_any!(NoopRdpdrServerBackend);

impl RdpdrServerBackend for NoopRdpdrServerBackend {
    fn on_device_announce(&mut self, devices: &[DeviceAnnounceHeader]) -> Vec<(u32, bool)> {
        devices.iter().map(|device| (device.device_id(), true)).collect()
    }

    fn on_device_remove(&mut self, _device_ids: &[u32]) {}
    fn on_client_name(&mut self, _computer_name: &str) {}

    fn on_create_complete(&mut self, _response: &DeviceCreateResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_close_complete(&mut self, _response: &DeviceCloseResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_read_complete(&mut self, _response: &DeviceReadResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_write_complete(&mut self, _response: &DeviceWriteResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_flush_buffers_complete(&mut self, _response: &DeviceFlushBuffersResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_device_control_complete(&mut self, _response: &DeviceControlResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_query_information_complete(&mut self, _response: &ClientDriveQueryInformationResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_set_information_complete(&mut self, _response: &ClientDriveSetInformationResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_query_directory_complete(&mut self, _response: &ClientDriveQueryDirectoryResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_notify_change_directory_complete(
        &mut self,
        _response: &ClientDriveNotifyChangeDirectoryResponse,
    ) -> PduResult<()> {
        Ok(())
    }
    fn on_query_volume_information_complete(
        &mut self,
        _response: &ClientDriveQueryVolumeInformationResponse,
    ) -> PduResult<()> {
        Ok(())
    }
    fn on_lock_control_complete(&mut self, _response: &ClientDriveLockControlResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_query_security_complete(&mut self, _response: &ClientDriveQuerySecurityResponse) -> PduResult<()> {
        Ok(())
    }
    fn on_set_security_complete(&mut self, _response: &ClientDriveSetSecurityResponse) -> PduResult<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RdpdrServerState {
    /// `start()` not yet called.
    Initializing,
    /// `ServerAnnounceRequest` sent; awaiting `ClientAnnounceReply` and `ClientNameRequest`.
    AwaitingAnnounce,
    /// `ServerCoreCapabilityRequest` and `ServerClientIdConfirm` sent; awaiting
    /// `ClientCoreCapabilityResponse`. The client's first `ClientDeviceListAnnounce`
    /// may also arrive in this state; real clients don't reliably order it after the
    /// capability response.
    CapabilityExchange,
    /// Capability exchange complete. Devices can be announced/removed and drive I/O
    /// requests can be sent.
    Active,
}

/// Message sent by the event loop to trigger server-initiated drive I/O.
///
/// One variant per [`RdpdrServer`] `drive_*` method (`Create` for `drive_create`,
/// `Read` for `drive_read`, and so on); each variant's fields match that method's
/// parameters exactly, `path: impl Into<String>` parameters included as plain
/// `String` since an enum field can't be generic. Mirrors
/// `RdpsndServerMessage`'s convention of dropping the channel-name prefix from
/// variant names, since the enum itself is already `RdpdrServerMessage`.
#[derive(Debug, Clone)]
pub enum RdpdrServerMessage {
    Create {
        device_id: u32,
        path: String,
        desired_access: DesiredAccess,
        create_disposition: CreateDisposition,
        create_options: CreateOptions,
    },
    Read {
        device_id: u32,
        file_id: u32,
        length: u32,
        offset: u64,
    },
    Write {
        device_id: u32,
        file_id: u32,
        data: Vec<u8>,
        offset: u64,
    },
    Close {
        device_id: u32,
        file_id: u32,
    },
    FlushBuffers {
        device_id: u32,
        file_id: u32,
    },
    QueryInformation {
        device_id: u32,
        file_id: u32,
        info_class: FileInformationClassLevel,
    },
    SetInformation {
        device_id: u32,
        file_id: u32,
        set_buffer: FileInformationClass,
    },
    QueryDirectory {
        device_id: u32,
        file_id: u32,
        info_class: FileInformationClassLevel,
        path: String,
        initial_query: bool,
    },
    NotifyChangeDirectory {
        device_id: u32,
        file_id: u32,
        watch_tree: bool,
        completion_filter: u32,
    },
    QueryVolumeInformation {
        device_id: u32,
        file_id: u32,
        fs_info_class: FileSystemInformationClassLevel,
    },
    LockControl {
        device_id: u32,
        file_id: u32,
        operation: LockOperation,
        wait: bool,
        locks: Vec<RdpLockInfo>,
    },
    QuerySecurity {
        device_id: u32,
        file_id: u32,
        security_information: SecurityInformation,
    },
    SetSecurity {
        device_id: u32,
        file_id: u32,
        security_information: SecurityInformation,
        security_descriptor: Vec<u8>,
    },
    DeviceControl {
        device_id: u32,
        file_id: u32,
        io_control_code: u32,
        input_buffer: Vec<u8>,
        output_buffer_length: u32,
    },
}

/// Server-side [\[MS-RDPEFS\]] channel handler.
///
/// Initiates the RDPDR handshake, tracks client-announced devices, and sends drive
/// I/O requests via the `drive_*` methods, matching completions back to their
/// original request and dispatching to a [`RdpdrServerBackend`].
///
/// Deliberately not a `Rdpdr<Role>` generic over the existing client-side [`crate::Rdpdr`]:
/// server and client roles diverge in who initiates, who owns the device list, and what
/// "processing a payload" even means (send a request and wait, versus answer one), which
/// would push most of this type's methods behind `if Role::IS_SERVER` branching instead.
#[derive(Debug)]
pub struct RdpdrServer {
    state: RdpdrServerState,
    /// Generated once in `start()` and reused as the confirmed ID in
    /// `ServerClientIdConfirm`. MS-RDPEFS lets the client mint its own legacy ID only
    /// when the server's `VersionMinor` predates `VERSION_MINOR_12`; this server always
    /// announces [`VERSION_MINOR_13`], so a compliant client is expected to echo this
    /// value back rather than replace it.
    client_id: u32,
    /// Client-announced devices, keyed by `DeviceId`. Reuses [`DeviceAnnounceHeader`]
    /// as the registry entry type rather than duplicating its fields into a separate
    /// `DeviceEntry`: the header already exposes `device_id()`, `device_type()`, and
    /// `preferred_dos_name()`, which is everything a registry entry needs.
    devices: HashMap<u32, DeviceAnnounceHeader>,
    pending_irps: IrpTracker,
    backend: Box<dyn RdpdrServerBackend>,
}

impl_as_any!(RdpdrServer);

/// Decodes a `PAKID_CORE_CLIENTID_CONFIRM` body as the client's Announce Reply.
///
/// Uses [`VersionAndIdPdu::decode_client_announce_reply`] rather than the general
/// [`RdpdrPdu::decode_body`] dispatch: that PacketId is shared with `ServerClientIdConfirm`
/// (MS-RDPEFS 2.2.1.1), and only this explicit entry point tags the decoded value with
/// the correct [`VersionAndIdPduKind::ClientAnnounceReply`] for a server receiving it.
fn decode_client_announce_reply(src: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
    let reply = VersionAndIdPdu::decode_client_announce_reply(src).map_err(|e| decode_err!(e))?;
    trace!(
        version_major = reply.version_major,
        version_minor = reply.version_minor,
        client_id = reply.client_id,
        "received ClientAnnounceReply"
    );
    Ok(Vec::new())
}

/// Builds a `DeviceIoRequest` header for a `drive_*` method.
///
/// A free function rather than a method: every `drive_*` method below builds the
/// type-specific request body inside a closure passed to
/// [`RdpdrServer::send_device_io_request`], and that closure already borrows `self`'s
/// caller for the duration of the call, so the header can't be built from a `&self`
/// method without a borrow conflict.
fn device_io_request_header(
    device_id: u32,
    file_id: u32,
    major_function: MajorFunction,
    minor_function: MinorFunction,
    completion_id: u32,
) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id,
        file_id,
        completion_id,
        major_function,
        minor_function,
    }
}

impl RdpdrServer {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpdr\0\0\0");

    pub fn new(backend: Box<dyn RdpdrServerBackend>) -> Self {
        Self {
            state: RdpdrServerState::Initializing,
            client_id: 0,
            devices: HashMap::new(),
            pending_irps: IrpTracker::new(),
            backend,
        }
    }

    pub fn downcast_backend<T: RdpdrServerBackend>(&self) -> Option<&T> {
        self.backend.as_any().downcast_ref::<T>()
    }

    pub fn downcast_backend_mut<T: RdpdrServerBackend>(&mut self) -> Option<&mut T> {
        self.backend.as_any_mut().downcast_mut::<T>()
    }

    fn handle_client_name_request(&mut self, src: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
        let request = ClientNameRequest::decode(src).map_err(|e| decode_err!(e))?;
        let computer_name = match &request {
            ClientNameRequest::Ascii(name) | ClientNameRequest::Unicode(name) => name.clone(),
        };
        self.backend.on_client_name(&computer_name);

        let capability_request = RdpdrPdu::CoreCapability(CoreCapability {
            capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_drive()],
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        });
        let client_id_confirm = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_13,
            client_id: self.client_id,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        });

        self.state = RdpdrServerState::CapabilityExchange;
        Ok(vec![
            SvcMessage::from(capability_request),
            SvcMessage::from(client_id_confirm),
        ])
    }

    fn handle_client_capability_response(
        &mut self,
        header: SharedHeader,
        src: &mut ReadCursor<'_>,
    ) -> PduResult<Vec<SvcMessage>> {
        let pdu = RdpdrPdu::decode_body(header, src).map_err(|e| decode_err!(e))?;
        let RdpdrPdu::CoreCapability(response) = pdu else {
            return Err(pdu_other_err!("RdpdrServer", "expected a CoreCapability body"));
        };
        trace!(
            capability_count = response.capabilities.len(),
            "received ClientCoreCapabilityResponse"
        );
        self.state = RdpdrServerState::Active;
        Ok(Vec::new())
    }

    fn handle_device_list_announce(&mut self, src: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
        let announce = ClientDeviceListAnnounce::decode(src).map_err(|e| decode_err!(e))?;
        let decisions = self.backend.on_device_announce(&announce.device_list);

        let mut messages = Vec::with_capacity(announce.device_list.len());
        for device in announce.device_list {
            let device_id = device.device_id();
            let accepted = decisions.iter().any(|(id, accepted)| *id == device_id && *accepted);
            let result_code = if accepted {
                self.devices.insert(device_id, device);
                NtStatus::SUCCESS
            } else {
                NtStatus::ACCESS_DENIED
            };
            messages.push(SvcMessage::from(RdpdrPdu::ServerDeviceAnnounceResponse(
                ServerDeviceAnnounceResponse { device_id, result_code },
            )));
        }
        Ok(messages)
    }

    fn handle_device_list_remove(&mut self, src: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
        let remove = ClientDeviceListRemove::decode(src).map_err(|e| decode_err!(e))?;
        for device_id in &remove.device_list {
            self.devices.remove(device_id);
        }
        self.backend.on_device_remove(&remove.device_list);
        Ok(Vec::new())
    }

    fn handle_device_io_completion(&mut self, src: &mut ReadCursor<'_>) -> PduResult<Vec<SvcMessage>> {
        let device_io_response = DeviceIoResponse::decode(src).map_err(|e| decode_err!(e))?;
        let Some(pending) = self.pending_irps.complete(device_io_response.completion_id) else {
            warn!(
                completion_id = device_io_response.completion_id,
                "ignoring RDPDR completion for an unknown or already-completed request"
            );
            return Ok(Vec::new());
        };

        let (major_function, minor_function, info_class, volume_info_class) = pending.completion_classifiers();
        let pdu = RdpdrPdu::decode_io_completion(
            major_function,
            minor_function,
            info_class,
            volume_info_class,
            device_io_response,
            src,
        )
        .map_err(|e| decode_err!(e))?;

        self.dispatch_completion(pdu)?;
        Ok(Vec::new())
    }

    fn dispatch_completion(&mut self, pdu: RdpdrPdu) -> PduResult<()> {
        match pdu {
            RdpdrPdu::DeviceCreateResponse(response) => self.backend.on_create_complete(&response),
            RdpdrPdu::DeviceCloseResponse(response) => self.backend.on_close_complete(&response),
            RdpdrPdu::DeviceReadResponse(response) => self.backend.on_read_complete(&response),
            RdpdrPdu::DeviceWriteResponse(response) => self.backend.on_write_complete(&response),
            RdpdrPdu::DeviceFlushBuffersResponse(response) => self.backend.on_flush_buffers_complete(&response),
            RdpdrPdu::DeviceControlResponse(response) => self.backend.on_device_control_complete(&response),
            RdpdrPdu::ClientDriveQueryInformationResponse(response) => {
                self.backend.on_query_information_complete(&response)
            }
            RdpdrPdu::ClientDriveSetInformationResponse(response) => {
                self.backend.on_set_information_complete(&response)
            }
            RdpdrPdu::ClientDriveQueryDirectoryResponse(response) => {
                self.backend.on_query_directory_complete(&response)
            }
            RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(response) => {
                self.backend.on_notify_change_directory_complete(&response)
            }
            RdpdrPdu::ClientDriveQueryVolumeInformationResponse(response) => {
                self.backend.on_query_volume_information_complete(&response)
            }
            RdpdrPdu::ClientDriveLockControlResponse(response) => self.backend.on_lock_control_complete(&response),
            RdpdrPdu::ClientDriveQuerySecurityResponse(response) => self.backend.on_query_security_complete(&response),
            RdpdrPdu::ClientDriveSetSecurityResponse(response) => self.backend.on_set_security_complete(&response),
            // `RdpdrPdu::decode_io_completion` only ever returns one of the thirteen
            // variants matched above (it errors rather than returning any other
            // `RdpdrPdu` variant, and rejects `SetVolumeInformation` outright); this arm
            // stays reachable-but-unused rather than `unreachable!()` so a future
            // `decode_io_completion` addition this module hasn't caught up to fails
            // loudly instead of panicking.
            _ => Err(pdu_other_err!(
                "RdpdrServer",
                "decode_io_completion produced an unexpected PDU variant"
            )),
        }
    }

    /// Sends a `PAKID_CORE_DEVICE_IOREQUEST` and registers `pending` under a freshly
    /// allocated `CompletionId`.
    ///
    /// `RdpdrPdu::DeviceIoRequest` only carries the fixed request header (`pdu::RdpdrPdu::header`
    /// maps every completion response to one shared `PacketId` for the same structural
    /// reason: the type-specific body of a create/read/write/etc. request lives on the
    /// request type itself, not on a `RdpdrPdu` variant). Every `drive_*` method below,
    /// and every request type that doesn't have a named `drive_*` wrapper, goes through
    /// this method rather than `RdpdrPdu::encode` for that reason.
    ///
    /// `build` receives the allocated `CompletionId` so it can finish constructing the
    /// request's `DeviceIoRequest` header before encoding.
    fn send_device_io_request<T: DriveRequestBody>(
        &mut self,
        pending: PendingIrp,
        build: impl FnOnce(u32) -> T,
    ) -> PduResult<Vec<SvcMessage>> {
        if self.state != RdpdrServerState::Active {
            return Err(pdu_other_err!("RdpdrServer", "drive I/O requires an active session"));
        }

        let completion_id = self.pending_irps.register(pending);
        let request = build(completion_id);

        let mut bytes = vec![0u8; SHARED_HEADER_SIZE + request.wire_size()];
        let mut dst = WriteCursor::new(&mut bytes);
        dst.write_u16(Component::RdpdrCtypCore.into());
        dst.write_u16(PacketId::CoreDeviceIoRequest.into());
        request.encode_into(&mut dst).map_err(|e| encode_err!(e))?;

        Ok(vec![SvcMessage::from(bytes)])
    }

    /// Opens or creates a file on a client-redirected drive.
    ///
    /// `file_attributes` is fixed at none (the client applies its own defaults) and
    /// `shared_access` at [`SharedAccess::all`] (don't lock out other opens). Neither
    /// is caller-configurable yet: covering that would mean widening this method's
    /// signature or exposing the internal request-building path, and no embedder has
    /// needed it so far.
    pub fn drive_create(
        &mut self,
        device_id: u32,
        path: impl Into<String>,
        desired_access: DesiredAccess,
        create_disposition: CreateDisposition,
        create_options: CreateOptions,
    ) -> PduResult<Vec<SvcMessage>> {
        let path = path.into();
        self.send_device_io_request(PendingIrp::Create, |completion_id| DeviceCreateRequest {
            device_io_request: device_io_request_header(
                device_id,
                0,
                MajorFunction::Create,
                MinorFunction::from(0),
                completion_id,
            ),
            desired_access,
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::all(),
            create_disposition,
            create_options,
            path,
        })
    }

    /// Reads `length` bytes at `offset` from an open file handle.
    pub fn drive_read(&mut self, device_id: u32, file_id: u32, length: u32, offset: u64) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::Read, |completion_id| DeviceReadRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::Read,
                MinorFunction::from(0),
                completion_id,
            ),
            length,
            offset,
        })
    }

    /// Writes `data` at `offset` to an open file handle.
    pub fn drive_write(
        &mut self,
        device_id: u32,
        file_id: u32,
        data: Vec<u8>,
        offset: u64,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::Write, |completion_id| DeviceWriteRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::Write,
                MinorFunction::from(0),
                completion_id,
            ),
            offset,
            write_data: data,
        })
    }

    /// Closes an open file handle.
    pub fn drive_close(&mut self, device_id: u32, file_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::Close, |completion_id| DeviceCloseRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::Close,
                MinorFunction::from(0),
                completion_id,
            ),
        })
    }

    /// Flushes buffered writes for an open file handle.
    pub fn drive_flush_buffers(&mut self, device_id: u32, file_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::FlushBuffers, |completion_id| DeviceFlushBuffersRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::FlushBuffers,
                MinorFunction::from(0),
                completion_id,
            ),
        })
    }

    /// Queries file attributes/size for an open file handle.
    pub fn drive_query_information(
        &mut self,
        device_id: u32,
        file_id: u32,
        info_class: FileInformationClassLevel,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(
            PendingIrp::QueryInformation {
                info_class: info_class.clone(),
            },
            |completion_id| ServerDriveQueryInformationRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::QueryInformation,
                    MinorFunction::from(0),
                    completion_id,
                ),
                file_info_class_lvl: info_class,
            },
        )
    }

    /// Sets file attributes (rename, delete-on-close, truncate, etc.) for an open file handle.
    pub fn drive_set_information(
        &mut self,
        device_id: u32,
        file_id: u32,
        set_buffer: FileInformationClass,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::SetInformation, |completion_id| {
            ServerDriveSetInformationRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::SetInformation,
                    MinorFunction::from(0),
                    completion_id,
                ),
                set_buffer,
            }
        })
    }

    /// Lists directory contents for an open directory handle.
    ///
    /// `initial_query` MUST be `true` only for the first `drive_query_directory` call
    /// against a given `file_id` (it tells the client to restart its directory
    /// enumeration cursor); subsequent calls continuing the same listing pass `false`.
    pub fn drive_query_directory(
        &mut self,
        device_id: u32,
        file_id: u32,
        info_class: FileInformationClassLevel,
        path: impl Into<String>,
        initial_query: bool,
    ) -> PduResult<Vec<SvcMessage>> {
        let path = path.into();
        self.send_device_io_request(
            PendingIrp::QueryDirectory {
                info_class: info_class.clone(),
            },
            |completion_id| ServerDriveQueryDirectoryRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::DirectoryControl,
                    MinorFunction::IRP_MN_QUERY_DIRECTORY,
                    completion_id,
                ),
                file_info_class_lvl: info_class,
                initial_query: u8::from(initial_query),
                path,
            },
        )
    }

    /// Requests notification of changes within an open directory handle.
    pub fn drive_notify_change_directory(
        &mut self,
        device_id: u32,
        file_id: u32,
        watch_tree: bool,
        completion_filter: u32,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::NotifyChangeDirectory, |completion_id| {
            ServerDriveNotifyChangeDirectoryRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::DirectoryControl,
                    MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
                    completion_id,
                ),
                watch_tree: u8::from(watch_tree),
                completion_filter,
            }
        })
    }

    /// Queries volume/filesystem information for an open file handle.
    pub fn drive_query_volume_information(
        &mut self,
        device_id: u32,
        file_id: u32,
        fs_info_class: FileSystemInformationClassLevel,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(
            PendingIrp::QueryVolumeInformation {
                volume_info_class: fs_info_class.clone(),
            },
            |completion_id| ServerDriveQueryVolumeInformationRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::QueryVolumeInformation,
                    MinorFunction::from(0),
                    completion_id,
                ),
                fs_info_class_lvl: fs_info_class,
            },
        )
    }

    /// Sends a byte-range lock or unlock request for an open file handle.
    pub fn drive_lock_control(
        &mut self,
        device_id: u32,
        file_id: u32,
        operation: LockOperation,
        wait: bool,
        locks: Vec<RdpLockInfo>,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::LockControl, |completion_id| ServerDriveLockControlRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::LockControl,
                MinorFunction::from(0),
                completion_id,
            ),
            operation,
            wait,
            locks,
        })
    }

    /// Queries the security descriptor of an open file handle.
    pub fn drive_query_security(
        &mut self,
        device_id: u32,
        file_id: u32,
        security_information: SecurityInformation,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::QuerySecurity, |completion_id| {
            ServerDriveQuerySecurityRequest {
                device_io_request: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::QuerySecurity,
                    MinorFunction::from(0),
                    completion_id,
                ),
                security_information,
            }
        })
    }

    /// Sets the security descriptor of an open file handle.
    pub fn drive_set_security(
        &mut self,
        device_id: u32,
        file_id: u32,
        security_information: SecurityInformation,
        security_descriptor: Vec<u8>,
    ) -> PduResult<Vec<SvcMessage>> {
        self.send_device_io_request(PendingIrp::SetSecurity, |completion_id| ServerDriveSetSecurityRequest {
            device_io_request: device_io_request_header(
                device_id,
                file_id,
                MajorFunction::SetSecurity,
                MinorFunction::from(0),
                completion_id,
            ),
            security_information,
            security_descriptor,
        })
    }

    /// Sends a device-control (IOCTL) request to an open file handle.
    pub fn drive_device_control(
        &mut self,
        device_id: u32,
        file_id: u32,
        io_control_code: u32,
        input_buffer: Vec<u8>,
        output_buffer_length: u32,
    ) -> PduResult<Vec<SvcMessage>> {
        let input_buffer_length =
            u32::try_from(input_buffer.len()).map_err(|_| pdu_other_err!("RdpdrServer", "input buffer too large"))?;
        self.send_device_io_request(PendingIrp::DeviceControl, |completion_id| DeviceControlIo {
            request: DeviceControlRequest {
                header: device_io_request_header(
                    device_id,
                    file_id,
                    MajorFunction::DeviceControl,
                    MinorFunction::from(0),
                    completion_id,
                ),
                output_buffer_length,
                input_buffer_length,
                io_control_code: AnyIoCtlCode(io_control_code),
            },
            input_buffer,
        })
    }
}

/// A [`DeviceControlRequest`] bundled with the opaque input buffer that follows it on
/// the wire, since the request type itself (matching its decode-side counterpart,
/// [`crate::pdu::efs::DecodedDeviceControlRequest`]) doesn't carry the buffer.
struct DeviceControlIo {
    request: DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: Vec<u8>,
}

impl DriveRequestBody for DeviceControlIo {
    fn encode_into(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.request.encode(dst)?;
        dst.write_slice(&self.input_buffer);
        Ok(())
    }

    fn wire_size(&self) -> usize {
        self.request.size() + self.input_buffer.len()
    }
}

/// A drive request type whose `.encode()`/`.size()` already include its
/// [`DeviceIoRequest`] header, letting [`RdpdrServer::send_device_io_request`] treat
/// every request type uniformly.
trait DriveRequestBody {
    fn encode_into(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()>;
    fn wire_size(&self) -> usize;
}

macro_rules! impl_drive_request_body {
    ($($ty:ty),+ $(,)?) => {
        $(impl DriveRequestBody for $ty {
            fn encode_into(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
                self.encode(dst)
            }
            fn wire_size(&self) -> usize {
                self.size()
            }
        })+
    };
}

impl_drive_request_body!(
    DeviceCreateRequest,
    DeviceReadRequest,
    DeviceWriteRequest,
    DeviceCloseRequest,
    DeviceFlushBuffersRequest,
    ServerDriveQueryInformationRequest,
    ServerDriveSetInformationRequest,
    ServerDriveQueryDirectoryRequest,
    ServerDriveNotifyChangeDirectoryRequest,
    ServerDriveQueryVolumeInformationRequest,
    ServerDriveLockControlRequest,
    ServerDriveQuerySecurityRequest,
    ServerDriveSetSecurityRequest,
);

impl SvcProcessor for RdpdrServer {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::WhenRdpDataIsCompressed
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        let mut bytes = [0u8; size_of::<u32>()];
        getrandom::fill(&mut bytes).map_err(|err| pdu_other_err!("generating rdpdr server client id", source: err))?;
        self.client_id = u32::from_le_bytes(bytes);

        self.state = RdpdrServerState::AwaitingAnnounce;
        let announce = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_13,
            client_id: self.client_id,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        });
        trace!("sending {:?}", announce);
        Ok(vec![SvcMessage::from(announce)])
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        let mut src = ReadCursor::new(payload);
        let header = SharedHeader::decode(&mut src).map_err(|e| decode_err!(e))?;

        match (self.state, header.packet_id) {
            (RdpdrServerState::AwaitingAnnounce, PacketId::CoreClientidConfirm) => {
                decode_client_announce_reply(&mut src)
            }
            (RdpdrServerState::AwaitingAnnounce, PacketId::CoreClientName) => self.handle_client_name_request(&mut src),
            (RdpdrServerState::CapabilityExchange, PacketId::CoreClientCapability) => {
                self.handle_client_capability_response(header, &mut src)
            }
            // Issue #195 (duplicate RDPDR init sequence): a client re-sending its
            // Announce Reply while this server is already Active is treated as a
            // fresh initialization rather than a protocol error. Devices and
            // in-flight IRPs from the previous session are discarded; the backend
            // will see a new `on_device_announce` batch once the client re-announces.
            (RdpdrServerState::Active, PacketId::CoreClientidConfirm) => {
                warn!("received a duplicate RDPDR initialization sequence while active; re-initializing");
                self.devices.clear();
                self.pending_irps.clear();
                self.state = RdpdrServerState::AwaitingAnnounce;
                decode_client_announce_reply(&mut src)
            }
            (RdpdrServerState::CapabilityExchange, PacketId::CoreDevicelistAnnounce)
            | (RdpdrServerState::Active, PacketId::CoreDevicelistAnnounce) => {
                self.handle_device_list_announce(&mut src)
            }
            (RdpdrServerState::Active, PacketId::CoreDevicelistRemove) => self.handle_device_list_remove(&mut src),
            (RdpdrServerState::Active, PacketId::CoreDeviceIoCompletion) => self.handle_device_io_completion(&mut src),
            (state, packet_id) => {
                warn!(?state, %packet_id, "RdpdrServer: unexpected packet for the current state");
                Err(pdu_other_err!("RdpdrServer", "unexpected packet for the current state"))
            }
        }
    }
}

impl SvcServerProcessor for RdpdrServer {}

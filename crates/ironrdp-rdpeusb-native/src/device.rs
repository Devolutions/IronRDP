//! A redirected USB device backed by nusb.
//!
//! Each redirected device owns a worker thread running a single-threaded Tokio runtime. The RDPEUSB
//! state machine calls [`NusbDeviceBackend`] from the session loop, which must not block: requests that
//! need device I/O are queued to the worker, and their completions come back through the event sink.
//!
//! Inside the worker, every bulk or interrupt endpoint gets its own task that keeps its transfers in
//! submission order, as the host controller does. Control transfers run as independent tasks.

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpeusb::client::UrbdrcDeviceBackend;
use ironrdp_rdpeusb::io::device::{
    DeviceInfo, UsbClassCodes, UsbConfigInfo, UsbConnectionSpeed, UsbDeviceDescriptorInfo, UsbDeviceLocation,
    UsbInterfaceInfo,
};
use ironrdp_rdpeusb::io::{
    CompletionData, DeviceText, InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, RequestId,
    TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket,
};
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::{
    TsUrbGetCurrFrameNumResult, TsUrbResultPayload, TsUrbSelectConfigResult,
};
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::utils::TsUsbdInterfaceInfo;
use ironrdp_rdpeusb::pdu::utils::UsbdStatus;
use ironrdp_usb::control::{Recipient, RequestKind, SetupPacket};
use ironrdp_usb::{BcdVersion, Direction};
use nusb::descriptors::TransferType;
use nusb::transfer::{
    Buffer, Bulk, BulkOrInterrupt, Completion, ControlIn, ControlOut, ControlType, EndpointDirection, In, Interrupt,
    Out, Recipient as NusbRecipient, TransferError,
};
use nusb::{Endpoint, MaybeFuture as _};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tracing::{debug, trace, warn};

use crate::filter::UsbDeviceIdentity;
use crate::manager::{DeviceKey, RedirectableDevice};
use crate::urb::{self, ControlRoute, Envelope, PipeOperation, UsbRequest, usbd_status};
use crate::{EventOrigin, UrbdrcEventKind, UrbdrcEventSink};

/// A local device that was opened, with every interface of its active configuration claimed.
pub(crate) struct OpenedDevice {
    pub(crate) id: nusb::DeviceId,
    device: nusb::Device,
    interfaces: BTreeMap<u8, nusb::Interface>,
    info: DeviceInfo,
    description: String,
    location: String,
}

impl OpenedDevice {
    /// Open a device and claim all of its interfaces.
    ///
    /// This blocks while the OS opens the device. RDPEUSB redirects whole devices, so a device with an
    /// interface that cannot be claimed is not redirected. On Windows, this is the case for every
    /// interface whose function driver is not WinUSB.
    pub(crate) fn open(info: &nusb::DeviceInfo, identity: &UsbDeviceIdentity) -> Result<Self, OpenError> {
        let device = info
            .open()
            .wait()
            .map_err(|source| OpenError::new("open device", source))?;

        let config = device
            .active_configuration()
            .map_err(|_| OpenError::message("device is not configured"))?;
        let mut interface_info = Vec::new();
        for interface in config
            .interface_alt_settings()
            .filter(|interface| interface.alternate_setting() == 0)
        {
            interface_info.push((
                interface.interface_number(),
                UsbClassCodes {
                    class_code: interface.class(),
                    sub_class_code: interface.subclass(),
                    protocol_code: interface.protocol(),
                },
            ));
        }

        let mut interfaces = BTreeMap::new();
        for (number, _) in &interface_info {
            let interface = device
                .detach_and_claim_interface(*number)
                .wait()
                .map_err(|source| OpenError::new("claim interface", source).with_interface(*number))?;
            interfaces.insert(*number, interface);
        }

        let descriptor = device.device_descriptor();

        Ok(Self {
            id: info.id(),
            info: DeviceInfo {
                location: UsbDeviceLocation {
                    bus_number: identity.bus_number,
                    address: identity.address,
                    port_numbers: info.port_chain().to_vec(),
                },
                descriptor: UsbDeviceDescriptorInfo {
                    vendor_id: descriptor.vendor_id(),
                    product_id: descriptor.product_id(),
                    device_version: descriptor.device_version(),
                    usb_version: BcdVersion::from_raw(descriptor.usb_version()),
                    class_codes: UsbClassCodes {
                        class_code: descriptor.class(),
                        sub_class_code: descriptor.subclass(),
                        protocol_code: descriptor.protocol(),
                    },
                    num_configurations: descriptor.num_configurations(),
                },
                active_config: Some(UsbConfigInfo {
                    interfaces: interface_info
                        .into_iter()
                        .map(|(_, class_codes)| UsbInterfaceInfo { class_codes })
                        .collect(),
                }),
                speed: connection_speed(device.speed()),
                // Isochronous transfers are rejected (see `urb::isochronous_rejection`).
                isochronous: false,
            },
            description: info
                .product_string()
                .map_or_else(|| String::from("USB Device"), str::to_owned),
            location: format!(
                "Port_#{:04}.Hub_#{:04}",
                info.port_chain().last().copied().unwrap_or(0),
                identity.bus_number
            ),
            device,
            interfaces,
        })
    }
}

impl RedirectableDevice for OpenedDevice {
    fn key(&self) -> DeviceKey {
        DeviceKey::Nusb(self.id)
    }

    fn vendor_product(&self) -> (u16, u16) {
        (self.info.descriptor.vendor_id, self.info.descriptor.product_id)
    }

    fn start(
        self: Box<Self>,
        reply: Reply,
        events_token: Arc<()>,
        on_close: Box<dyn FnOnce() + Send>,
    ) -> Box<dyn UrbdrcDeviceBackend> {
        Box::new(NusbDeviceBackend::start(*self, reply, events_token, on_close))
    }
}

/// Describe a device from its enumeration data alone, without opening it.
///
/// For backends that cannot read the descriptors of a device before redirecting it. The device is
/// assumed to have a single configuration.
#[cfg(windows)]
pub(crate) fn describe(
    info: &nusb::DeviceInfo,
    identity: &UsbDeviceIdentity,
    isochronous: bool,
) -> (DeviceInfo, String) {
    let device_info = DeviceInfo {
        location: UsbDeviceLocation {
            bus_number: identity.bus_number,
            address: identity.address,
            port_numbers: info.port_chain().to_vec(),
        },
        descriptor: UsbDeviceDescriptorInfo {
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            device_version: info.device_version(),
            usb_version: BcdVersion::from_raw(info.usb_version()),
            class_codes: UsbClassCodes {
                class_code: info.class(),
                sub_class_code: info.subclass(),
                protocol_code: info.protocol(),
            },
            num_configurations: 1,
        },
        active_config: Some(UsbConfigInfo {
            interfaces: info
                .interfaces()
                .map(|interface| UsbInterfaceInfo {
                    class_codes: UsbClassCodes {
                        class_code: interface.class(),
                        sub_class_code: interface.subclass(),
                        protocol_code: interface.protocol(),
                    },
                })
                .collect(),
        }),
        speed: connection_speed(info.speed()),
        isochronous,
    };
    let description = info
        .product_string()
        .map_or_else(|| String::from("USB Device"), str::to_owned);
    (device_info, description)
}

fn connection_speed(speed: Option<nusb::Speed>) -> UsbConnectionSpeed {
    match speed {
        Some(nusb::Speed::Low) => UsbConnectionSpeed::Low,
        Some(nusb::Speed::Full) => UsbConnectionSpeed::Full,
        Some(nusb::Speed::High) => UsbConnectionSpeed::High,
        Some(nusb::Speed::Super) => UsbConnectionSpeed::Super,
        Some(nusb::Speed::SuperPlus) => UsbConnectionSpeed::SuperPlus,
        _ => UsbConnectionSpeed::Unknown,
    }
}

/// Build the identity redirection rules are matched against.
pub(crate) fn identity(info: &nusb::DeviceInfo) -> UsbDeviceIdentity {
    UsbDeviceIdentity {
        vendor_id: info.vendor_id(),
        product_id: info.product_id(),
        bus_number: bus_number(info.bus_id()),
        address: info.device_address(),
        device_class: info.class(),
        interface_classes: info.interfaces().map(|interface| interface.class()).collect(),
        #[cfg(windows)]
        instance_id: Some(info.instance_id().to_string_lossy().into_owned()),
        #[cfg(not(windows))]
        instance_id: info.serial_number().filter(|serial| !serial.is_empty()).map(|serial| {
            format!(
                "USB\\VID_{:04X}&PID_{:04X}\\{serial}",
                info.vendor_id(),
                info.product_id()
            )
        }),
    }
}

/// Map a platform bus identifier to the number used in `addr:` rules and device IDs.
///
/// Linux and macOS identify buses with numbers. Windows uses host controller instance IDs, numbered
/// from 1 by their position among the controllers present now, sorted by instance ID. Nothing is
/// cached, so a fresh enumeration such as [`crate::local_devices`] reports the same numbers as long as
/// no controller was added or removed in between. `addr:` rules apply to the devices present when the
/// channel opens, which is when those devices are numbered.
fn bus_number(bus_id: &str) -> u8 {
    if let Ok(number) = bus_id.parse::<u8>() {
        return number;
    }

    let present = nusb::list_buses()
        .wait()
        .map(|buses| buses.map(|bus| bus.bus_id().to_owned()).collect())
        .unwrap_or_default();
    controller_number(bus_id, present)
}

/// Number `bus_id` by its position, from 1, among the `present` controllers sorted by instance ID.
fn controller_number(bus_id: &str, mut present: Vec<String>) -> u8 {
    present.push(bus_id.to_owned());
    present.sort();
    present.dedup();
    let index = present.iter().position(|id| id == bus_id).unwrap_or(0);
    u8::try_from(index.saturating_add(1)).unwrap_or(u8::MAX)
}

/// Error returned when a local device cannot be opened for redirection.
#[derive(Debug)]
pub struct OpenError {
    context: &'static str,
    interface: Option<u8>,
    source: Option<nusb::Error>,
}

impl OpenError {
    fn new(context: &'static str, source: nusb::Error) -> Self {
        Self {
            context,
            interface: None,
            source: Some(source),
        }
    }

    fn message(context: &'static str) -> Self {
        Self {
            context,
            interface: None,
            source: None,
        }
    }

    fn with_interface(mut self, interface: u8) -> Self {
        self.interface = Some(interface);
        self
    }
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.context)?;
        if let Some(interface) = self.interface {
            write!(f, " {interface}")?;
        }
        if let Some(source) = &self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl core::error::Error for OpenError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match &self.source {
            Some(source) => Some(source),
            None => None,
        }
    }
}

/// Device-channel event routing shared by discovery and worker tasks.
#[derive(Clone)]
pub(crate) struct Reply {
    channel_id: u32,
    events: EventOrigin,
}

impl Reply {
    pub(crate) fn new(channel_id: u32, token: &Arc<()>, sink: UrbdrcEventSink) -> Self {
        Self {
            channel_id,
            events: EventOrigin::new(token, sink),
        }
    }

    pub(crate) fn channel_id(&self) -> u32 {
        self.channel_id
    }

    pub(crate) fn same_channel(&self, other: &Self) -> bool {
        self.events.token.ptr_eq(&other.events.token)
    }

    pub(crate) fn complete(&self, request_id: RequestId, completion: CompletionData) {
        self.events.send(UrbdrcEventKind::Completion {
            channel_id: self.channel_id,
            request_id,
            completion,
        });
    }

    /// Close the device channel after unplugging or an error that cannot be reported.
    pub(crate) fn terminate(&self) {
        self.events.send(UrbdrcEventKind::CloseChannel {
            channel_id: self.channel_id,
        });
    }
}

/// Remaining bytes of transfer buffers a device may hold, shared by its in-flight requests.
pub(crate) struct TransferBudget {
    available: AtomicUsize,
}

/// Bytes taken from a [`TransferBudget`], given back when the request completes.
pub(crate) struct Reservation {
    budget: Arc<TransferBudget>,
    bytes: usize,
}

impl TransferBudget {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            available: AtomicUsize::new(urb::MAX_IN_FLIGHT_BYTES),
        })
    }

    pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        self.available
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |available| {
                available.checked_sub(bytes)
            })
            .ok()?;
        Some(Reservation {
            budget: Arc::clone(self),
            bytes,
        })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.available.fetch_add(self.bytes, Ordering::AcqRel);
    }
}

enum Command {
    Submit {
        request_id: RequestId,
        envelope: Envelope,
        request: UsbRequest,
        reservation: Reservation,
    },
    Cancel(RequestId),
}

#[derive(Default)]
struct ControlRequests {
    pending: HashMap<RequestId, (tokio::task::Id, Envelope)>,
    tasks: JoinSet<(RequestId, CompletionData)>,
}

impl ControlRequests {
    fn spawn(
        &mut self,
        request_id: RequestId,
        envelope: Envelope,
        reservation: Reservation,
        future: impl Future<Output = CompletionData> + Send + 'static,
    ) {
        let task = self.tasks.spawn(async move {
            let _reservation = reservation;
            (request_id, future.await)
        });
        self.pending.insert(request_id, (task.id(), envelope));
    }

    fn cancel(&mut self, request_id: RequestId) -> Option<CompletionData> {
        let (_, envelope) = self.pending.remove(&request_id)?;
        // nusb retains submitted control transfers after their future is dropped.
        // Keep polling and charging the physical operation, but complete the logical request now.
        // UNTESTED: native controls must finish by their timeout so shutdown can release claims.
        // Verify cancelled-control timeouts on hardware when changing nusb or this lifetime handling.
        Some(urb::failure_completion(envelope, usbd_status::CANCELED))
    }

    async fn next_completed(&mut self) -> Result<Option<(RequestId, CompletionData)>, tokio::task::JoinError> {
        while let Some(completed) = self.tasks.join_next_with_id().await {
            let (task_id, (request_id, completion)) = completed?;
            if self.pending.get(&request_id).is_some_and(|(id, _)| *id == task_id) {
                self.pending.remove(&request_id);
                return Ok(Some((request_id, completion)));
            }
        }
        Ok(None)
    }
}

/// [`UrbdrcDeviceBackend`] for a device opened with nusb.
pub(crate) struct NusbDeviceBackend {
    info: DeviceInfo,
    description: String,
    location: String,
    started: Instant,
    commands: Option<mpsc::UnboundedSender<Command>>,
    budget: Arc<TransferBudget>,
    reply: Reply,
    /// Keeps the events of this device current; they are dropped once the device channel is gone.
    _events_token: Arc<()>,
}

/// Runs the wrapped callback when dropped.
pub(crate) struct OnClose(pub(crate) Option<Box<dyn FnOnce() + Send>>);

impl Drop for OnClose {
    fn drop(&mut self) {
        if let Some(on_close) = self.0.take() {
            on_close();
        }
    }
}

impl NusbDeviceBackend {
    /// Start the worker thread for `device`, using its device-channel event origin.
    ///
    /// `on_close` runs once the worker has released the device, so that it can be offered again.
    pub(crate) fn start(
        device: OpenedDevice,
        reply: Reply,
        events_token: Arc<()>,
        on_close: Box<dyn FnOnce() + Send>,
    ) -> Self {
        let OpenedDevice {
            id: _,
            device,
            interfaces,
            info,
            description,
            location,
        } = device;
        let (commands, receiver) = mpsc::unbounded_channel();

        let worker = Worker {
            device,
            interfaces,
            endpoints: BTreeMap::new(),
            controls: ControlRequests::default(),
            reply: reply.clone(),
        };
        let thread_name = format!(
            "urbdrc-{:04x}:{:04x}",
            info.descriptor.vendor_id, info.descriptor.product_id
        );
        // Dropped last on the worker thread, after the runtime and the claimed interfaces. If the
        // thread cannot start, the closure is dropped right away and the callback still runs.
        let on_close = OnClose(Some(on_close));
        let spawned = thread::Builder::new().name(thread_name).spawn(move || {
            let _on_close = on_close;
            match tokio::runtime::Builder::new_current_thread().enable_time().build() {
                Ok(runtime) => runtime.block_on(worker.run(receiver)),
                Err(error) => warn!(%error, "Failed to start the USB device worker runtime"),
            }
        });
        let commands = match spawned {
            Ok(_) => Some(commands),
            Err(error) => {
                warn!(%error, "Failed to start the USB device worker thread");
                None
            }
        };

        Self {
            info,
            description,
            location,
            started: Instant::now(),
            commands,
            budget: TransferBudget::new(),
            reply,
            _events_token: events_token,
        }
    }

    fn frame_number(&self) -> u32 {
        // RDPEUSB only needs a monotonic 1 ms frame counter; it wraps like the USB frame number.
        let [b0, b1, b2, b3, ..] = self.started.elapsed().as_millis().to_le_bytes();
        u32::from_le_bytes([b0, b1, b2, b3])
    }

    /// Execute `request` locally when possible, otherwise queue it to the worker.
    ///
    /// Returns the completion when it is available immediately.
    fn dispatch(&self, request_id: RequestId, envelope: Envelope, request: UsbRequest) -> Option<CompletionData> {
        match request {
            UsbRequest::CurrentFrameNumber => Some(urb::payload_completion(
                envelope,
                TsUrbResultPayload::FrameNum(TsUrbGetCurrFrameNumResult {
                    frame_number: self.frame_number(),
                }),
            )),
            UsbRequest::Isochronous { packets } => {
                debug!("Rejecting an isochronous transfer, which the USB backend does not support");
                Some(urb::isochronous_rejection(envelope, &packets))
            }
            UsbRequest::Reject(status) => Some(urb::failure_completion(envelope, status)),
            request => {
                let charge = urb::buffer_bytes(&request).saturating_add(urb::REQUEST_OVERHEAD_BYTES);
                let Some(reservation) = self.budget.reserve(charge) else {
                    debug!(request_id, "Rejecting a USB request beyond the in-flight buffer budget");
                    return Some(urb::failure_completion(envelope, usbd_status::INSUFFICIENT_RESOURCES));
                };
                let sent = self.commands.as_ref().is_some_and(|commands| {
                    commands
                        .send(Command::Submit {
                            request_id,
                            envelope,
                            request,
                            reservation,
                        })
                        .is_ok()
                });
                if sent {
                    None
                } else {
                    Some(urb::failure_completion(envelope, usbd_status::DEVICE_GONE))
                }
            }
        }
    }

    fn stop(&mut self) {
        // Dropping the last strong sender ends the worker, which then releases the device.
        self.commands = None;
    }
}

impl Drop for NusbDeviceBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

impl UrbdrcDeviceBackend for NusbDeviceBackend {
    fn device_info(&mut self, _channel_id: u32) -> PduResult<DeviceInfo> {
        Ok(self.info.clone())
    }

    fn cancel_request(&mut self, request_id: RequestId, _channel_id: u32) {
        if let Some(commands) = &self.commands {
            // The worker is gone when sending fails, and then nothing is left to cancel.
            let _ = commands.send(Command::Cancel(request_id));
        }
    }

    fn query_device_text(&mut self, _channel_id: u32, text_type: u32, _locale_id: u32) -> PduResult<DeviceText> {
        Ok(urb::device_text(text_type, &self.description, &self.location))
    }

    fn io_control(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        request: IoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        Ok(Some(urb::io_control(&request)))
    }

    fn internal_io_control(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        request: InternalIoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        match request {
            InternalIoControlPacket::QueryBusTime => Ok(Some(IoControlCompletionResult {
                hresult: 0,
                information: 4,
                output_buffer: self.frame_number().to_le_bytes().to_vec(),
            })),
        }
    }

    fn transfer_in(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: TransferInPacket,
    ) -> PduResult<Option<TransferInCompletionResult>> {
        let envelope = Envelope::In {
            output_buffer_size: request.output_buffer_size,
        };
        match self.dispatch(request_id, envelope, urb::translate_in(request)) {
            None => Ok(None),
            Some(CompletionData::TransferIn(completion)) => Ok(Some(completion)),
            Some(_) => Err(pdu_other_err!("transfer-in request completed with a different kind")),
        }
    }

    fn transfer_out(
        &mut self,
        _channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<Option<TransferOutCompletionResult>> {
        match self.dispatch(request_id, Envelope::Out, urb::translate_out(request)) {
            None => Ok(None),
            Some(CompletionData::TransferOut(completion)) => Ok(Some(completion)),
            Some(_) => Err(pdu_other_err!("transfer-out request completed with a different kind")),
        }
    }

    fn transfer_out_no_ack(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: TransferOutPacket,
    ) -> PduResult<()> {
        // NoAck is only valid for isochronous OUT transfers, which this backend does not perform.
        trace!("Dropping a no-ack isochronous OUT transfer");
        Ok(())
    }

    fn retract(&mut self, channel_id: u32) -> PduResult<()> {
        debug!(channel_id, "Server retracted the redirected USB device");
        self.stop();
        self.reply.events.send(UrbdrcEventKind::CloseChannel { channel_id });
        Ok(())
    }
}

/// Device-side state owned by the worker thread.
struct Worker {
    device: nusb::Device,
    interfaces: BTreeMap<u8, nusb::Interface>,
    endpoints: BTreeMap<u8, EndpointTask>,
    controls: ControlRequests,
    reply: Reply,
}

struct EndpointTask {
    interface: u8,
    commands: mpsc::UnboundedSender<EndpointCommand>,
    task: JoinHandle<()>,
}

impl Worker {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else {
                        break;
                    };
                    match command {
                        Command::Submit {
                            request_id,
                            envelope,
                            request,
                            reservation,
                        } => self.submit(request_id, envelope, request, reservation).await,
                        Command::Cancel(request_id) => {
                            if let Some(completion) = self.controls.cancel(request_id) {
                                self.reply.complete(request_id, completion);
                            } else {
                                for endpoint in self.endpoints.values() {
                                    let _ = endpoint.commands.send(EndpointCommand::Cancel(request_id));
                                }
                            }
                        }
                    }
                }
                completed = self.controls.next_completed(), if !self.controls.tasks.is_empty() => {
                    match completed {
                        Ok(Some((request_id, completion))) => self.reply.complete(request_id, completion),
                        Ok(None) => {}
                        Err(error) => {
                            warn!(%error, "USB control task failed; closing the device channel");
                            self.reply.terminate();
                            break;
                        }
                    }
                }
            }
        }

        debug!(channel_id = self.reply.channel_id, "Stopping USB device worker");
        for (_, endpoint) in core::mem::take(&mut self.endpoints) {
            drop(endpoint.commands);
            let _ = endpoint.task.await;
        }
        // A closed channel must not release its claims or budget before native controls finish.
        while let Some(completed) = self.controls.tasks.join_next().await {
            if let Err(error) = completed {
                warn!(%error, "USB control task failed during device shutdown");
            }
        }
    }

    async fn submit(
        &mut self,
        request_id: RequestId,
        envelope: Envelope,
        request: UsbRequest,
        reservation: Reservation,
    ) {
        let completion = match request {
            UsbRequest::Control { setup, data, timeout } => match urb::route_control(&setup) {
                ControlRoute::Forward => {
                    self.spawn_control(request_id, envelope, reservation, setup, data, timeout);
                    None
                }
                ControlRoute::GetDeviceDescriptor {
                    descriptor_type,
                    descriptor_index,
                    language_id,
                } => {
                    let device = self.device.clone();
                    let length = usize::from(setup.length);
                    self.controls.spawn(request_id, envelope, reservation, async move {
                        match device
                            .get_descriptor(descriptor_type, descriptor_index, language_id, timeout)
                            .await
                        {
                            Ok(mut descriptor) => {
                                descriptor.truncate(length);
                                urb::transfer_completion(envelope, usbd_status::SUCCESS, descriptor, 0)
                            }
                            Err(error) => urb::failure_completion(envelope, descriptor_error_status(error)),
                        }
                    });
                    None
                }
                ControlRoute::SetConfiguration(value) => Some(self.set_configuration(envelope, value)),
                ControlRoute::SetInterface {
                    interface_number,
                    alternate_setting,
                } => Some(
                    match self.set_alternate_setting(interface_number, alternate_setting).await {
                        Ok(()) => urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0),
                        Err(status) => urb::failure_completion(envelope, status),
                    },
                ),
                ControlRoute::ClearHalt { endpoint } => Some(match self.clear_halt(endpoint).await {
                    Ok(()) => urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0),
                    Err(status) => urb::failure_completion(envelope, status),
                }),
                ControlRoute::Reject => Some(urb::failure_completion(envelope, usbd_status::NOT_SUPPORTED)),
            },
            UsbRequest::Transfer {
                endpoint,
                direction,
                length,
                data,
            } => match self.endpoint(endpoint, direction) {
                Ok(task) => {
                    let submitted = task
                        .commands
                        .send(EndpointCommand::Submit(Submission {
                            request_id,
                            envelope,
                            length,
                            data,
                            reservation,
                        }))
                        .is_ok();
                    (!submitted).then(|| urb::failure_completion(envelope, usbd_status::DEVICE_GONE))
                }
                Err(status) => Some(urb::failure_completion(envelope, status)),
            },
            UsbRequest::OsFeatureDescriptor {
                recipient,
                interface_number,
                feature_index,
                length,
            } => {
                let setup = SetupPacket {
                    request_type: ironrdp_usb::control::RequestType::new(Direction::In, RequestKind::VENDOR, recipient),
                    // Patched with bMS_VendorCode once the Microsoft OS string descriptor is read.
                    request: 0,
                    value: u16::from_be_bytes([interface_number, 0 /* MS_PageIndex */]),
                    index: feature_index,
                    length,
                };
                // wIndex carries the feature index, so the interface comes from the request itself.
                // WinUSB only accepts an interface-directed request whose wIndex low byte is the
                // interface number of the handle, which a feature index does not satisfy in general.
                let interface = if recipient == Recipient::INTERFACE {
                    let reachable = !cfg!(windows) || feature_index.to_le_bytes()[0] == interface_number;
                    if reachable {
                        self.interfaces
                            .get(&interface_number)
                            .cloned()
                            .ok_or(usbd_status::INVALID_PARAMETER)
                    } else {
                        Err(usbd_status::NOT_SUPPORTED)
                    }
                } else {
                    self.interfaces
                        .values()
                        .next()
                        .cloned()
                        .ok_or(usbd_status::INVALID_PARAMETER)
                };
                match interface {
                    Ok(interface) => {
                        let device = self.device.clone();
                        self.controls.spawn(request_id, envelope, reservation, async move {
                            let vendor_code = device
                                .get_descriptor(
                                    urb::DESCRIPTOR_TYPE_STRING,
                                    urb::MS_OS_STRING_DESCRIPTOR_INDEX,
                                    0,
                                    urb::DEFAULT_CONTROL_TIMEOUT,
                                )
                                .await
                                .ok()
                                .and_then(|descriptor| urb::ms_os_vendor_code(&descriptor));
                            match vendor_code {
                                Some(request) => {
                                    control(
                                        &interface,
                                        envelope,
                                        SetupPacket { request, ..setup },
                                        Vec::new(),
                                        urb::DEFAULT_CONTROL_TIMEOUT,
                                    )
                                    .await
                                }
                                None => urb::failure_completion(envelope, usbd_status::STALL_PID),
                            }
                        });
                        None
                    }
                    Err(status) => Some(urb::failure_completion(envelope, status)),
                }
            }
            UsbRequest::SelectConfiguration {
                configuration_value,
                interfaces,
            } => Some(
                self.select_configuration(envelope, configuration_value, interfaces)
                    .await,
            ),
            UsbRequest::SelectInterface { interface } => Some(self.select_interface(envelope, interface).await),
            UsbRequest::Pipe { endpoint, operation } => Some(match operation {
                PipeOperation::Abort => {
                    if let Some(task) = self.endpoints.get(&endpoint) {
                        let (done, aborted) = oneshot::channel();
                        if task.commands.send(EndpointCommand::Abort(done)).is_ok() {
                            let _ = aborted.await;
                        }
                    }
                    urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0)
                }
                PipeOperation::ClearHalt => match self.clear_halt(endpoint).await {
                    Ok(()) => urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0),
                    Err(status) => urb::failure_completion(envelope, status),
                },
                PipeOperation::Ignore => urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0),
            }),
            // Unreachable: `NusbDeviceBackend::dispatch`, the only producer of `Command::Submit`, answers
            // these locally. The arms keep the match exhaustive and still complete the request should
            // that ever change.
            UsbRequest::CurrentFrameNumber | UsbRequest::Isochronous { .. } => {
                Some(urb::failure_completion(envelope, usbd_status::NOT_SUPPORTED))
            }
            UsbRequest::Reject(status) => Some(urb::failure_completion(envelope, status)),
        };

        if let Some(completion) = completion {
            self.reply.complete(request_id, completion);
        }
    }

    fn spawn_control(
        &mut self,
        request_id: RequestId,
        envelope: Envelope,
        reservation: Reservation,
        setup: SetupPacket,
        data: Vec<u8>,
        timeout: Duration,
    ) {
        let Some(interface) = self.control_interface(&setup) else {
            self.reply.complete(
                request_id,
                urb::failure_completion(envelope, usbd_status::INVALID_PARAMETER),
            );
            return;
        };

        self.controls.spawn(request_id, envelope, reservation, async move {
            control(&interface, envelope, setup, data, timeout).await
        });
    }

    /// Pick the claimed interface that carries a control request.
    ///
    /// WinUSB only accepts interface-directed requests on the handle of that interface, and on every
    /// platform any claimed interface reaches the default control pipe.
    fn control_interface(&self, setup: &SetupPacket) -> Option<nusb::Interface> {
        let [index_low, _] = setup.index.to_le_bytes();
        let recipient = setup.request_type.recipient();

        if recipient == Recipient::INTERFACE {
            return self.interfaces.get(&index_low).cloned();
        }
        if recipient == Recipient::ENDPOINT
            && let Some((number, _)) = self.endpoint_descriptor(index_low)
        {
            return self.interfaces.get(&number).cloned();
        }
        self.interfaces.values().next().cloned()
    }

    /// Find the interface and transfer type of an endpoint in the current alternate settings.
    fn endpoint_descriptor(&self, address: u8) -> Option<(u8, TransferType)> {
        self.interfaces.iter().find_map(|(number, interface)| {
            interface
                .descriptor()?
                .endpoints()
                .find(|endpoint| endpoint.address() == address)
                .map(|endpoint| (*number, endpoint.transfer_type()))
        })
    }

    /// Get the task serving an endpoint, opening the endpoint on first use.
    fn endpoint(&mut self, address: u8, direction: Direction) -> Result<&EndpointTask, UsbdStatus> {
        let address_direction = if address & 0x80 == 0 {
            Direction::Out
        } else {
            Direction::In
        };
        if address_direction != direction {
            return Err(usbd_status::INVALID_PIPE_HANDLE);
        }

        if !self.endpoints.contains_key(&address) {
            let (interface_number, transfer_type) = self
                .endpoint_descriptor(address)
                .ok_or(usbd_status::INVALID_PIPE_HANDLE)?;
            let interface = self
                .interfaces
                .get(&interface_number)
                .ok_or(usbd_status::INVALID_PIPE_HANDLE)?;
            let reply = self.reply.clone();
            let (commands, receiver) = mpsc::unbounded_channel();

            let task = match (transfer_type, direction) {
                (TransferType::Bulk, Direction::In) => tokio::spawn(run_endpoint(
                    open_endpoint::<Bulk, In>(interface, address)?,
                    receiver,
                    reply,
                )),
                (TransferType::Bulk, Direction::Out) => tokio::spawn(run_endpoint(
                    open_endpoint::<Bulk, Out>(interface, address)?,
                    receiver,
                    reply,
                )),
                (TransferType::Interrupt, Direction::In) => tokio::spawn(run_endpoint(
                    open_endpoint::<Interrupt, In>(interface, address)?,
                    receiver,
                    reply,
                )),
                (TransferType::Interrupt, Direction::Out) => tokio::spawn(run_endpoint(
                    open_endpoint::<Interrupt, Out>(interface, address)?,
                    receiver,
                    reply,
                )),
                (TransferType::Isochronous, _) => return Err(usbd_status::NOT_SUPPORTED),
                (TransferType::Control, _) => return Err(usbd_status::INVALID_PIPE_HANDLE),
            };

            trace!(endpoint = address, "Opened USB endpoint");
            self.endpoints.insert(
                address,
                EndpointTask {
                    interface: interface_number,
                    commands,
                    task,
                },
            );
        }

        self.endpoints.get(&address).ok_or(usbd_status::INVALID_PIPE_HANDLE)
    }

    /// Close the endpoints of an interface, completing their pending transfers as cancelled.
    async fn close_endpoints(&mut self, interface_number: u8) {
        let addresses: Vec<u8> = self
            .endpoints
            .iter()
            .filter(|(_, task)| task.interface == interface_number)
            .map(|(address, _)| *address)
            .collect();
        for address in addresses {
            if let Some(endpoint) = self.endpoints.remove(&address) {
                drop(endpoint.commands);
                let _ = endpoint.task.await;
            }
        }
    }

    async fn set_alternate_setting(&mut self, interface_number: u8, alternate_setting: u8) -> Result<(), UsbdStatus> {
        let Some(interface) = self.interfaces.get(&interface_number) else {
            return Err(usbd_status::INVALID_PARAMETER);
        };
        if interface.get_alt_setting() == alternate_setting {
            return Ok(());
        }

        // Endpoints change with the alternate setting, and nusb refuses the switch while any is open.
        self.close_endpoints(interface_number).await;

        let interface = self
            .interfaces
            .get(&interface_number)
            .ok_or(usbd_status::INVALID_PARAMETER)?;
        interface.set_alt_setting(alternate_setting).await.map_err(|error| {
            warn!(%error, interface_number, alternate_setting, "Failed to select USB alternate setting");
            error_status(&error)
        })
    }

    async fn clear_halt(&mut self, endpoint: u8) -> Result<(), UsbdStatus> {
        // The default control pipe clears its stall on the next SETUP packet.
        if endpoint & 0x0f == 0 {
            return Ok(());
        }

        let direction = if endpoint & 0x80 == 0 {
            Direction::Out
        } else {
            Direction::In
        };
        let task = self.endpoint(endpoint, direction)?;
        let (done, cleared) = oneshot::channel();
        task.commands
            .send(EndpointCommand::ClearHalt(done))
            .map_err(|_| usbd_status::DEVICE_GONE)?;
        cleared.await.unwrap_or(Err(usbd_status::DEVICE_GONE))
    }

    fn set_configuration(&self, envelope: Envelope, value: u8) -> CompletionData {
        let active = self
            .device
            .active_configuration()
            .map(|config| config.configuration_value())
            .ok();
        if active == Some(value) {
            urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0)
        } else {
            // Changing the configuration would invalidate every claimed interface, and WinUSB does not
            // allow it at all. Devices with a single configuration never need it.
            debug!(value, ?active, "Rejecting a USB configuration change");
            urb::failure_completion(envelope, usbd_status::NOT_SUPPORTED)
        }
    }

    async fn select_configuration(
        &mut self,
        envelope: Envelope,
        configuration_value: Option<u8>,
        interfaces: Vec<TsUsbdInterfaceInfo>,
    ) -> CompletionData {
        let Some(configuration_value) = configuration_value else {
            // Unconfiguring happens when the server-side driver unloads. The device stays configured on
            // the client, but its pipes are gone: stop their transfers.
            let interface_numbers: Vec<u8> = self.interfaces.keys().copied().collect();
            for interface_number in interface_numbers {
                self.close_endpoints(interface_number).await;
            }
            return urb::payload_completion(
                envelope,
                TsUrbResultPayload::SelectConfig(TsUrbSelectConfigResult {
                    config_handle: 0,
                    interface: Vec::new(),
                }),
            );
        };

        let active = self
            .device
            .active_configuration()
            .map(|config| config.configuration_value())
            .ok();
        if active != Some(configuration_value) {
            return self.set_configuration(envelope, configuration_value);
        }

        for interface in &interfaces {
            if let Err(status) = self
                .set_alternate_setting(interface.interface_number, interface.alternate_setting)
                .await
            {
                return urb::failure_completion(envelope, status);
            }
        }

        self.selection_completion(envelope, |config| {
            urb::select_configuration_result(config, &interfaces).map(TsUrbResultPayload::SelectConfig)
        })
    }

    async fn select_interface(&mut self, envelope: Envelope, interface: TsUsbdInterfaceInfo) -> CompletionData {
        if let Err(status) = self
            .set_alternate_setting(interface.interface_number, interface.alternate_setting)
            .await
        {
            return urb::failure_completion(envelope, status);
        }

        self.selection_completion(envelope, |config| {
            urb::select_interface_result(config, &interface).map(TsUrbResultPayload::SelectIface)
        })
    }

    /// Complete a selection request with the result `describe` builds from the active configuration
    /// descriptor.
    fn selection_completion(
        &self,
        envelope: Envelope,
        describe: impl FnOnce(&[u8]) -> Result<TsUrbResultPayload, UsbdStatus>,
    ) -> CompletionData {
        let result = self
            .device
            .active_configuration()
            .map_err(|_| usbd_status::REQUEST_FAILED)
            .and_then(|config| describe(config.as_bytes()));
        match result {
            Ok(payload) => urb::payload_completion(envelope, payload),
            Err(status) => urb::failure_completion(envelope, status),
        }
    }
}

fn open_endpoint<T: BulkOrInterrupt, D: EndpointDirection>(
    interface: &nusb::Interface,
    address: u8,
) -> Result<Endpoint<T, D>, UsbdStatus> {
    interface.endpoint::<T, D>(address).map_err(|error| {
        warn!(%error, endpoint = address, "Failed to open USB endpoint");
        error_status(&error)
    })
}

/// Perform a control transfer through `interface`.
async fn control(
    interface: &nusb::Interface,
    envelope: Envelope,
    setup: SetupPacket,
    data: Vec<u8>,
    timeout: Duration,
) -> CompletionData {
    let control_type = match setup.request_type.kind() {
        RequestKind::STANDARD => ControlType::Standard,
        RequestKind::CLASS => ControlType::Class,
        RequestKind::VENDOR => ControlType::Vendor,
        _ => return urb::failure_completion(envelope, usbd_status::INVALID_PARAMETER),
    };
    let recipient = match setup.request_type.recipient() {
        Recipient::DEVICE => NusbRecipient::Device,
        Recipient::INTERFACE => NusbRecipient::Interface,
        Recipient::ENDPOINT => NusbRecipient::Endpoint,
        Recipient::OTHER => NusbRecipient::Other,
        _ => return urb::failure_completion(envelope, usbd_status::INVALID_PARAMETER),
    };

    match setup.request_type.direction() {
        Direction::In => {
            let result = interface
                .control_in(
                    ControlIn {
                        control_type,
                        recipient,
                        request: setup.request,
                        value: setup.value,
                        index: setup.index,
                        length: setup.length,
                    },
                    timeout,
                )
                .await;
            match result {
                Ok(data) => urb::transfer_completion(envelope, usbd_status::SUCCESS, data, 0),
                Err(error) => urb::failure_completion(envelope, transfer_error_status(error)),
            }
        }
        Direction::Out => {
            let written = u32::from(setup.length);
            let result = interface
                .control_out(
                    ControlOut {
                        control_type,
                        recipient,
                        request: setup.request,
                        value: setup.value,
                        index: setup.index,
                        data: &data,
                    },
                    timeout,
                )
                .await;
            match result {
                Ok(()) => urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), written),
                Err(error) => urb::failure_completion(envelope, transfer_error_status(error)),
            }
        }
    }
}

struct Submission {
    request_id: RequestId,
    envelope: Envelope,
    length: u32,
    data: Vec<u8>,
    reservation: Reservation,
}

enum EndpointCommand {
    Submit(Submission),
    Cancel(RequestId),
    /// Cancel every pending transfer, as `URB_FUNCTION_ABORT_PIPE` requires.
    Abort(oneshot::Sender<()>),
    ClearHalt(oneshot::Sender<Result<(), UsbdStatus>>),
}

struct InFlight {
    /// Position of the transfer among those submitted to the endpoint, kept when it resumes.
    seq: u64,
    request_id: RequestId,
    envelope: Envelope,
    length: u32,
    /// The server cancelled this request: complete it with whatever the cancellation left, and
    /// never submit it again.
    cancelled: bool,
    /// Data moved before nusb cancelled the transfer along with another one, while the rest of the
    /// transfer runs again: the bytes an IN transfer received, or the count an OUT transfer wrote.
    received: Vec<u8>,
    written: usize,
    _reservation: Reservation,
}

impl InFlight {
    fn into_cancelled(self) -> CompletionData {
        urb::transfer_completion(
            self.envelope,
            usbd_status::CANCELED,
            self.received,
            u32::try_from(self.written).unwrap_or(u32::MAX),
        )
    }
}

/// Transfers of one endpoint, in submission order.
struct EndpointQueue {
    in_flight: VecDeque<InFlight>,
    /// Set after a cancellation, until every transfer cancelled with it has returned.
    draining: bool,
    /// Transfers cancelled only because nusb cancels a whole endpoint, to submit again in order.
    retry: VecDeque<(InFlight, Buffer)>,
    /// Requests received while draining.
    held: VecDeque<Submission>,
    /// Finished transfers waiting for an earlier one to finish, by position.
    finished: BTreeMap<u64, Outcome>,
    next_seq: u64,
}

/// How a finished transfer is reported.
enum Outcome {
    Complete(RequestId, CompletionData),
    /// An IN transfer overran its buffer, which the protocol only lets the client report by closing
    /// the device channel.
    Terminate,
}

impl EndpointQueue {
    fn new() -> Self {
        Self {
            in_flight: VecDeque::new(),
            draining: false,
            retry: VecDeque::new(),
            held: VecDeque::new(),
            finished: BTreeMap::new(),
            next_seq: 0,
        }
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        seq
    }

    /// The position of the oldest transfer still running or waiting to resume.
    fn oldest_unfinished(&self) -> Option<u64> {
        let in_flight = self.in_flight.front().map(|entry| entry.seq);
        let retry = self.retry.front().map(|(entry, _)| entry.seq);
        match (in_flight, retry) {
            (Some(in_flight), Some(retry)) => Some(in_flight.min(retry)),
            (in_flight, retry) => in_flight.or(retry),
        }
    }

    /// Report a finished transfer once every transfer submitted before it finished too.
    ///
    /// USB completes the transfers of a pipe in order. A transfer that nusb cancelled along with
    /// another one runs again later, and those after it must not be reported first.
    fn finish(&mut self, reply: &Reply, seq: u64, outcome: Outcome) {
        self.finished.insert(seq, outcome);
        let oldest = self.oldest_unfinished();
        while let Some(entry) = self.finished.first_entry() {
            if oldest.is_some_and(|oldest| oldest < *entry.key()) {
                break;
            }
            match entry.remove() {
                Outcome::Complete(request_id, completion) => reply.complete(request_id, completion),
                Outcome::Terminate => reply.terminate(),
            }
        }
    }
}

/// Serve the transfers of one bulk or interrupt endpoint.
///
/// nusb can only cancel every transfer of an endpoint at once. When the server cancels one request,
/// the other transfers that were cancelled along with it resume where they stopped, submitted again
/// in their original order.
async fn run_endpoint<T: BulkOrInterrupt, D: EndpointDirection>(
    mut endpoint: Endpoint<T, D>,
    mut commands: mpsc::UnboundedReceiver<EndpointCommand>,
    reply: Reply,
) {
    // UNTESTED: nusb must return completions in submission order to match them to the right URB.
    // Verify cancellation and resume ordering on hardware when changing endpoint handling.
    let mut queue = EndpointQueue::new();

    loop {
        let pending = endpoint.pending() != 0;
        tokio::select! {
            command = commands.recv() => match command {
                None => break,
                Some(EndpointCommand::Submit(submission)) => {
                    if queue.draining {
                        queue.held.push_back(submission);
                    } else {
                        submit(&mut endpoint, &mut queue, &reply, submission);
                    }
                }
                Some(EndpointCommand::Cancel(request_id)) => {
                    if let Some(entry) = queue.in_flight.iter_mut().find(|entry| entry.request_id == request_id) {
                        entry.cancelled = true;
                        queue.draining = true;
                        endpoint.cancel_all();
                    } else {
                        // Waiting to submit or resume: preserve any progress without touching the device.
                        let held = queue.held.iter().position(|submission| submission.request_id == request_id);
                        let retried = queue.retry.iter().position(|(entry, _)| entry.request_id == request_id);
                        let completion = held
                            .and_then(|index| queue.held.remove(index))
                            .map(|submission| urb::failure_completion(submission.envelope, usbd_status::CANCELED))
                            .or_else(|| retried.and_then(|index| queue.retry.remove(index)).map(|(entry, _)| entry.into_cancelled()));
                        if let Some(completion) = completion {
                            reply.complete(request_id, completion);
                        }
                    }
                }
                Some(EndpointCommand::Abort(done)) => {
                    cancel_all(&mut endpoint, &mut queue, &reply).await;
                    let _ = done.send(());
                }
                Some(EndpointCommand::ClearHalt(done)) => {
                    // Clearing the halt with transfers pending is not allowed; Windows aborts the pipe
                    // before resetting it anyway.
                    cancel_all(&mut endpoint, &mut queue, &reply).await;
                    let result = endpoint.clear_halt().await.map_err(|error| {
                        debug!(%error, endpoint = endpoint.endpoint_address(), "Failed to clear USB endpoint halt");
                        error_status(&error)
                    });
                    let _ = done.send(result);
                }
            },
            completion = endpoint.next_complete(), if pending => {
                let Some(mut entry) = queue.in_flight.pop_front() else {
                    continue;
                };
                let cancelled = completion.status == Err(TransferError::Cancelled);
                let seq = entry.seq;
                if cancelled && queue.draining && !entry.cancelled {
                    match resume::<D>(endpoint.max_packet_size(), &mut entry, completion) {
                        Resume::Submit(buffer) => queue.retry.push_back((entry, buffer)),
                        Resume::Finished(completion) => {
                            let outcome = transfer_outcome::<D>(entry, completion);
                            queue.finish(&reply, seq, outcome);
                        }
                        Resume::Invalid => {
                            let completion = urb::failure_completion(entry.envelope, usbd_status::INVALID_PARAMETER);
                            queue.finish(&reply, seq, Outcome::Complete(entry.request_id, completion));
                        }
                    }
                } else {
                    let outcome = transfer_outcome::<D>(entry, completion);
                    queue.finish(&reply, seq, outcome);
                }

                if queue.draining && queue.in_flight.is_empty() {
                    queue.draining = false;
                    while let Some((entry, buffer)) = queue.retry.pop_front() {
                        endpoint.submit(buffer);
                        queue.in_flight.push_back(entry);
                    }
                    while let Some(submission) = queue.held.pop_front() {
                        submit(&mut endpoint, &mut queue, &reply, submission);
                    }
                }
            }
        }
    }

    cancel_all(&mut endpoint, &mut queue, &reply).await;
}

fn submit<T: BulkOrInterrupt, D: EndpointDirection>(
    endpoint: &mut Endpoint<T, D>,
    queue: &mut EndpointQueue,
    reply: &Reply,
    submission: Submission,
) {
    let Submission {
        request_id,
        envelope,
        length,
        data,
        reservation,
    } = submission;
    let seq = queue.next_seq();

    let buffer = if D::DIR == nusb::transfer::Direction::In {
        // Completed without touching the device, still after the transfers submitted before.
        if length == 0 {
            let completion = urb::transfer_completion(envelope, usbd_status::SUCCESS, Vec::new(), 0);
            queue.finish(reply, seq, Outcome::Complete(request_id, completion));
            return;
        }
        let Some(buffer_length) = in_buffer_length(endpoint.max_packet_size(), length) else {
            let completion = urb::failure_completion(envelope, usbd_status::INVALID_PARAMETER);
            queue.finish(reply, seq, Outcome::Complete(request_id, completion));
            return;
        };
        Buffer::new(buffer_length)
    } else {
        Buffer::from(data)
    };

    endpoint.submit(buffer);
    queue.in_flight.push_back(InFlight {
        seq,
        request_id,
        envelope,
        length,
        cancelled: false,
        received: Vec::new(),
        written: 0,
        _reservation: reservation,
    });
}

enum Resume {
    /// Submit this buffer for the rest of the transfer.
    Submit(Buffer),
    /// The transfer already finished: complete it.
    Finished(Completion),
    Invalid,
}

/// Resume a transfer that nusb cancelled only because it cancels a whole endpoint at once.
///
/// The data that moved is kept and the rest of the transfer is submitted again, as if it had never
/// been cancelled. An IN transfer that received a short packet or filled its buffer, or an OUT
/// transfer that wrote everything, finished instead.
fn resume<D: EndpointDirection>(max_packet_size: usize, entry: &mut InFlight, completion: Completion) -> Resume {
    let finished = |buffer| {
        Resume::Finished(Completion {
            buffer,
            actual_len: 0,
            status: Ok(()),
        })
    };
    let length = usize::try_from(entry.length).unwrap_or(usize::MAX);

    if D::DIR == nusb::transfer::Direction::In {
        if completion.actual_len == 0 && entry.received.is_empty() {
            // Nothing moved: nusb keeps the requested length, so the same buffer runs again.
            return Resume::Submit(completion.buffer);
        }
        let short = !completion.actual_len.is_multiple_of(max_packet_size.max(1));
        entry.received.extend_from_slice(&completion.buffer);
        let remaining = length.saturating_sub(entry.received.len());
        if short || remaining == 0 {
            return finished(Buffer::new(0));
        }
        match u32::try_from(remaining)
            .ok()
            .and_then(|remaining| in_buffer_length(max_packet_size, remaining))
        {
            Some(buffer_length) => Resume::Submit(Buffer::new(buffer_length)),
            None => Resume::Invalid,
        }
    } else {
        // nusb returns an OUT buffer with its whole payload.
        let mut data = completion.buffer.into_vec();
        let written = completion.actual_len.min(data.len());
        data.drain(..written);
        entry.written = entry.written.saturating_add(written);
        if data.is_empty() {
            finished(Buffer::new(0))
        } else {
            Resume::Submit(Buffer::from(data))
        }
    }
}

/// nusb requires IN transfers to be a multiple of the maximum packet size.
///
/// The buffer is rounded up to whole packets. A real host controller would also see the device
/// overrun a buffer of the requested length, so data beyond it terminates the device channel (see
/// [`transfer_completion`]). Returns `None` for a length above [`urb::MAX_TRANSFER_LENGTH`], which
/// [`urb::translate_in`] already rejects.
fn in_buffer_length(max_packet_size: usize, length: u32) -> Option<usize> {
    if urb::MAX_TRANSFER_LENGTH < length {
        return None;
    }
    let length = usize::try_from(length).ok()?;
    let max_packet_size = max_packet_size.max(1);
    length.div_ceil(max_packet_size).checked_mul(max_packet_size)
}

/// Cancel every pending and queued transfer, completing them as cancelled.
async fn cancel_all<T: BulkOrInterrupt, D: EndpointDirection>(
    endpoint: &mut Endpoint<T, D>,
    queue: &mut EndpointQueue,
    reply: &Reply,
) {
    if endpoint.pending() != 0 {
        endpoint.cancel_all();
    }
    while endpoint.pending() != 0 {
        let completion = endpoint.next_complete().await;
        let Some(entry) = queue.in_flight.pop_front() else {
            continue;
        };
        let seq = entry.seq;
        let outcome = transfer_outcome::<D>(entry, completion);
        queue.finish(reply, seq, outcome);
    }
    queue.in_flight.clear();
    queue.draining = false;

    while let Some((entry, _)) = queue.retry.pop_front() {
        let (seq, request_id) = (entry.seq, entry.request_id);
        queue.finish(reply, seq, Outcome::Complete(request_id, entry.into_cancelled()));
    }
    // Nothing is left running: report whatever still waited for an earlier transfer.
    while let Some((_, outcome)) = queue.finished.pop_first() {
        match outcome {
            Outcome::Complete(request_id, completion) => reply.complete(request_id, completion),
            Outcome::Terminate => reply.terminate(),
        }
    }
    for submission in queue.held.drain(..) {
        reply.complete(
            submission.request_id,
            urb::failure_completion(submission.envelope, usbd_status::CANCELED),
        );
    }
}

/// Report a bulk or interrupt transfer, or terminate the device channel on an IN overrun.
fn transfer_outcome<D: EndpointDirection>(entry: InFlight, completion: Completion) -> Outcome {
    let request_id = entry.request_id;
    match transfer_completion::<D>(entry, completion) {
        Some(completion) => Outcome::Complete(request_id, completion),
        None => Outcome::Terminate,
    }
}

/// Returns `None` when an IN transfer received more data than the server asked for.
///
/// A cancelled transfer completes as cancelled, even when some data moved: the server cancelled it,
/// or aborted its pipe. Transfers cancelled only along with another one resume instead (see
/// [`resume`]).
fn transfer_completion<D: EndpointDirection>(mut entry: InFlight, completion: Completion) -> Option<CompletionData> {
    let status = completion
        .status
        .map_or_else(transfer_error_status, |()| usbd_status::SUCCESS);
    Some(if D::DIR == nusb::transfer::Direction::In {
        // Reuse nusb's buffer unless the transfer resumed after a cancellation.
        let data = if entry.received.is_empty() {
            completion.buffer.into_vec()
        } else {
            let mut data = core::mem::take(&mut entry.received);
            data.extend_from_slice(&completion.buffer);
            data
        };
        if usize::try_from(entry.length).is_ok_and(|length| length < data.len()) {
            // The buffer was rounded up to whole packets and the device filled more than the server
            // asked for. MS-RDPEUSB 3.3.5.3.6 requires terminating the device channel in this case.
            warn!(
                request_id = entry.request_id,
                requested = entry.length,
                received = data.len(),
                "USB IN transfer overran its buffer; closing the device channel"
            );
            return None;
        }
        urb::transfer_completion(entry.envelope, status, data, 0)
    } else {
        urb::transfer_completion(
            entry.envelope,
            status,
            Vec::new(),
            u32::try_from(entry.written.saturating_add(completion.actual_len)).unwrap_or(u32::MAX),
        )
    })
}

fn transfer_error_status(error: TransferError) -> UsbdStatus {
    match error {
        TransferError::Cancelled => usbd_status::CANCELED,
        TransferError::Stall => usbd_status::STALL_PID,
        TransferError::Disconnected => usbd_status::DEVICE_GONE,
        TransferError::Fault => usbd_status::DEV_NOT_RESPONDING,
        TransferError::InvalidArgument => usbd_status::INVALID_PARAMETER,
        TransferError::Unknown(_) => usbd_status::REQUEST_FAILED,
    }
}

fn descriptor_error_status(error: nusb::GetDescriptorError) -> UsbdStatus {
    match error {
        nusb::GetDescriptorError::Transfer(error) => transfer_error_status(error),
        nusb::GetDescriptorError::InvalidDescriptor => usbd_status::REQUEST_FAILED,
    }
}

fn error_status(error: &nusb::Error) -> UsbdStatus {
    match error.kind() {
        nusb::ErrorKind::Disconnected => usbd_status::DEVICE_GONE,
        nusb::ErrorKind::Unsupported => usbd_status::NOT_SUPPORTED,
        nusb::ErrorKind::NotFound => usbd_status::INVALID_PARAMETER,
        _ => usbd_status::REQUEST_FAILED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: reference microsoft/win32metadata@6d6617a8eee5:generation/WinSDK/RecompiledIdlHeaders/shared/usb.h
    const CANCELED: UsbdStatus = 0xc001_0000;
    const STALL_PID: UsbdStatus = 0xc000_0004;

    fn budget(bytes: usize) -> Arc<TransferBudget> {
        Arc::new(TransferBudget {
            available: AtomicUsize::new(bytes),
        })
    }

    fn in_flight(length: u32) -> InFlight {
        InFlight {
            seq: 0,
            request_id: 7,
            envelope: Envelope::In {
                output_buffer_size: length,
            },
            length,
            cancelled: true,
            received: Vec::new(),
            written: 0,
            _reservation: budget(0).reserve(0).expect("zero bytes always fit"),
        }
    }

    fn completion(data: &[u8], status: Result<(), TransferError>) -> Completion {
        Completion {
            buffer: Buffer::from(data.to_vec()),
            actual_len: data.len(),
            status,
        }
    }

    #[test]
    fn transfers_of_an_endpoint_finish_in_submission_order() {
        let (sender, received) = std::sync::mpsc::channel();
        let sink: UrbdrcEventSink = Arc::new(move |event| sender.send(event).expect("receiver is alive"));
        let token = Arc::new(());
        let reply = Reply::new(3, &token, sink);
        let finished = || -> Vec<RequestId> {
            received
                .try_iter()
                .filter_map(|event| match event.into_current() {
                    Some(UrbdrcEventKind::Completion { request_id, .. }) => Some(request_id),
                    _ => None,
                })
                .collect()
        };
        let ok = || urb::transfer_completion(Envelope::Out, usbd_status::SUCCESS, Vec::new(), 0);

        let mut queue = EndpointQueue::new();
        let mut first = in_flight(64);
        first.seq = queue.next_seq();
        first.request_id = 1;
        let second_seq = queue.next_seq();

        // The first transfer was cancelled along with another one and waits to run again: the second,
        // which finished meanwhile, is held back.
        queue.retry.push_back((first, Buffer::new(64)));
        queue.finish(&reply, second_seq, Outcome::Complete(2, ok()));
        assert!(finished().is_empty());

        // Once the first one finishes, both are reported, in order.
        let (first, _) = queue.retry.pop_front().expect("queued for retry");
        queue.finish(&reply, first.seq, Outcome::Complete(1, ok()));
        assert_eq!(finished(), [1, 2]);

        // With nothing earlier pending, a transfer is reported right away.
        let third_seq = queue.next_seq();
        queue.finish(&reply, third_seq, Outcome::Complete(3, ok()));
        assert_eq!(finished(), [3]);
    }

    #[test]
    fn budget_bounds_bytes_in_flight() {
        let budget = budget(100);
        let first = budget.reserve(60).expect("fits");
        assert!(budget.reserve(41).is_none());
        let second = budget.reserve(40).expect("fits exactly");
        drop(first);
        assert!(budget.reserve(60).is_some(), "released bytes are reusable");
        drop(second);
        assert_eq!(budget.available.load(Ordering::Acquire), 100);
    }

    #[test]
    fn queued_unplug_close_does_not_survive_its_device_channel() {
        // oracle: spec MS-RDPEDYC §3.3.5.2 permits channel ID reuse; stale events must not affect its new owner.
        let (sender, received) = std::sync::mpsc::channel();
        let sink: UrbdrcEventSink = Arc::new(move |event| sender.send(event).expect("receiver is alive"));
        let manager_token = Arc::new(());
        let manager = EventOrigin::new(&manager_token, Arc::clone(&sink));
        let old_token = Arc::new(());
        let old = Reply::new(7, &old_token, Arc::clone(&sink));
        old.terminate();
        drop(old_token);

        let replacement_token = Arc::new(());
        let replacement = Reply::new(7, &replacement_token, sink);
        assert!(!old.same_channel(&replacement));
        assert!(received.try_recv().expect("queued unplug").into_current().is_none());

        replacement.terminate();
        assert!(matches!(
            received.try_recv().expect("replacement close").into_current(),
            Some(UrbdrcEventKind::CloseChannel { channel_id: 7 })
        ));
        manager.send(UrbdrcEventKind::CloseChannel { channel_id: 8 });
        assert!(
            received
                .try_recv()
                .expect("manager still lives")
                .into_current()
                .is_some()
        );
    }

    #[tokio::test]
    async fn cancelled_controls_remain_charged_until_native_completion() {
        // oracle: reference nusb 0.2.8 Pending::drop retains native controls after their future is dropped.
        // oracle: spec crates/ironrdp-rdpeusb-native/README.md §Limitations requires charging in-flight controls.
        let budget = budget(100);
        let envelope = Envelope::In { output_buffer_size: 1 };
        let (started, running) = oneshot::channel();
        let (finish, finished) = oneshot::channel();
        let mut controls = ControlRequests::default();
        controls.spawn(7, envelope, budget.reserve(100).expect("fits"), async move {
            started.send(()).expect("test is waiting");
            finished.await.expect("test completes the native operation");
            urb::transfer_completion(envelope, usbd_status::SUCCESS, vec![1], 0)
        });
        running.await.expect("native operation started");

        let Some(CompletionData::TransferIn(cancelled)) = controls.cancel(7) else {
            panic!("expected immediate logical cancellation");
        };
        assert_eq!(cancelled.ts_urb_result.header.usbd_status, CANCELED);
        tokio::task::yield_now().await;
        assert!(
            budget.reserve(1).is_none(),
            "the physical operation still owns its buffer"
        );

        finish.send(()).expect("native operation was not aborted");
        assert!(controls.next_completed().await.expect("task succeeds").is_none());
        assert_eq!(budget.available.load(Ordering::Acquire), 100);
    }

    #[tokio::test]
    async fn late_control_completion_cannot_complete_a_reused_request_id() {
        // oracle: spec MS-RDPEUSB §3.3.5.4 requires a completion for its corresponding request only.
        let budget = budget(2);
        let envelope = Envelope::In { output_buffer_size: 1 };
        let mut controls = ControlRequests::default();
        let (finish_old, old_finished) = oneshot::channel();
        let (old_done, old_completed) = oneshot::channel();
        controls.spawn(7, envelope, budget.reserve(1).expect("fits"), async move {
            old_finished.await.expect("test completes old request");
            let completion = urb::transfer_completion(envelope, usbd_status::SUCCESS, vec![1], 0);
            old_done.send(()).expect("test is waiting for old completion");
            completion
        });
        assert!(controls.cancel(7).is_some());
        let (finish_new, new_finished) = oneshot::channel();
        controls.spawn(7, envelope, budget.reserve(1).expect("fits"), async move {
            new_finished.await.expect("test completes new request");
            urb::transfer_completion(envelope, usbd_status::SUCCESS, vec![2], 0)
        });

        finish_old.send(()).expect("old operation remains alive");
        old_completed.await.expect("old operation completed first");
        finish_new.send(()).expect("new operation remains alive");
        let (request_id, CompletionData::TransferIn(done)) = controls
            .next_completed()
            .await
            .expect("task succeeds")
            .expect("new request completes")
        else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(request_id, 7);
        assert_eq!(done.output_buffer, [2]);
        assert!(controls.next_completed().await.expect("tasks succeed").is_none());
        assert_eq!(budget.available.load(Ordering::Acquire), 2);
    }

    #[test]
    fn cancelled_transfers_still_complete() {
        // oracle: spec MS-RDPEUSB §3.3.5.4.2 returns the operation result and all data read.
        for data in [&[][..], &[1, 2, 3]] {
            let Some(CompletionData::TransferIn(cancelled)) =
                transfer_completion::<In>(in_flight(64), completion(data, Err(TransferError::Cancelled)))
            else {
                panic!("expected a transfer-in completion");
            };
            assert_eq!(cancelled.ts_urb_result.header.usbd_status, CANCELED);
            assert_eq!(cancelled.output_buffer, data);
        }
    }

    #[test]
    fn stalled_in_transfer_keeps_resumed_data() {
        // oracle: spec MS-RDPEUSB §3.3.5.4.2 returns all data read, including before a failure.
        let mut entry = in_flight(6);
        entry.received = vec![1, 2];
        let Some(CompletionData::TransferIn(done)) =
            transfer_completion::<In>(entry, completion(&[3, 4], Err(TransferError::Stall)))
        else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, STALL_PID);
        assert_eq!(done.output_buffer, [1, 2, 3, 4]);
    }

    #[test]
    fn cancelled_out_transfer_reports_partial_progress() {
        // oracle: spec MS-RDPEUSB §2.2.7.3 reports the bytes actually sent, not the requested length.
        let mut entry = in_flight(6);
        entry.envelope = Envelope::Out;
        entry.written = 2;
        let partial = Completion {
            buffer: Buffer::from(vec![3, 4, 5, 6]),
            actual_len: 2,
            status: Err(TransferError::Cancelled),
        };
        let Some(CompletionData::TransferOut(done)) = transfer_completion::<Out>(entry, partial) else {
            panic!("expected a transfer-out completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, CANCELED);
        assert_eq!(done.output_buffer_size, 4);
    }

    #[test]
    fn cancelled_in_retry_keeps_received_data() {
        // oracle: spec MS-RDPEUSB §3.3.5.4.2 returns data read even when cancelled before resubmission.
        let mut entry = in_flight(6);
        entry.received = vec![1, 2];
        let CompletionData::TransferIn(done) = entry.into_cancelled() else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, CANCELED);
        assert_eq!(done.output_buffer, [1, 2]);
    }

    #[test]
    fn cancelled_out_retry_keeps_written_count() {
        // oracle: spec MS-RDPEUSB §2.2.7.3 reports bytes already sent, even before resubmission.
        let mut entry = in_flight(6);
        entry.envelope = Envelope::Out;
        entry.written = 2;
        let CompletionData::TransferOut(done) = entry.into_cancelled() else {
            panic!("expected a transfer-out completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, CANCELED);
        assert_eq!(done.output_buffer_size, 2);
    }

    #[test]
    fn collateral_in_cancellation_resumes() {
        let mut entry = in_flight(256);

        // Nothing moved: the same buffer runs again.
        let untouched = Completion {
            buffer: Buffer::new(256),
            actual_len: 0,
            status: Err(TransferError::Cancelled),
        };
        let Resume::Submit(buffer) = resume::<In>(64, &mut entry, untouched) else {
            panic!("expected the transfer to resume");
        };
        assert_eq!(buffer.requested_len(), 256);
        assert!(entry.received.is_empty());

        // Whole packets moved: the rest of the transfer runs again.
        let Resume::Submit(buffer) = resume::<In>(64, &mut entry, completion(&[1; 64], Err(TransferError::Cancelled)))
        else {
            panic!("expected the transfer to resume");
        };
        assert_eq!(buffer.requested_len(), 192);

        // The resumed transfer completes with all of the data.
        let Some(CompletionData::TransferIn(done)) = transfer_completion::<In>(entry, completion(&[2; 100], Ok(())))
        else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, usbd_status::SUCCESS);
        assert_eq!(done.output_buffer.len(), 164);
        assert_eq!(done.output_buffer.first(), Some(&1));
        assert_eq!(done.output_buffer.last(), Some(&2));

        // A short packet ended the transfer before the cancellation.
        let mut entry = in_flight(256);
        let Resume::Finished(finished) =
            resume::<In>(64, &mut entry, completion(&[3; 10], Err(TransferError::Cancelled)))
        else {
            panic!("expected the transfer to finish");
        };
        let Some(CompletionData::TransferIn(done)) = transfer_completion::<In>(entry, finished) else {
            panic!("expected a transfer-in completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, usbd_status::SUCCESS);
        assert_eq!(done.output_buffer, [3; 10]);
    }

    #[test]
    fn collateral_out_cancellation_resumes() {
        let mut entry = in_flight(6);
        entry.envelope = Envelope::Out;

        // nusb returns the whole payload, of which the first bytes were written.
        let partial = Completion {
            buffer: Buffer::from(vec![1, 2, 3, 4, 5, 6]),
            actual_len: 4,
            status: Err(TransferError::Cancelled),
        };
        let Resume::Submit(buffer) = resume::<Out>(64, &mut entry, partial) else {
            panic!("expected the transfer to resume");
        };
        assert_eq!(buffer.into_vec(), [5, 6]);

        let rest = Completion {
            buffer: Buffer::from(vec![5, 6]),
            actual_len: 2,
            status: Ok(()),
        };
        let Some(CompletionData::TransferOut(done)) = transfer_completion::<Out>(entry, rest) else {
            panic!("expected a transfer-out completion");
        };
        assert_eq!(done.ts_urb_result.header.usbd_status, usbd_status::SUCCESS);
        assert_eq!(done.output_buffer_size, 6);
    }

    #[test]
    fn overrun_terminates_instead_of_completing() {
        assert!(transfer_completion::<In>(in_flight(2), completion(&[1, 2, 3], Ok(()))).is_none());
        assert!(transfer_completion::<In>(in_flight(3), completion(&[1, 2, 3], Ok(()))).is_some());
    }

    #[test]
    fn bus_numbers_follow_the_present_controllers() {
        assert_eq!(bus_number("3"), 3);
        assert_eq!(bus_number("003"), 3);

        // Windows host controllers are numbered by sorted instance ID, from 1.
        let present = || {
            vec![
                r"PCI\VEN_8086&DEV_A0ED&B".to_owned(),
                r"PCI\VEN_8086&DEV_A0ED&A".to_owned(),
            ]
        };
        assert_eq!(controller_number(r"PCI\VEN_8086&DEV_A0ED&A", present()), 1);
        assert_eq!(controller_number(r"PCI\VEN_8086&DEV_A0ED&B", present()), 2);
        // A controller missing from the listing, like one that just appeared, still gets a number.
        assert_eq!(controller_number(r"PCI\VEN_8086&DEV_A0ED&C", present()), 3);
    }
}

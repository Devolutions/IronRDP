//! A redirected USB device backed by nusb.
//!
//! Each redirected device owns a worker thread running a single-threaded Tokio runtime. The RDPEUSB
//! state machine calls [`NusbDeviceBackend`] from the session loop, which must not block: requests that
//! need device I/O are queued to the worker, and their completions come back through the event sink.
//!
//! Inside the worker, every bulk or interrupt endpoint gets its own task that keeps its transfers in
//! submission order, as the host controller does. Control transfers run as independent tasks.

use core::fmt;
use core::time::Duration;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
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
use tokio::task::{AbortHandle, JoinHandle};
use tracing::{debug, trace, warn};

use crate::filter::UsbDeviceIdentity;
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
    pub(crate) fn open(info: &nusb::DeviceInfo) -> Result<Self, OpenError> {
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
        let identity = identity(info);

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
                speed: match device.speed() {
                    Some(nusb::Speed::Low) => UsbConnectionSpeed::Low,
                    Some(nusb::Speed::Full) => UsbConnectionSpeed::Full,
                    Some(nusb::Speed::High) => UsbConnectionSpeed::High,
                    Some(nusb::Speed::Super) => UsbConnectionSpeed::Super,
                    Some(nusb::Speed::SuperPlus) => UsbConnectionSpeed::SuperPlus,
                    _ => UsbConnectionSpeed::Unknown,
                },
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

    pub(crate) fn vendor_product(&self) -> (u16, u16) {
        (self.info.descriptor.vendor_id, self.info.descriptor.product_id)
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
/// Linux and macOS identify buses with numbers. Windows uses host controller instance IDs, which are
/// numbered from 1 in sorted order: every controller gets a distinct number, and the numbers are the
/// same in every process as long as the set of controllers does not change. A controller that
/// appears later gets the next number without renumbering the others.
fn bus_number(bus_id: &str) -> u8 {
    static KNOWN_BUSES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    if let Ok(number) = bus_id.parse::<u8>() {
        return number;
    }

    let mut known = KNOWN_BUSES.lock().unwrap_or_else(PoisonError::into_inner);
    if !known.iter().any(|known| known == bus_id) {
        let mut present: Vec<String> = nusb::list_buses()
            .wait()
            .map(|buses| buses.map(|bus| bus.bus_id().to_owned()).collect())
            .unwrap_or_default();
        present.push(bus_id.to_owned());
        present.sort();
        present.dedup();
        for id in present {
            if !known.contains(&id) {
                known.push(id);
            }
        }
    }

    let index = known.iter().position(|known| known == bus_id).unwrap_or(0);
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

/// Completion routing shared by the worker tasks.
#[derive(Clone)]
struct Reply {
    channel_id: u32,
    events: EventOrigin,
}

impl Reply {
    fn complete(&self, request_id: RequestId, completion: CompletionData) {
        self.events.send(UrbdrcEventKind::Completion {
            channel_id: self.channel_id,
            request_id,
            completion,
        });
    }

    /// Close the device channel, for errors the protocol does not let the client report.
    fn terminate(&self) {
        self.events.send(UrbdrcEventKind::CloseChannel {
            channel_id: self.channel_id,
        });
    }
}

enum Command {
    Submit {
        request_id: RequestId,
        envelope: Envelope,
        request: UsbRequest,
    },
    Cancel(RequestId),
    ControlDone(RequestId),
}

/// [`UrbdrcDeviceBackend`] for a device opened with nusb.
pub(crate) struct NusbDeviceBackend {
    info: DeviceInfo,
    description: String,
    location: String,
    started: Instant,
    commands: Option<mpsc::UnboundedSender<Command>>,
    reply: Reply,
    /// Keeps the events of this device current; they are dropped once the device channel is gone.
    _events_token: Arc<()>,
}

/// Runs the wrapped callback when dropped.
struct OnClose(Option<Box<dyn FnOnce() + Send>>);

impl Drop for OnClose {
    fn drop(&mut self) {
        if let Some(on_close) = self.0.take() {
            on_close();
        }
    }
}

impl NusbDeviceBackend {
    /// Start the worker thread for `device`, redirected on the device DVC `channel_id`.
    ///
    /// `on_close` runs once the worker has released the device, so that it can be offered again.
    pub(crate) fn start(
        device: OpenedDevice,
        channel_id: u32,
        sink: UrbdrcEventSink,
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
        let events_token = Arc::new(());
        let reply = Reply {
            channel_id,
            events: EventOrigin::new(&events_token, sink),
        };
        let (commands, receiver) = mpsc::unbounded_channel();

        let worker = Worker {
            device,
            interfaces,
            endpoints: BTreeMap::new(),
            controls: HashMap::new(),
            reply: reply.clone(),
            commands: commands.downgrade(),
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
                let sent = self.commands.as_ref().is_some_and(|commands| {
                    commands
                        .send(Command::Submit {
                            request_id,
                            envelope,
                            request,
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
    controls: HashMap<RequestId, AbortHandle>,
    reply: Reply,
    /// Weak, so that the worker stops once the backend drops the last strong sender.
    commands: mpsc::WeakUnboundedSender<Command>,
}

struct EndpointTask {
    interface: u8,
    commands: mpsc::UnboundedSender<EndpointCommand>,
    task: JoinHandle<()>,
}

impl Worker {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        while let Some(command) = commands.recv().await {
            match command {
                Command::Submit {
                    request_id,
                    envelope,
                    request,
                } => self.submit(request_id, envelope, request).await,
                Command::Cancel(request_id) => {
                    if let Some(control) = self.controls.remove(&request_id) {
                        // The transfer itself keeps running on the device; only its completion is dropped.
                        control.abort();
                    } else {
                        for endpoint in self.endpoints.values() {
                            let _ = endpoint.commands.send(EndpointCommand::Cancel(request_id));
                        }
                    }
                }
                Command::ControlDone(request_id) => {
                    self.controls.remove(&request_id);
                }
            }
        }

        debug!(channel_id = self.reply.channel_id, "Stopping USB device worker");
        for control in self.controls.into_values() {
            control.abort();
        }
        for (_, endpoint) in core::mem::take(&mut self.endpoints) {
            drop(endpoint.commands);
            let _ = endpoint.task.await;
        }
    }

    async fn submit(&mut self, request_id: RequestId, envelope: Envelope, request: UsbRequest) {
        let completion = match request {
            UsbRequest::Control { setup, data, timeout } => match urb::route_control(&setup) {
                ControlRoute::Forward => {
                    self.spawn_control(request_id, envelope, setup, data, timeout);
                    None
                }
                ControlRoute::GetDeviceDescriptor {
                    descriptor_type,
                    descriptor_index,
                    language_id,
                } => {
                    let device = self.device.clone();
                    let reply = self.reply.clone();
                    let length = usize::from(setup.length);
                    self.spawn(request_id, async move {
                        let completion = match device
                            .get_descriptor(descriptor_type, descriptor_index, language_id, timeout)
                            .await
                        {
                            Ok(mut descriptor) => {
                                descriptor.truncate(length);
                                urb::transfer_completion(envelope, descriptor, 0)
                            }
                            Err(error) => urb::failure_completion(envelope, descriptor_error_status(error)),
                        };
                        reply.complete(request_id, completion);
                    });
                    None
                }
                ControlRoute::SetConfiguration(value) => Some(self.set_configuration(envelope, value)),
                ControlRoute::SetInterface {
                    interface_number,
                    alternate_setting,
                } => Some(
                    match self.set_alternate_setting(interface_number, alternate_setting).await {
                        Ok(()) => urb::transfer_completion(envelope, Vec::new(), 0),
                        Err(status) => urb::failure_completion(envelope, status),
                    },
                ),
                ControlRoute::ClearHalt { endpoint } => Some(match self.clear_halt(endpoint).await {
                    Ok(()) => urb::transfer_completion(envelope, Vec::new(), 0),
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
                        let reply = self.reply.clone();
                        self.spawn(request_id, async move {
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
                            let completion = match vendor_code {
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
                            };
                            reply.complete(request_id, completion);
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
                    urb::transfer_completion(envelope, Vec::new(), 0)
                }
                PipeOperation::ClearHalt => match self.clear_halt(endpoint).await {
                    Ok(()) => urb::transfer_completion(envelope, Vec::new(), 0),
                    Err(status) => urb::failure_completion(envelope, status),
                },
                PipeOperation::Ignore => urb::transfer_completion(envelope, Vec::new(), 0),
            }),
            UsbRequest::CurrentFrameNumber | UsbRequest::Isochronous { .. } => {
                Some(urb::failure_completion(envelope, usbd_status::NOT_SUPPORTED))
            }
            UsbRequest::Reject(status) => Some(urb::failure_completion(envelope, status)),
        };

        if let Some(completion) = completion {
            self.reply.complete(request_id, completion);
        }
    }

    /// Run a request that completes on its own, keeping a handle to cancel it.
    fn spawn(&mut self, request_id: RequestId, future: impl Future<Output = ()> + Send + 'static) {
        let commands = self.commands.clone();
        let task = tokio::spawn(async move {
            future.await;
            if let Some(commands) = commands.upgrade() {
                let _ = commands.send(Command::ControlDone(request_id));
            }
        });
        self.controls.insert(request_id, task.abort_handle());
    }

    fn spawn_control(
        &mut self,
        request_id: RequestId,
        envelope: Envelope,
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

        let reply = self.reply.clone();
        self.spawn(request_id, async move {
            let completion = control(&interface, envelope, setup, data, timeout).await;
            reply.complete(request_id, completion);
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
            urb::transfer_completion(envelope, Vec::new(), 0)
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
                Ok(data) => urb::transfer_completion(envelope, data, 0),
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
                Ok(()) => urb::transfer_completion(envelope, Vec::new(), written),
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
}

enum EndpointCommand {
    Submit(Submission),
    Cancel(RequestId),
    /// Cancel every pending transfer, as `URB_FUNCTION_ABORT_PIPE` requires.
    Abort(oneshot::Sender<()>),
    ClearHalt(oneshot::Sender<Result<(), UsbdStatus>>),
}

struct InFlight {
    request_id: RequestId,
    envelope: Envelope,
    length: u32,
    /// The server cancelled this request: drop its completion.
    cancelled: bool,
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
}

/// Serve the transfers of one bulk or interrupt endpoint.
///
/// nusb can only cancel every transfer of an endpoint at once. When the server cancels one request,
/// the other IN transfers that were cancelled along with it, without moving any data, are submitted
/// again in their original order.
async fn run_endpoint<T: BulkOrInterrupt, D: EndpointDirection>(
    mut endpoint: Endpoint<T, D>,
    mut commands: mpsc::UnboundedReceiver<EndpointCommand>,
    reply: Reply,
) {
    let mut queue = EndpointQueue {
        in_flight: VecDeque::new(),
        draining: false,
        retry: VecDeque::new(),
        held: VecDeque::new(),
    };

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
                        queue.held.retain(|submission| submission.request_id != request_id);
                        queue.retry.retain(|(entry, _)| entry.request_id != request_id);
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
                let Some(entry) = queue.in_flight.pop_front() else {
                    continue;
                };
                let cancelled_with_others = queue.draining && completion.status == Err(TransferError::Cancelled);
                if entry.cancelled {
                    trace!(request_id = entry.request_id, "Dropped the completion of a cancelled USB transfer");
                } else if cancelled_with_others && completion.actual_len == 0 {
                    // Nothing moved, so the transfer runs again as if it had never been cancelled. nusb
                    // returns an OUT buffer with its whole payload.
                    let buffer = if D::DIR == nusb::transfer::Direction::In {
                        in_buffer_length(&endpoint, entry.length).map(Buffer::new)
                    } else {
                        Some(completion.buffer)
                    };
                    match buffer {
                        Some(buffer) => queue.retry.push_back((entry, buffer)),
                        None => reply.complete(
                            entry.request_id,
                            urb::failure_completion(entry.envelope, usbd_status::INVALID_PARAMETER),
                        ),
                    }
                } else if cancelled_with_others {
                    // Data already moved: report it as a short transfer rather than losing it.
                    let completion = Completion {
                        status: Ok(()),
                        ..completion
                    };
                    complete_transfer::<D>(&reply, &entry, completion);
                } else {
                    complete_transfer::<D>(&reply, &entry, completion);
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
    } = submission;

    let buffer = if D::DIR == nusb::transfer::Direction::In {
        if length == 0 {
            reply.complete(request_id, urb::transfer_completion(envelope, Vec::new(), 0));
            return;
        }
        let Some(buffer_length) = in_buffer_length(endpoint, length) else {
            reply.complete(
                request_id,
                urb::failure_completion(envelope, usbd_status::INVALID_PARAMETER),
            );
            return;
        };
        Buffer::new(buffer_length)
    } else {
        Buffer::from(data)
    };

    endpoint.submit(buffer);
    queue.in_flight.push_back(InFlight {
        request_id,
        envelope,
        length,
        cancelled: false,
    });
}

/// nusb requires IN transfers to be a multiple of the maximum packet size.
///
/// Data the device sends beyond the requested length is reported as a buffer overrun. Returns `None`
/// for a length above [`urb::MAX_TRANSFER_LENGTH`], which [`urb::translate_in`] already rejects.
fn in_buffer_length<T: BulkOrInterrupt, D: EndpointDirection>(endpoint: &Endpoint<T, D>, length: u32) -> Option<usize> {
    if urb::MAX_TRANSFER_LENGTH < length {
        return None;
    }
    let length = usize::try_from(length).ok()?;
    let max_packet_size = endpoint.max_packet_size().max(1);
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
        if !entry.cancelled {
            complete_transfer::<D>(reply, &entry, completion);
        }
    }
    queue.in_flight.clear();
    queue.draining = false;

    let cancelled = queue
        .retry
        .drain(..)
        .map(|(entry, _)| (entry.request_id, entry.envelope))
        .chain(
            queue
                .held
                .drain(..)
                .map(|submission| (submission.request_id, submission.envelope)),
        );
    for (request_id, envelope) in cancelled {
        reply.complete(request_id, urb::failure_completion(envelope, usbd_status::CANCELED));
    }
}

/// Complete a bulk or interrupt transfer, or terminate the device channel on an IN overrun.
fn complete_transfer<D: EndpointDirection>(reply: &Reply, entry: &InFlight, completion: Completion) {
    match transfer_completion::<D>(entry, completion) {
        Some(completion) => reply.complete(entry.request_id, completion),
        None => reply.terminate(),
    }
}

/// Returns `None` when an IN transfer received more data than the server asked for.
fn transfer_completion<D: EndpointDirection>(entry: &InFlight, completion: Completion) -> Option<CompletionData> {
    Some(match completion.status {
        Ok(()) if D::DIR == nusb::transfer::Direction::In => {
            let data = completion.buffer.into_vec();
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
            urb::transfer_completion(entry.envelope, data, 0)
        }
        Ok(()) => urb::transfer_completion(
            entry.envelope,
            Vec::new(),
            u32::try_from(completion.actual_len).unwrap_or(u32::MAX),
        ),
        Err(error) => urb::failure_completion(entry.envelope, transfer_error_status(error)),
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

    #[test]
    fn bus_numbers_are_stable_and_nonzero() {
        assert_eq!(bus_number("3"), 3);
        assert_eq!(bus_number("003"), 3);

        // Host controller instance IDs get distinct numbers, stable across calls.
        let first = r"PCI\VEN_8086&DEV_A0ED&SUBSYS_0A1F1028&REV_20\TEST&1";
        let second = r"PCI\VEN_8086&DEV_A0ED&SUBSYS_0A1F1028&REV_20\TEST&2";
        let first_number = bus_number(first);
        let second_number = bus_number(second);
        assert_ne!(first_number, 0);
        assert_ne!(second_number, 0);
        assert_ne!(first_number, second_number);
        assert_eq!(bus_number(first), first_number);
        assert_eq!(bus_number(second), second_number);
    }
}

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ironrdp_rdpewa::{
    Attachment, Attestation, RdpewaServerRpc, RdpewaWebAuthnRequest, UserVerification, WebAuthnPara,
    WebAuthnRequestBody,
};
use ironrdp_server::{ServerEvent, ServerEventSender};
use tokio::sync::mpsc::UnboundedSender;

use crate::rdp::ChannelGuard;
use crate::remote_devices::{AuthenticatorEvent, AuthenticatorRequest, AuthenticatorResponder, RemoteAuthenticator};

const DRDYNVC_READY_RETRIES: u8 = 80;

#[derive(Clone)]
pub(crate) struct WebAuthnInput {
    shared: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    enabled: bool,
    responder: Option<AuthenticatorResponder>,
    events: Option<UnboundedSender<ServerEvent>>,
    connected: bool,
    connection_token: Arc<AtomicBool>,
    generation: u64,
    next_guid_counter: u64,
    iuvpaa: IuvpaaState,
    ceremonies: BTreeMap<u64, Ceremony>,
}

#[derive(Default)]
enum IuvpaaState {
    #[default]
    Done,
    Pending {
        retries_left: u8,
    },
    InFlight,
}

struct Ceremony {
    cancellation_id: [u8; 16],
    active: Arc<AtomicBool>,
    channel: Option<(u32, ChannelGuard)>,
    pending_request: Option<(RdpewaWebAuthnRequest, u8)>,
    dispatched: bool,
}

impl WebAuthnInput {
    pub(crate) fn disabled() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    pub(crate) fn install(authenticator: &mut RemoteAuthenticator) -> anyhow::Result<Self> {
        authenticator.set_platform_available(false)?;
        let responder = authenticator.responder();
        let input = Self::with_responder(responder);
        let handler_input = input.clone();
        authenticator.set_handler(move |event| handler_input.handle_browser_event(event))?;
        Ok(input)
    }

    pub(crate) fn with_responder(responder: AuthenticatorResponder) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                enabled: true,
                responder: Some(responder),
                ..Shared::default()
            })),
        }
    }

    pub(crate) fn connected(&self) {
        let mut shared = self.shared.lock().expect("webauthn state poisoned");
        if !shared.enabled || shared.connected {
            return;
        }
        shared.connection_token = Arc::new(AtomicBool::new(true));
        shared.connected = true;
        shared.generation = shared.generation.wrapping_add(1);
        shared.iuvpaa = IuvpaaState::Pending {
            retries_left: DRDYNVC_READY_RETRIES,
        };
    }

    pub(crate) fn disconnected(&self) {
        let (responder, failed_ids) = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            if !shared.enabled {
                return;
            }
            shared.connection_token.store(false, Ordering::Release);
            shared.connected = false;
            shared.generation = shared.generation.wrapping_add(1);
            shared.iuvpaa = IuvpaaState::Done;
            let ceremonies = std::mem::take(&mut shared.ceremonies);
            let mut failed_ids = Vec::with_capacity(ceremonies.len());
            for (id, ceremony) in ceremonies {
                ceremony.active.store(false, Ordering::Release);
                if let Some((_, token)) = ceremony.channel {
                    token.invalidate();
                }
                failed_ids.push(id);
            }
            (shared.responder, failed_ids)
        };
        if let Some(responder) = responder {
            if let Err(error) = responder.set_platform_available(false) {
                tracing::warn!(%error, "Failed to clear remote WebAuthn platform availability");
            }
            for id in failed_ids {
                if let Err(error) = responder.respond(id, &[]) {
                    tracing::warn!(%error, id, "Failed to abort remote WebAuthn ceremony on disconnect");
                }
            }
        }
    }

    pub(crate) fn tick(&self) {
        let (events, iuvpaa, ceremonies) = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            if !shared.enabled || !shared.connected {
                return;
            }
            let Some(events) = shared.events.clone() else {
                return;
            };
            let generation = shared.generation;
            let iuvpaa = if let IuvpaaState::Pending { retries_left } = shared.iuvpaa {
                shared.iuvpaa = IuvpaaState::InFlight;
                Some((generation, shared.next_guid(), retries_left))
            } else {
                None
            };
            let pending_ids: Vec<u64> = shared
                .ceremonies
                .iter()
                .filter_map(|(id, ceremony)| ceremony.pending_request.as_ref().map(|_| *id))
                .collect();
            let mut ceremonies = Vec::with_capacity(pending_ids.len());
            for id in pending_ids {
                if let Some(ceremony) = shared.ceremonies.get_mut(&id)
                    && let Some((request, retries_left)) = ceremony.pending_request.take()
                {
                    ceremonies.push((generation, id, request, retries_left));
                }
            }
            (events, iuvpaa, ceremonies)
        };

        let mut queued = Vec::with_capacity(usize::from(iuvpaa.is_some()) + ceremonies.len());
        if let Some((generation, transaction_id, retries_left)) = iuvpaa {
            queued.push(create_iuvpaa_event(
                Arc::clone(&self.shared),
                generation,
                transaction_id,
                retries_left,
            ));
        }
        for (generation, id, request, retries_left) in ceremonies {
            queued.push(create_webauthn_event(
                Arc::clone(&self.shared),
                generation,
                id,
                request,
                retries_left,
            ));
        }

        for event in queued {
            if let Err(error) = events.send(event) {
                tracing::debug!(%error, "RDP server stopped before a WebAuthn event was sent");
                self.disconnected();
                break;
            }
        }
    }

    pub(crate) fn handle_browser_event(&self, event: AuthenticatorEvent) {
        match event {
            AuthenticatorEvent::Request { id, request } => self.start_ceremony(id, request),
            AuthenticatorEvent::Cancel { id } => self.cancel_ceremony(id),
        }
    }

    fn start_ceremony(&self, id: u64, request: AuthenticatorRequest) {
        let body = match WebAuthnRequestBody::decode(&request.ctap_request) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(%error, id, "Rejected malformed CTAP2 request from browser");
                self.fail_immediately(id);
                return;
            }
        };
        let dispatch = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            if !shared.enabled || !shared.connected {
                None
            } else if let Some(events) = shared.events.clone() {
                let transaction_id = shared.next_guid();
                let cancellation_id = shared.next_guid();
                let generation = shared.generation;
                let rpc_request = RdpewaWebAuthnRequest {
                    rp_id: (!request.rp_id.is_empty()).then_some(request.rp_id),
                    timeout_ms: request.timeout_ms,
                    transaction_id,
                    client_data_json: request.client_data_json,
                    para: WebAuthnPara {
                        attachment: Attachment::from_u32(request.attachment),
                        require_resident_key: request.require_resident_key,
                        prefer_resident_key: request.prefer_resident_key,
                        user_verification: UserVerification::from_u32(request.user_verification),
                        attestation: Attestation::from_u32(request.attestation),
                        cancellation_id: Some(cancellation_id.to_vec()),
                        ui_origin: (!request.origin.is_empty()).then_some(request.origin),
                        extra: BTreeMap::new(),
                    },
                    body,
                };
                shared.ceremonies.insert(
                    id,
                    Ceremony {
                        cancellation_id,
                        active: Arc::new(AtomicBool::new(true)),
                        channel: None,
                        pending_request: None,
                        dispatched: false,
                    },
                );
                Some((events, generation, rpc_request))
            } else {
                None
            }
        };
        match dispatch {
            Some((events, generation, rpc_request)) => {
                let event = create_webauthn_event(
                    Arc::clone(&self.shared),
                    generation,
                    id,
                    rpc_request,
                    DRDYNVC_READY_RETRIES,
                );
                if events.send(event).is_err() {
                    self.fail_immediately(id);
                }
            }
            None => self.fail_immediately(id),
        }
    }

    fn cancel_ceremony(&self, id: u64) {
        let actions = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            let Some(ceremony) = shared.ceremonies.remove(&id) else {
                return;
            };
            ceremony.active.store(false, Ordering::Release);
            let Some(events) = shared.events.clone() else {
                if let Some((_, token)) = ceremony.channel {
                    token.invalidate();
                }
                return;
            };
            let mut queued = Vec::new();
            if let Some((channel_id, token)) = ceremony.channel {
                queued.push(token.close(channel_id));
            }
            if shared.connected && ceremony.dispatched {
                let transaction_id = shared.next_guid();
                queued.push(create_cancel_event(
                    Arc::clone(&self.shared),
                    shared.generation,
                    ceremony.cancellation_id,
                    transaction_id,
                ));
            }
            Some((events, queued))
        };
        if let Some((events, queued)) = actions {
            for event in queued {
                let _ = events.send(event);
            }
        }
    }

    fn fail_immediately(&self, id: u64) {
        let responder = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            shared.ceremonies.remove(&id);
            shared.responder
        };
        if let Some(responder) = responder
            && let Err(error) = responder.respond(id, &[])
        {
            tracing::warn!(%error, id, "Failed to reject remote WebAuthn request");
        }
    }
}

impl Shared {
    fn next_guid(&mut self) -> [u8; 16] {
        self.next_guid_counter = self.next_guid_counter.wrapping_add(1);
        let mut guid = [0u8; 16];
        guid[..8].copy_from_slice(&self.generation.to_le_bytes());
        guid[8..].copy_from_slice(&self.next_guid_counter.to_le_bytes());
        guid
    }
}

impl ServerEventSender for WebAuthnInput {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.shared.lock().expect("webauthn state poisoned").events = Some(sender);
    }
}

struct UnstartedIuvpaa {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    retries_left: u8,
    armed: bool,
}

impl UnstartedIuvpaa {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for UnstartedIuvpaa {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut shared = self.shared.lock().expect("webauthn state poisoned");
        if shared.connected && shared.generation == self.generation {
            shared.iuvpaa = if 0 < self.retries_left {
                IuvpaaState::Pending {
                    retries_left: self.retries_left - 1,
                }
            } else {
                IuvpaaState::Done
            };
        }
    }
}

fn create_iuvpaa_event(
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    transaction_id: [u8; 16],
    retries_left: u8,
) -> ServerEvent {
    let mut unstarted = UnstartedIuvpaa {
        shared: Arc::clone(&shared),
        generation,
        retries_left,
        armed: true,
    };
    ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
        unstarted.disarm();
        let (token, current, events) = {
            let mut state = shared.lock().expect("webauthn state poisoned");
            let token = ChannelGuard::new(&state.connection_token);
            let current =
                state.connected && state.generation == generation && matches!(state.iuvpaa, IuvpaaState::InFlight);
            if current {
                state.iuvpaa = IuvpaaState::Done;
            }
            (token, current, state.events.clone())
        };
        if !current {
            if let Some(events) = events {
                let _ = events.send(token.close(channel_id));
            }
            return Box::new(RdpewaServerRpc::aborted());
        }
        let done_shared = Arc::clone(&shared);
        let close_token = token.clone();
        let start_token = token;
        Box::new(
            RdpewaServerRpc::iuvpaa(
                transaction_id,
                Box::new(move |result| {
                    let (responder, events) = {
                        let state = done_shared.lock().expect("webauthn state poisoned");
                        if !(state.connected && state.generation == generation) {
                            return;
                        }
                        (state.responder, state.events.clone())
                    };
                    if let Some(events) = events
                        && close_token.is_current()
                    {
                        let _ = events.send(close_token.close(channel_id));
                    }
                    if let Some(responder) = responder
                        && let Err(error) = responder.set_platform_available(result.unwrap_or(false))
                    {
                        tracing::warn!(%error, "Failed to update remote WebAuthn platform availability");
                    }
                }),
            )
            .with_start_guard(Box::new(move || start_token.is_current())),
        )
    }))
}

struct UnstartedCeremony {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    id: u64,
    request: Option<RdpewaWebAuthnRequest>,
    retries_left: u8,
}

impl UnstartedCeremony {
    fn take_request(&mut self) -> Option<RdpewaWebAuthnRequest> {
        self.request.take()
    }
}

impl Drop for UnstartedCeremony {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else {
            return;
        };
        let responder = {
            let mut shared = self.shared.lock().expect("webauthn state poisoned");
            if !(shared.connected && shared.generation == self.generation) {
                return;
            }
            let Some(ceremony) = shared.ceremonies.get_mut(&self.id) else {
                return;
            };
            if 0 < self.retries_left {
                ceremony.pending_request = Some((request, self.retries_left - 1));
                None
            } else {
                shared.ceremonies.remove(&self.id);
                shared.responder
            }
        };
        if let Some(responder) = responder {
            let _ = responder.respond(self.id, &[]);
        }
    }
}

fn create_webauthn_event(
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    id: u64,
    request: RdpewaWebAuthnRequest,
    retries_left: u8,
) -> ServerEvent {
    let mut unstarted = UnstartedCeremony {
        shared: Arc::clone(&shared),
        generation,
        id,
        request: Some(request),
        retries_left,
    };
    ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
        let request = unstarted.take_request().expect("ceremony request present");
        let (token, ceremony_active, events) = {
            let mut state = shared.lock().expect("webauthn state poisoned");
            let token = ChannelGuard::new(&state.connection_token);
            let ceremony_active = if state.connected
                && state.generation == generation
                && let Some(ceremony) = state.ceremonies.get_mut(&id)
            {
                ceremony.channel = Some((channel_id, token.clone()));
                ceremony.dispatched = true;
                Some(Arc::clone(&ceremony.active))
            } else {
                None
            };
            (token, ceremony_active, state.events.clone())
        };
        let Some(ceremony_active) = ceremony_active else {
            if let Some(events) = events {
                let _ = events.send(token.close(channel_id));
            }
            return Box::new(RdpewaServerRpc::aborted());
        };
        let done_shared = Arc::clone(&shared);
        let close_token = token.clone();
        let start_token = token;
        Box::new(
            RdpewaServerRpc::webauthn(
                request,
                Box::new(move |result| {
                    let (responder, events, should_respond) = {
                        let mut state = done_shared.lock().expect("webauthn state poisoned");
                        let should_respond =
                            state.connected && state.generation == generation && state.ceremonies.remove(&id).is_some();
                        (state.responder, state.events.clone(), should_respond)
                    };
                    if let Some(events) = events
                        && close_token.is_current()
                    {
                        let _ = events.send(close_token.close(channel_id));
                    }
                    if should_respond && let Some(responder) = responder {
                        let ctap = match &result {
                            Ok(payload) => payload.response.as_slice(),
                            Err(_) => &[],
                        };
                        if let Err(error) = responder.respond(id, ctap) {
                            tracing::warn!(%error, id, "Failed to deliver remote WebAuthn response");
                        }
                    }
                }),
            )
            .with_start_guard(Box::new(move || {
                start_token.is_current() && ceremony_active.load(Ordering::Acquire)
            })),
        )
    }))
}

fn create_cancel_event(
    shared: Arc<Mutex<Shared>>,
    generation: u64,
    cancellation_id: [u8; 16],
    transaction_id: [u8; 16],
) -> ServerEvent {
    ServerEvent::CreateDynamicChannel(Box::new(move |channel_id| {
        let (token, current, events) = {
            let state = shared.lock().expect("webauthn state poisoned");
            let token = ChannelGuard::new(&state.connection_token);
            (
                token,
                state.connected && state.generation == generation,
                state.events.clone(),
            )
        };
        if !current {
            if let Some(events) = &events {
                let _ = events.send(token.close(channel_id));
            }
            return Box::new(RdpewaServerRpc::aborted());
        }
        let close_token = token.clone();
        let start_token = token;
        Box::new(
            RdpewaServerRpc::cancel(
                cancellation_id,
                transaction_id,
                Box::new(move |_| {
                    if let Some(events) = events
                        && close_token.is_current()
                    {
                        let _ = events.send(close_token.close(channel_id));
                    }
                }),
            )
            .with_start_guard(Box::new(move || start_token.is_current())),
        )
    }))
}

#[cfg(test)]
mod tests {
    use std::ffi::c_int;
    use std::sync::Mutex;

    use ironrdp_core::encode_vec;
    use ironrdp_rdpewa::{DeviceInfo, RdpewaRequest, RdpewaResponse, RpcCommand, S_OK, WebAuthnResponsePayload};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static PLATFORM_VALUES: Mutex<Vec<bool>> = Mutex::new(Vec::new());
    static RESPONSES: Mutex<Vec<(u64, Vec<u8>)>> = Mutex::new(Vec::new());

    unsafe extern "C" fn record_respond(id: u64, data: *const u8, len: usize) -> c_int {
        let bytes = if data.is_null() || len == 0 {
            Vec::new()
        } else {
            // SAFETY: Test caller supplies `len` readable bytes.
            unsafe { std::slice::from_raw_parts(data, len) }.to_vec()
        };
        RESPONSES.lock().unwrap().push((id, bytes));
        0
    }

    unsafe extern "C" fn record_platform(available: c_int) -> c_int {
        PLATFORM_VALUES.lock().unwrap().push(available != 0);
        0
    }

    fn reset_records() {
        PLATFORM_VALUES.lock().unwrap().clear();
        RESPONSES.lock().unwrap().clear();
    }

    fn sample_request() -> AuthenticatorRequest {
        AuthenticatorRequest {
            rp_id: "localhost".to_owned(),
            origin: "https://localhost".to_owned(),
            client_data_json: br#"{"type":"webauthn.create"}"#.to_vec(),
            ctap_request: vec![0x01, 0xA1, 0x01, 0x40],
            timeout_ms: 30_000,
            attachment: 1,
            user_verification: 1,
            attestation: 1,
            require_resident_key: true,
            prefer_resident_key: false,
        }
    }

    // oracle: mutant hanging a WebAuthn ceremony when no RDP client is connected.
    #[test]
    fn request_while_disconnected_fails_immediately() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_records();
        let webauthn = WebAuthnInput::with_responder(AuthenticatorResponder::for_test(record_respond, record_platform));

        webauthn.handle_browser_event(AuthenticatorEvent::Request {
            id: 5,
            request: sample_request(),
        });

        assert_eq!(*RESPONSES.lock().unwrap(), [(5, Vec::new())]);
    }

    // oracle: spec MS-RDPEWA §§2.2.1 and 2.2.2; mutant failing to retry IUVPAA before DRDYNVC is ready or keeping availability after disconnect.
    #[test]
    fn iuvpaa_retries_after_unready_drop_and_resets_on_disconnect() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_records();
        let mut webauthn =
            WebAuthnInput::with_responder(AuthenticatorResponder::for_test(record_respond, record_platform));
        let (sender, mut events) = unbounded_channel();
        webauthn.set_sender(sender);
        webauthn.connected();

        webauthn.tick();
        let first = events.try_recv().expect("initial IUVPAA channel request");
        drop(first);

        webauthn.tick();
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().expect("retried IUVPAA channel request")
        else {
            panic!("expected dynamic channel creation");
        };
        let mut processor = build(21);
        let outbound = processor.start(21).unwrap();
        let decoded = RdpewaRequest::decode(&encode_vec(outbound[0].as_ref()).unwrap()).unwrap();
        assert_eq!(decoded.command, RpcCommand::Iuvpaa);

        processor
            .process(21, &RdpewaResponse::with_u32(S_OK, 1).to_bytes())
            .unwrap();
        assert_eq!(*PLATFORM_VALUES.lock().unwrap(), [true]);
        let ServerEvent::CloseDynamicChannel { channel_id, is_current } =
            events.try_recv().expect("close per-RPC WebAuthN_Channel")
        else {
            panic!("expected dynamic channel close");
        };
        assert_eq!(channel_id, 21);
        assert!(is_current());

        webauthn.disconnected();
        assert_eq!(*PLATFORM_VALUES.lock().unwrap(), [true, false]);
    }

    // oracle: spec MS-RDPEWA §§2.2.1 and 4.3.1; mutant omitting client cancellation or leaking ceremony state across disconnect.
    #[test]
    fn ceremony_forwards_response_cancel_and_disconnect_abort() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_records();
        let mut webauthn =
            WebAuthnInput::with_responder(AuthenticatorResponder::for_test(record_respond, record_platform));
        let (sender, mut events) = unbounded_channel();
        webauthn.set_sender(sender);
        webauthn.connected();

        webauthn.handle_browser_event(AuthenticatorEvent::Request {
            id: 1,
            request: sample_request(),
        });
        let ServerEvent::CreateDynamicChannel(build) = events.try_recv().expect("WebAuthn channel request") else {
            panic!("expected dynamic channel creation");
        };
        let mut processor = build(30);
        let outbound = processor.start(30).unwrap();
        let decoded = RdpewaRequest::decode(&encode_vec(outbound[0].as_ref()).unwrap()).unwrap();
        assert_eq!(decoded.command, RpcCommand::WebAuthn);
        let cancellation_id = decoded
            .webauthn_para
            .as_ref()
            .and_then(|para| para.cancellation_id.clone())
            .expect("cancellation GUID");

        let payload = WebAuthnResponsePayload {
            device_info: DeviceInfo::default(),
            status: 0,
            response: vec![0x00, 0xA0],
        };
        processor
            .process(
                30,
                &RdpewaResponse::with_payload(S_OK, payload.encode().unwrap()).to_bytes(),
            )
            .unwrap();
        assert_eq!(*RESPONSES.lock().unwrap(), [(1, vec![0x00, 0xA0])]);
        let _ = events.try_recv().expect("close ceremony channel");

        webauthn.handle_browser_event(AuthenticatorEvent::Request {
            id: 2,
            request: sample_request(),
        });
        let ServerEvent::CreateDynamicChannel(build_second) = events.try_recv().expect("second WebAuthn channel")
        else {
            panic!("expected dynamic channel creation");
        };
        let mut second_processor = build_second(31);
        let second_outbound = second_processor.start(31).unwrap();
        let second_decoded = RdpewaRequest::decode(&encode_vec(second_outbound[0].as_ref()).unwrap()).unwrap();
        let second_cancel_id = second_decoded
            .webauthn_para
            .unwrap()
            .cancellation_id
            .expect("second cancellation GUID");
        assert_ne!(cancellation_id, second_cancel_id);

        webauthn.handle_browser_event(AuthenticatorEvent::Cancel { id: 2 });
        let ServerEvent::CloseDynamicChannel { channel_id, .. } = events.try_recv().expect("close cancelled channel")
        else {
            panic!("expected close event");
        };
        assert_eq!(channel_id, 31);
        let ServerEvent::CreateDynamicChannel(build_cancel) = events.try_recv().expect("cancel RPC channel") else {
            panic!("expected cancel channel creation");
        };
        let mut cancel_processor = build_cancel(32);
        let cancel_outbound = cancel_processor.start(32).unwrap();
        let cancel_decoded = RdpewaRequest::decode(&encode_vec(cancel_outbound[0].as_ref()).unwrap()).unwrap();
        assert_eq!(cancel_decoded.command, RpcCommand::CancelCurOp);
        assert_eq!(cancel_decoded.request_body, second_cancel_id);
        drop(second_processor);
        drop(cancel_processor);
        while events.try_recv().is_ok() {}

        webauthn.handle_browser_event(AuthenticatorEvent::Request {
            id: 3,
            request: sample_request(),
        });
        let ServerEvent::CreateDynamicChannel(stale_build) = events.try_recv().expect("third WebAuthn channel") else {
            panic!("expected dynamic channel creation");
        };
        webauthn.disconnected();
        assert_eq!(RESPONSES.lock().unwrap().last(), Some(&(3, Vec::new())));

        // Even if an already-queued factory runs before its Close event, it must emit no request bytes.
        let mut inert = stale_build(33);
        assert!(inert.start(33).unwrap().is_empty());

        // Canceling between channel reservation and CreateResponse must also prevent request emission.
        webauthn.connected();
        while events.try_recv().is_ok() {}
        webauthn.handle_browser_event(AuthenticatorEvent::Request {
            id: 4,
            request: sample_request(),
        });
        let ServerEvent::CreateDynamicChannel(reserved_build) = events.try_recv().expect("fourth WebAuthn channel")
        else {
            panic!("expected dynamic channel creation");
        };
        let mut reserved = reserved_build(34);
        webauthn.handle_browser_event(AuthenticatorEvent::Cancel { id: 4 });
        assert!(reserved.start(34).unwrap().is_empty());
    }
}

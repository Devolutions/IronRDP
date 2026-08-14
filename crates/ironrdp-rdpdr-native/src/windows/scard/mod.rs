//! Windows WinSCard core backend for MS-RDPESC Device Control IRPs.
//!
//! ANSI (`*A`) IOCTLs upgrade to UTF-16 and call WinSCard `*W` APIs.
//! List/Status `*A` replies keep ANSI on the wire. Extended IOCTLs remain unsupported.

use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use ironrdp_core::Encode as _;
use ironrdp_pdu::PduResult;
use ironrdp_pdu::utils::{CharacterSet, encoded_multistring_len};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{DeviceControlRequest, DeviceControlResponse, NtStatus};
use ironrdp_rdpdr::pdu::esc::{
    CardProtocol, CardState, CardStateFlags, ConnectCall, ConnectReturn, ContextCall, ControlReturn,
    EstablishContextCall, EstablishContextReturn, GetAttribReturn, GetDeviceTypeIdReturn, GetReaderIconReturn,
    GetStatusChangeCall, GetStatusChangeReturn, GetTransmitCountReturn, HCardAndDispositionCall, ListReaderGroupsCall,
    ListReadersCall, ListReadersReturn, LongReturn, ReadCacheReturn, ReaderState, ReaderStateCommonCall, ReconnectCall,
    ReconnectReturn, ReturnCode, SCardIORequest, ScardCall, ScardContext, ScardHandle, ScardIoCtlCode, Scope,
    StateCall, StateReturn, StatusCall, StatusReturn, TransmitCall, TransmitReturn, rpce,
};
use ironrdp_svc::SvcMessage;
use tracing::warn;
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Security::Credentials::{
    SCARD_IO_REQUEST, SCARD_READERSTATEW, SCARD_SCOPE, SCARD_SCOPE_SYSTEM, SCARD_SCOPE_USER, SCARD_STATE,
    SCardAccessStartedEvent, SCardBeginTransaction, SCardCancel, SCardConnectW, SCardDisconnect, SCardEndTransaction,
    SCardEstablishContext, SCardGetStatusChangeW, SCardIsValidContext, SCardListReaderGroupsW, SCardListReadersW,
    SCardReconnect, SCardReleaseContext, SCardReleaseStartedEvent, SCardStatusW, SCardTransmit,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::{PCWSTR, PWSTR};

const MAX_DEFERRED: usize = 32;
const MAX_ATR: usize = 32;
const MAX_RECV: u32 = 64 * 1024;
const SCARD_SCOPE_TERMINAL: SCARD_SCOPE = SCARD_SCOPE(1);

/// Per-connection WinSCard state for RDPDR smartcard device ID 0.
#[derive(Debug)]
pub(super) struct ScardSession {
    contexts: HashMap<usize, ()>,
    cards: HashMap<usize, usize>,
    deferred: DeferredOps,
    access_event: Option<isize>,
    access_refs: u32,
}

#[derive(Debug)]
struct DeferredOps {
    next_id: u64,
    epoch: u64,
    ops: HashMap<u64, PendingOp>,
    completions: Arc<Mutex<Vec<DeferredCompletion>>>,
}

#[derive(Debug)]
struct PendingOp {
    context_id: usize,
    cancel: Arc<AtomicBool>,
    worker: thread::JoinHandle<()>,
}

#[derive(Debug)]
struct DeferredCompletion {
    id: u64,
    epoch: u64,
    outcome: Outcome,
}

#[derive(Debug)]
enum Outcome {
    Message(SvcMessage),
    Connect {
        req: DeviceControlRequest<ScardIoCtlCode>,
        context: ScardContext,
        code: ReturnCode,
        native_handle: usize,
        active_protocol: CardProtocol,
    },
    Disconnect {
        req: DeviceControlRequest<ScardIoCtlCode>,
        card: usize,
        code: ReturnCode,
    },
}

impl ScardSession {
    pub(super) fn new() -> Self {
        Self {
            contexts: HashMap::new(),
            cards: HashMap::new(),
            deferred: DeferredOps::new(),
            access_event: None,
            access_refs: 0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.deferred.reset();
        for h in self.cards.keys().copied() {
            // SAFETY: WinSCard.
            let _ = unsafe { SCardDisconnect(h, 0) };
        }
        self.cards.clear();
        for c in self.contexts.keys().copied() {
            // SAFETY: WinSCard.
            let _ = unsafe { SCardReleaseContext(c) };
        }
        self.contexts.clear();
        while self.access_refs > 0 {
            // SAFETY: WinSCard.
            unsafe { SCardReleaseStartedEvent() };
            self.access_refs -= 1;
        }
        self.access_event = None;
    }

    pub(super) fn poll(&mut self) -> Vec<SvcMessage> {
        self.deferred.poll().into_iter().map(|o| self.apply(o)).collect()
    }

    fn apply(&mut self, outcome: Outcome) -> SvcMessage {
        match outcome {
            Outcome::Message(m) => m,
            Outcome::Connect {
                req,
                context,
                code,
                native_handle,
                active_protocol,
            } => {
                if code != ReturnCode::Success {
                    return complete_connect(req, code, ScardHandle::from_native(context, 0), active_protocol);
                }
                let ctx = context.native();
                if !self.contexts.contains_key(&ctx) {
                    // SAFETY: WinSCard.
                    let _ = unsafe { SCardDisconnect(native_handle, 0) };
                    return complete_connect(
                        req,
                        ReturnCode::InvalidHandle,
                        ScardHandle::from_native(context, 0),
                        CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                    );
                }
                let handle = ScardHandle::from_native(context, native_handle);
                let out = ConnectReturn::new(ReturnCode::Success, handle, active_protocol);
                if out.size() > req.output_buffer_length as usize {
                    // SAFETY: WinSCard.
                    let _ = unsafe { SCardDisconnect(native_handle, 0) };
                    return buffer_too_small(req);
                }
                self.cards.insert(native_handle, ctx);
                complete_rpce(req, Box::new(out))
            }
            Outcome::Disconnect { req, card, code } => {
                if code == ReturnCode::Success {
                    self.cards.remove(&card);
                }
                complete_long(req, code)
            }
        }
    }

    fn defer<F, E>(
        &mut self,
        context_id: usize,
        name: &str,
        req: DeviceControlRequest<ScardIoCtlCode>,
        work: F,
        on_err: E,
    ) -> PduResult<Vec<SvcMessage>>
    where
        F: FnOnce(DeviceControlRequest<ScardIoCtlCode>, Arc<AtomicBool>) -> Outcome + Send + 'static,
        E: FnOnce(DeviceControlRequest<ScardIoCtlCode>, ReturnCode) -> SvcMessage,
    {
        if self.deferred.ops.len() >= MAX_DEFERRED {
            return Ok(vec![on_err(req, ReturnCode::NoMemory)]);
        }
        let id = self.deferred.alloc_id();
        let cancel = Arc::new(AtomicBool::new(false));
        let completions = Arc::clone(&self.deferred.completions);
        let epoch = self.deferred.epoch;
        let worker_cancel = Arc::clone(&cancel);
        let req_err = req.clone();
        let worker = match thread::Builder::new().name(name.into()).spawn(move || {
            push(completions, id, epoch, work(req, worker_cancel));
        }) {
            Ok(h) => h,
            Err(_) => return Ok(vec![on_err(req_err, ReturnCode::InternalError)]),
        };
        self.deferred.ops.insert(
            id,
            PendingOp {
                context_id,
                cancel,
                worker,
            },
        );
        Ok(Vec::new())
    }

    pub(super) fn handle_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let ioctl = req.io_control_code;
        if ioctl == ScardIoCtlCode::ReleaseTartedEvent {
            return Ok(vec![complete_long(req, ReturnCode::Success)]);
        }
        match call {
            ScardCall::AccessStartedEventCall(_) => self.access_started(req),
            ScardCall::EstablishContextCall(c) => Ok(vec![self.establish_context(req, c)]),
            ScardCall::ListReaderGroupsCall(c) => Ok(vec![self.list_groups(req, ioctl, c)]),
            ScardCall::ListReadersCall(c) => Ok(vec![self.list_readers(req, ioctl, c)]),
            ScardCall::GetStatusChangeCall(c) => self.get_status_change(req, c),
            ScardCall::ConnectCall(c) => self.connect(req, c),
            ScardCall::ReconnectCall(c) => self.reconnect(req, c),
            ScardCall::HCardAndDispositionCall(c) => self.hcard(req, ioctl, c),
            ScardCall::TransmitCall(c) => self.transmit(req, c),
            ScardCall::StatusCall(c) => self.status(req, ioctl, c),
            ScardCall::StateCall(c) => self.state(req, c),
            ScardCall::ContextCall(c) => Ok(vec![self.context_call(req, ioctl, c)]),
            other => Ok(vec![complete_rpce(req, unsupported(ioctl, other))]),
        }
    }

    fn ctx(&self, c: ScardContext) -> Result<usize, ReturnCode> {
        let n = c.native();
        self.contexts
            .contains_key(&n)
            .then_some(n)
            .ok_or(ReturnCode::InvalidHandle)
    }

    fn card(&self, h: &ScardHandle) -> Result<(usize, usize), ReturnCode> {
        let card = h.native();
        let ctx = *self.cards.get(&card).ok_or(ReturnCode::InvalidHandle)?;
        if ctx != h.context().native() {
            return Err(ReturnCode::InvalidHandle);
        }
        Ok((ctx, card))
    }

    fn access_started(&mut self, req: DeviceControlRequest<ScardIoCtlCode>) -> PduResult<Vec<SvcMessage>> {
        let raw = match self.access_event {
            Some(r) => r,
            // SAFETY: WinSCard.
            None => match unsafe { SCardAccessStartedEvent() } {
                Ok(h) if !h.is_invalid() => {
                    let r = h.0 as isize;
                    self.access_event = Some(r);
                    self.access_refs = 1;
                    r
                }
                _ => return Ok(vec![complete_long(req, ReturnCode::Success)]),
            },
        };
        // SAFETY: WinSCard.
        if unsafe { WaitForSingleObject(HANDLE(raw as *mut _), 0) } == WAIT_OBJECT_0 {
            return Ok(vec![complete_long(req, ReturnCode::Success)]);
        }
        self.defer(
            0,
            "ironrdp-scard-access",
            req,
            move |req, cancel| {
                let code = loop {
                    if cancel.load(Ordering::Acquire) {
                        break ReturnCode::Cancelled;
                    }
                    // SAFETY: WinSCard.
                    let w = unsafe { WaitForSingleObject(HANDLE(raw as *mut _), 100) };
                    if w == WAIT_OBJECT_0 || w != WAIT_TIMEOUT {
                        break ReturnCode::Success;
                    }
                };
                Outcome::Message(complete_long(req, code))
            },
            complete_long,
        )
    }

    fn establish_context(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: EstablishContextCall,
    ) -> SvcMessage {
        let preview = EstablishContextReturn::new(ReturnCode::Success, ScardContext::from_native(usize::MAX));
        if preview.size() > req.output_buffer_length as usize {
            return buffer_too_small(req);
        }
        let scope = match call.scope {
            Scope::User => SCARD_SCOPE_USER,
            Scope::Terminal => SCARD_SCOPE_TERMINAL,
            Scope::System => SCARD_SCOPE_SYSTEM,
        };
        let mut native = 0usize;
        // SAFETY: WinSCard.
        let code = map_status(unsafe { SCardEstablishContext(scope, None, None, &mut native) });
        if code != ReturnCode::Success {
            return complete_establish(req, code, ScardContext::from_native(0));
        }
        self.contexts.insert(native, ());
        complete_establish(req, ReturnCode::Success, ScardContext::from_native(native))
    }

    fn list_groups(
        &self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        ioctl: ScardIoCtlCode,
        call: ListReaderGroupsCall,
    ) -> SvcMessage {
        let cs = list_cs(ioctl);
        match self.ctx(call.context).and_then(list_groups_w) {
            Ok(v) => complete_list_fit(req, v, call.groups_is_null, call.groups_size, cs),
            Err(c) => complete_list(req, c, Vec::new(), cs),
        }
    }

    fn list_readers(
        &self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        ioctl: ScardIoCtlCode,
        call: ListReadersCall,
    ) -> SvcMessage {
        let cs = list_cs(ioctl);
        match self.ctx(call.context).and_then(|n| list_readers_w(n, &call.groups)) {
            Ok(v) => complete_list_fit(req, v, call.readers_is_null, call.readers_size, cs),
            Err(c) => complete_list(req, c, Vec::new(), cs),
        }
    }

    fn get_status_change(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: GetStatusChangeCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let native = match self.ctx(call.context) {
            Ok(n) => n,
            Err(c) => return Ok(vec![complete_gsc(req, c, Vec::new())]),
        };
        let ctx_id = call.context.native();
        self.defer(
            ctx_id,
            "ironrdp-scard-gsc",
            req,
            move |req, cancel| {
                if cancel.load(Ordering::Acquire) {
                    return Outcome::Message(complete_gsc(req, ReturnCode::Cancelled, Vec::new()));
                }
                let bufs: Vec<Vec<u16>> = call.states.iter().map(|s| wide(&s.reader)).collect();
                let mut states: Vec<SCARD_READERSTATEW> = call
                    .states
                    .iter()
                    .zip(bufs.iter())
                    .map(|(s, r)| reader_state_w(s, r))
                    .collect();
                // SAFETY: WinSCard.
                let status =
                    unsafe { SCardGetStatusChangeW(native, call.timeout, states.as_mut_ptr(), states.len() as u32) };
                let code = if cancel.load(Ordering::Acquire) {
                    ReturnCode::Cancelled
                } else {
                    map_status(status)
                };
                let out = if matches!(code, ReturnCode::Success | ReturnCode::Timeout) {
                    reader_states_from_native(states)
                } else {
                    Vec::new()
                };
                Outcome::Message(complete_gsc(req, code, out))
            },
            |req, c| complete_gsc(req, c, Vec::new()),
        )
    }

    fn connect(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ConnectCall) -> PduResult<Vec<SvcMessage>> {
        let native_ctx = match self.ctx(call.common.context) {
            Ok(n) => n,
            Err(c) => {
                return Ok(vec![complete_connect(
                    req,
                    c,
                    ScardHandle::from_native(call.common.context, 0),
                    CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                )]);
            }
        };
        let preview = ConnectReturn::new(
            ReturnCode::Success,
            ScardHandle::from_native(call.common.context, usize::MAX),
            CardProtocol::SCARD_PROTOCOL_T0,
        );
        if preview.size() > req.output_buffer_length as usize {
            return Ok(vec![buffer_too_small(req)]);
        }
        let context = call.common.context;
        self.defer(
            context.native(),
            "ironrdp-scard-connect",
            req,
            move |req, _| {
                let reader = wide(&call.reader);
                let mut card = 0usize;
                let mut active = 0u32;
                // SAFETY: WinSCard.
                let status = unsafe {
                    SCardConnectW(
                        native_ctx,
                        PCWSTR(reader.as_ptr()),
                        call.common.share_mode,
                        call.common.preferred_protocols.bits(),
                        &mut card,
                        &mut active,
                    )
                };
                Outcome::Connect {
                    req,
                    context: call.common.context,
                    code: map_status(status),
                    native_handle: card,
                    active_protocol: CardProtocol::from_bits_retain(active),
                }
            },
            move |req, c| {
                complete_connect(
                    req,
                    c,
                    ScardHandle::from_native(context, 0),
                    CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                )
            },
        )
    }

    fn reconnect(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ReconnectCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let (ctx_id, handle) = match self.card(&call.handle) {
            Ok(v) => v,
            Err(c) => return Ok(vec![complete_reconnect(req, c, CardProtocol::SCARD_PROTOCOL_UNDEFINED)]),
        };
        self.defer(
            ctx_id,
            "ironrdp-scard-reconnect",
            req,
            move |req, _| {
                let mut active = 0u32;
                // SAFETY: WinSCard.
                let status = unsafe {
                    SCardReconnect(
                        handle,
                        call.share_mode,
                        call.preferred_protocols.bits(),
                        call.initialization,
                        Some(&mut active),
                    )
                };
                Outcome::Message(complete_reconnect(
                    req,
                    map_status(status),
                    CardProtocol::from_bits_retain(active),
                ))
            },
            |req, c| complete_reconnect(req, c, CardProtocol::SCARD_PROTOCOL_UNDEFINED),
        )
    }

    fn hcard(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        ioctl: ScardIoCtlCode,
        call: HCardAndDispositionCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let (ctx_id, handle) = match self.card(&call.handle) {
            Ok(v) => v,
            Err(c) => return Ok(vec![complete_long(req, c)]),
        };
        let name = match ioctl {
            ScardIoCtlCode::BeginTransaction => "ironrdp-scard-begin",
            ScardIoCtlCode::EndTransaction => "ironrdp-scard-end",
            ScardIoCtlCode::Disconnect => "ironrdp-scard-disc",
            _ => return Ok(vec![complete_long(req, ReturnCode::UnsupportedFeature)]),
        };
        let disp = call.disposition;
        self.defer(
            ctx_id,
            name,
            req,
            move |req, _| match ioctl {
                // SAFETY: WinSCard.
                ScardIoCtlCode::BeginTransaction => {
                    Outcome::Message(complete_long(req, map_status(unsafe { SCardBeginTransaction(handle) })))
                }
                ScardIoCtlCode::EndTransaction => Outcome::Message(complete_long(
                    req,
                    map_status(unsafe { SCardEndTransaction(handle, disp) }),
                )),
                ScardIoCtlCode::Disconnect => Outcome::Disconnect {
                    req,
                    card: handle,
                    code: map_status(unsafe { SCardDisconnect(handle, disp) }),
                },
                _ => Outcome::Message(complete_long(req, ReturnCode::UnsupportedFeature)),
            },
            complete_long,
        )
    }

    fn transmit(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: TransmitCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let (ctx_id, handle) = match self.card(&call.handle) {
            Ok(v) => v,
            Err(c) => return Ok(vec![complete_transmit(req, c, None, Vec::new())]),
        };
        self.defer(
            ctx_id,
            "ironrdp-scard-xmit",
            req,
            move |req, _| run_transmit(req, handle, call),
            |req, c| complete_transmit(req, c, None, Vec::new()),
        )
    }

    fn status(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        ioctl: ScardIoCtlCode,
        call: StatusCall,
    ) -> PduResult<Vec<SvcMessage>> {
        let (ctx_id, handle) = match self.card(&call.handle) {
            Ok(v) => v,
            Err(c) => return Ok(vec![complete_status_err(req, c, ioctl)]),
        };
        self.defer(
            ctx_id,
            "ironrdp-scard-status",
            req,
            move |req, _| match card_status_w(handle) {
                Ok((names, state, proto, atr, atr_len)) => {
                    let cs = status_cs(ioctl);
                    let (code, names_opt, c_bytes) =
                        fit_multi(names, call.reader_names_is_null, call.reader_length, cs);
                    Outcome::Message(complete_status_fit(
                        req, code, names_opt, c_bytes, state, proto, atr, atr_len, cs,
                    ))
                }
                Err(c) => Outcome::Message(complete_status_err(req, c, ioctl)),
            },
            move |req, c| complete_status_err(req, c, ioctl),
        )
    }

    fn state(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: StateCall) -> PduResult<Vec<SvcMessage>> {
        let (ctx_id, handle) = match self.card(&call.handle) {
            Ok(v) => v,
            Err(c) => {
                return Ok(vec![complete_state(
                    req,
                    c,
                    CardState::Unknown,
                    CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                    Vec::new(),
                )]);
            }
        };
        self.defer(
            ctx_id,
            "ironrdp-scard-state",
            req,
            move |req, _| match card_status_w(handle) {
                Ok((_n, state, proto, atr, atr_len)) => Outcome::Message(complete_state_fit(
                    req,
                    ReturnCode::Success,
                    state,
                    proto,
                    atr,
                    atr_len,
                    call,
                )),
                Err(c) => Outcome::Message(complete_state(
                    req,
                    c,
                    CardState::Unknown,
                    CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                    Vec::new(),
                )),
            },
            |req, c| {
                complete_state(
                    req,
                    c,
                    CardState::Unknown,
                    CardProtocol::SCARD_PROTOCOL_UNDEFINED,
                    Vec::new(),
                )
            },
        )
    }

    fn context_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        ioctl: ScardIoCtlCode,
        call: ContextCall,
    ) -> SvcMessage {
        match ioctl {
            ScardIoCtlCode::ReleaseContext => self.release_context(req, call),
            ScardIoCtlCode::IsValidContext => match self.ctx(call.context) {
                // SAFETY: WinSCard.
                Ok(n) => complete_long(req, map_status(unsafe { SCardIsValidContext(n) })),
                Err(c) => complete_long(req, c),
            },
            ScardIoCtlCode::Cancel => match self.ctx(call.context) {
                Ok(n) => {
                    for op in self.deferred.ops.values() {
                        if op.context_id == n {
                            op.cancel.store(true, Ordering::Release);
                        }
                    }
                    // SAFETY: WinSCard.
                    complete_long(req, map_status(unsafe { SCardCancel(n) }))
                }
                Err(c) => complete_long(req, c),
            },
            _ => complete_long(req, ReturnCode::UnsupportedFeature),
        }
    }

    fn release_context(&mut self, req: DeviceControlRequest<ScardIoCtlCode>, call: ContextCall) -> SvcMessage {
        let native = match self.ctx(call.context) {
            Ok(n) => n,
            Err(c) => return complete_long(req, c),
        };
        // ReleaseContext frees context-owned cards; update maps only after success.
        // SAFETY: WinSCard.
        let code = map_status(unsafe { SCardReleaseContext(native) });
        if code == ReturnCode::Success {
            self.cards.retain(|_, c| *c != native);
            self.contexts.remove(&native);
        }
        complete_long(req, code)
    }
}

impl Drop for ScardSession {
    fn drop(&mut self) {
        self.reset();
    }
}

impl DeferredOps {
    fn new() -> Self {
        Self {
            next_id: 1,
            epoch: 0,
            ops: HashMap::new(),
            completions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    fn poll(&mut self) -> Vec<Outcome> {
        let mut ready = match self.completions.lock() {
            Ok(mut g) => core::mem::take(&mut *g),
            Err(p) => core::mem::take(&mut *p.into_inner()),
        };
        ready.retain(|c| c.epoch == self.epoch);
        let mut out = Vec::with_capacity(ready.len());
        for c in ready {
            if let Some(op) = self.ops.remove(&c.id) {
                let _ = op.worker.join();
            }
            out.push(c.outcome);
        }
        self.ops.retain(|_, op| !op.worker.is_finished());
        out
    }

    fn reset(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        for (_, op) in self.ops.drain() {
            op.cancel.store(true, Ordering::Release);
            let _ = op.worker.join();
        }
        if let Ok(mut g) = self.completions.lock() {
            g.clear();
        }
    }
}

fn push(completions: Arc<Mutex<Vec<DeferredCompletion>>>, id: u64, epoch: u64, outcome: Outcome) {
    let item = DeferredCompletion { id, epoch, outcome };
    match completions.lock() {
        Ok(mut g) => g.push(item),
        Err(p) => p.into_inner().push(item),
    }
}

fn list_groups_w(context: usize) -> Result<Vec<String>, ReturnCode> {
    let mut needed = 0u32;
    // SAFETY: WinSCard.
    let probe = unsafe { SCardListReaderGroupsW(context, None, &mut needed) };
    let probe_code = map_status(probe);
    if probe_code == ReturnCode::NoReadersAvailable {
        return Ok(Vec::new());
    }
    if probe_code != ReturnCode::Success && probe_code != ReturnCode::InsufficientBuffer {
        return Err(probe_code);
    }
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u16; needed as usize];
    let mut len = needed;
    // SAFETY: WinSCard.
    let code = map_status(unsafe { SCardListReaderGroupsW(context, Some(PWSTR(buf.as_mut_ptr())), &mut len) });
    if code != ReturnCode::Success {
        return Err(code);
    }
    Ok(parse_multi_wide(&buf[..len as usize]))
}

fn list_readers_w(context: usize, groups: &[String]) -> Result<Vec<String>, ReturnCode> {
    let groups_w = if groups.is_empty() {
        None
    } else {
        Some(multi_wide(groups))
    };
    let groups_ptr = groups_w.as_ref().map(|g| PCWSTR(g.as_ptr())).unwrap_or(PCWSTR::null());
    let mut needed = 0u32;
    // SAFETY: WinSCard.
    let probe = unsafe { SCardListReadersW(context, groups_ptr, None, &mut needed) };
    let probe_code = map_status(probe);
    if probe_code == ReturnCode::NoReadersAvailable {
        return Ok(Vec::new());
    }
    if probe_code != ReturnCode::Success && probe_code != ReturnCode::InsufficientBuffer {
        return Err(probe_code);
    }
    if needed == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u16; needed as usize];
    let mut len = needed;
    // SAFETY: WinSCard.
    let code = map_status(unsafe { SCardListReadersW(context, groups_ptr, Some(PWSTR(buf.as_mut_ptr())), &mut len) });
    if code != ReturnCode::Success {
        return Err(code);
    }
    Ok(parse_multi_wide(&buf[..len as usize]))
}

fn card_status_w(handle: usize) -> Result<(Vec<String>, CardState, CardProtocol, [u8; MAX_ATR], u32), ReturnCode> {
    let mut name_len = 0u32;
    let mut state = 0u32;
    let mut protocol = 0u32;
    let mut atr = [0u8; MAX_ATR];
    let mut atr_len = MAX_ATR as u32;
    // SAFETY: WinSCard.
    let probe = unsafe {
        SCardStatusW(
            handle,
            None,
            Some(&mut name_len),
            Some(&mut state),
            Some(&mut protocol),
            Some(atr.as_mut_ptr()),
            Some(&mut atr_len),
        )
    };
    let probe_code = map_status(probe);
    let mut names = Vec::new();
    if probe_code == ReturnCode::Success || probe_code == ReturnCode::InsufficientBuffer {
        if name_len > 0 {
            let mut name_buf = vec![0u16; name_len as usize];
            atr_len = MAX_ATR as u32;
            // SAFETY: WinSCard.
            let code = map_status(unsafe {
                SCardStatusW(
                    handle,
                    Some(PWSTR(name_buf.as_mut_ptr())),
                    Some(&mut name_len),
                    Some(&mut state),
                    Some(&mut protocol),
                    Some(atr.as_mut_ptr()),
                    Some(&mut atr_len),
                )
            });
            if code != ReturnCode::Success {
                return Err(code);
            }
            names = parse_multi_wide(&name_buf[..name_len as usize]);
        }
    } else {
        return Err(probe_code);
    }
    let mut wire = [0u8; MAX_ATR];
    let n = (atr_len as usize).min(MAX_ATR);
    wire[..n].copy_from_slice(&atr[..n]);
    let card_state = match state {
        1 => CardState::Absent,
        2 => CardState::Present,
        3 => CardState::Swallowed,
        4 => CardState::Powered,
        5 => CardState::Negotiable,
        6 => CardState::SpecificMode,
        _ => CardState::Unknown,
    };
    Ok((
        names,
        card_state,
        CardProtocol::from_bits_retain(protocol),
        wire,
        atr_len.min(MAX_ATR as u32),
    ))
}

fn run_transmit(req: DeviceControlRequest<ScardIoCtlCode>, handle: usize, call: TransmitCall) -> Outcome {
    let send_raw = pack_pci(&call.send_pci);
    // SAFETY: WinSCard.
    let send_pci = unsafe { &*(send_raw.as_ptr() as *const SCARD_IO_REQUEST) };
    let mut recv_raw = call.recv_pci.as_ref().map(|p| {
        let mut v = pack_pci(p);
        if v.len() < core::mem::size_of::<SCARD_IO_REQUEST>() {
            v.resize(core::mem::size_of::<SCARD_IO_REQUEST>(), 0);
        }
        v
    });
    let recv_pci_ptr = recv_raw.as_mut().map(|b| b.as_mut_ptr() as *mut SCARD_IO_REQUEST);

    if call.recv_buffer_is_null {
        let mut len = 0u32;
        // SAFETY: WinSCard.
        let status = unsafe {
            SCardTransmit(
                handle,
                send_pci,
                &call.send_buffer,
                recv_pci_ptr,
                core::ptr::null_mut(),
                &mut len,
            )
        };
        let code = map_status(status);
        let out_pci = recv_raw.as_ref().map(|b| unpack_pci(b));
        return Outcome::Message(complete_rpce(
            req,
            Box::new(TransmitReturn::recv_probe(code, out_pci, len)),
        ));
    }

    let recv_len = call.recv_length.min(MAX_RECV);
    let mut buf = vec![0u8; recv_len as usize];
    let mut len = recv_len;
    // SAFETY: WinSCard.
    let status = unsafe {
        SCardTransmit(
            handle,
            send_pci,
            &call.send_buffer,
            recv_pci_ptr,
            if recv_len == 0 {
                core::ptr::null_mut()
            } else {
                buf.as_mut_ptr()
            },
            &mut len,
        )
    };
    let code = map_status(status);
    if code != ReturnCode::Success {
        return Outcome::Message(complete_transmit(req, code, None, Vec::new()));
    }
    buf.truncate(len as usize);
    let out_pci = recv_raw.as_ref().map(|b| unpack_pci(b));
    Outcome::Message(complete_transmit(req, ReturnCode::Success, out_pci, buf))
}

fn pack_pci(pci: &SCardIORequest) -> Vec<u8> {
    let total = core::mem::size_of::<SCARD_IO_REQUEST>() + pci.extra_bytes.len();
    let mut buf = vec![0u8; total];
    let header = SCARD_IO_REQUEST {
        dwProtocol: pci.protocol.bits(),
        cbPciLength: total as u32,
    };
    // SAFETY: WinSCard.
    unsafe {
        core::ptr::write_unaligned(buf.as_mut_ptr() as *mut SCARD_IO_REQUEST, header);
    }
    if !pci.extra_bytes.is_empty() {
        buf[core::mem::size_of::<SCARD_IO_REQUEST>()..].copy_from_slice(&pci.extra_bytes);
    }
    buf
}

fn unpack_pci(buf: &[u8]) -> SCardIORequest {
    if buf.len() < core::mem::size_of::<SCARD_IO_REQUEST>() {
        return SCardIORequest {
            protocol: CardProtocol::SCARD_PROTOCOL_UNDEFINED,
            extra_bytes_length: 0,
            extra_bytes: Vec::new(),
        };
    }
    // SAFETY: WinSCard.
    let header = unsafe { core::ptr::read_unaligned(buf.as_ptr() as *const SCARD_IO_REQUEST) };
    let declared = (header.cbPciLength as usize).min(buf.len());
    let extra = if declared > core::mem::size_of::<SCARD_IO_REQUEST>() {
        buf[core::mem::size_of::<SCARD_IO_REQUEST>()..declared].to_vec()
    } else {
        Vec::new()
    };
    SCardIORequest {
        protocol: CardProtocol::from_bits_retain(header.dwProtocol),
        extra_bytes_length: extra.len(),
        extra_bytes: extra,
    }
}

fn fit_multi(
    values: Vec<String>,
    is_null: bool,
    capacity_chars: u32,
    cs: CharacterSet,
) -> (ReturnCode, Option<Vec<String>>, u32) {
    let c_bytes = encoded_multistring_len(&values, cs) as u32;
    let needed_chars = match cs {
        CharacterSet::Unicode => c_bytes / 2,
        CharacterSet::Ansi => c_bytes,
    };
    if is_null {
        return (ReturnCode::Success, None, c_bytes);
    }
    if capacity_chars < needed_chars {
        return (ReturnCode::InsufficientBuffer, None, c_bytes);
    }
    (ReturnCode::Success, Some(values), c_bytes)
}

fn buffer_too_small(req: DeviceControlRequest<ScardIoCtlCode>) -> SvcMessage {
    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
        req,
        NtStatus::BUFFER_TOO_SMALL,
        None,
    )))
}

fn complete_rpce(req: DeviceControlRequest<ScardIoCtlCode>, output: Box<dyn rpce::Encode>) -> SvcMessage {
    let (status, buf) = if output.size() <= req.output_buffer_length as usize {
        (NtStatus::SUCCESS, Some(output))
    } else {
        (NtStatus::BUFFER_TOO_SMALL, None)
    };
    SvcMessage::from(RdpdrPdu::DeviceControlResponse(DeviceControlResponse::new(
        req, status, buf,
    )))
}

fn complete_long(req: DeviceControlRequest<ScardIoCtlCode>, code: ReturnCode) -> SvcMessage {
    complete_rpce(req, Box::new(LongReturn::new(code)))
}
fn complete_establish(req: DeviceControlRequest<ScardIoCtlCode>, code: ReturnCode, ctx: ScardContext) -> SvcMessage {
    complete_rpce(req, Box::new(EstablishContextReturn::new(code, ctx)))
}
fn complete_list(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    v: Vec<String>,
    cs: CharacterSet,
) -> SvcMessage {
    complete_rpce(req, Box::new(ListReadersReturn::new(code, v, cs)))
}
fn complete_list_fit(
    req: DeviceControlRequest<ScardIoCtlCode>,
    v: Vec<String>,
    is_null: bool,
    capacity_chars: u32,
    cs: CharacterSet,
) -> SvcMessage {
    let (code, names, c_bytes) = fit_multi(v, is_null, capacity_chars, cs);
    match names {
        Some(n) => complete_list(req, code, n, cs),
        None => complete_rpce(req, Box::new(ListReadersReturn::probe(code, c_bytes, cs))),
    }
}
fn complete_gsc(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    states: Vec<ReaderStateCommonCall>,
) -> SvcMessage {
    complete_rpce(req, Box::new(GetStatusChangeReturn::new(code, states)))
}
fn complete_connect(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    handle: ScardHandle,
    proto: CardProtocol,
) -> SvcMessage {
    complete_rpce(req, Box::new(ConnectReturn::new(code, handle, proto)))
}
fn complete_transmit(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    pci: Option<SCardIORequest>,
    data: Vec<u8>,
) -> SvcMessage {
    complete_rpce(req, Box::new(TransmitReturn::new(code, pci, data)))
}

#[allow(clippy::too_many_arguments)]
fn complete_status_fit(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    names: Option<Vec<String>>,
    c_bytes: u32,
    state: CardState,
    proto: CardProtocol,
    atr: [u8; MAX_ATR],
    atr_len: u32,
    cs: CharacterSet,
) -> SvcMessage {
    match names {
        Some(n) => complete_rpce(
            req,
            Box::new(StatusReturn::new(code, n, state, proto, atr, atr_len, cs)),
        ),
        None => complete_rpce(
            req,
            Box::new(StatusReturn::names_probe(code, c_bytes, state, proto, atr, atr_len, cs)),
        ),
    }
}
fn complete_status_err(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    ioctl: ScardIoCtlCode,
) -> SvcMessage {
    complete_rpce(
        req,
        Box::new(StatusReturn::new(
            code,
            Vec::new(),
            CardState::Unknown,
            CardProtocol::SCARD_PROTOCOL_UNDEFINED,
            [0; MAX_ATR],
            0,
            status_cs(ioctl),
        )),
    )
}
fn complete_reconnect(req: DeviceControlRequest<ScardIoCtlCode>, code: ReturnCode, proto: CardProtocol) -> SvcMessage {
    complete_rpce(req, Box::new(ReconnectReturn::new(code, proto)))
}
fn complete_state(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    state: CardState,
    proto: CardProtocol,
    atr: Vec<u8>,
) -> SvcMessage {
    complete_rpce(req, Box::new(StateReturn::new(code, state, proto, atr)))
}
fn complete_state_fit(
    req: DeviceControlRequest<ScardIoCtlCode>,
    code: ReturnCode,
    state: CardState,
    proto: CardProtocol,
    atr: [u8; MAX_ATR],
    atr_len: u32,
    call: StateCall,
) -> SvcMessage {
    let full_len = atr_len.min(36);
    if call.atr_is_null || call.atr_length == 0 {
        return complete_rpce(req, Box::new(StateReturn::atr_probe(code, state, proto, full_len)));
    }
    if call.atr_length < atr_len {
        return complete_rpce(
            req,
            Box::new(StateReturn::atr_probe(
                ReturnCode::InsufficientBuffer,
                state,
                proto,
                full_len,
            )),
        );
    }
    complete_state(req, code, state, proto, atr[..atr_len as usize].to_vec())
}

fn list_cs(ioctl: ScardIoCtlCode) -> CharacterSet {
    match ioctl {
        ScardIoCtlCode::ListReadersA | ScardIoCtlCode::ListReaderGroupsA => CharacterSet::Ansi,
        _ => CharacterSet::Unicode,
    }
}
fn status_cs(ioctl: ScardIoCtlCode) -> CharacterSet {
    match ioctl {
        ScardIoCtlCode::StatusA => CharacterSet::Ansi,
        _ => CharacterSet::Unicode,
    }
}

fn map_status(status: i32) -> ReturnCode {
    let code = status as u32;
    match code {
        0 => ReturnCode::Success,
        // SAFETY: WinSCard.
        0x8010_0001..=0x8010_0034 | 0x8010_0065..=0x8010_0072 => unsafe {
            core::mem::transmute::<u32, ReturnCode>(code)
        },
        other => {
            warn!(status = other, "Unmapped WinSCard status; treating as unknown error");
            ReturnCode::UnknownError
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    let mut w: Vec<u16> = value.encode_utf16().collect();
    w.push(0);
    w
}
fn multi_wide(values: &[String]) -> Vec<u16> {
    let mut out = Vec::new();
    for v in values {
        out.extend(v.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}
fn parse_multi_wide(buffer: &[u16]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < buffer.len() {
        if buffer[start] == 0 {
            break;
        }
        let end = buffer[start..]
            .iter()
            .position(|&c| c == 0)
            .map(|r| start + r)
            .unwrap_or(buffer.len());
        out.push(String::from_utf16_lossy(&buffer[start..end]));
        start = end + 1;
    }
    out
}
fn reader_state_w(state: &ReaderState, reader: &[u16]) -> SCARD_READERSTATEW {
    SCARD_READERSTATEW {
        szReader: PCWSTR(reader.as_ptr()),
        pvUserData: core::ptr::null_mut(),
        dwCurrentState: SCARD_STATE(state.common.current_state.bits()),
        dwEventState: SCARD_STATE(state.common.event_state.bits()),
        cbAtr: state.common.atr_length.min(36),
        rgbAtr: state.common.atr,
    }
}
fn reader_states_from_native(states: Vec<SCARD_READERSTATEW>) -> Vec<ReaderStateCommonCall> {
    states
        .into_iter()
        .map(|s| ReaderStateCommonCall {
            current_state: CardStateFlags::from_bits_retain(s.dwCurrentState.0),
            event_state: CardStateFlags::from_bits_retain(s.dwEventState.0),
            atr_length: s.cbAtr.min(36),
            atr: s.rgbAtr,
        })
        .collect()
}

fn unsupported(ioctl: ScardIoCtlCode, call: ScardCall) -> Box<dyn rpce::Encode> {
    let code = ReturnCode::UnsupportedFeature;
    let undef = CardProtocol::SCARD_PROTOCOL_UNDEFINED;
    match call {
        ScardCall::LocateCardsCall(_) | ScardCall::LocateCardsByAtrCall(_) => {
            Box::new(GetStatusChangeReturn::new(code, Vec::new()))
        }
        ScardCall::ControlCall(_) => Box::new(ControlReturn::new(code, Vec::new())),
        ScardCall::GetAttribCall(_) => Box::new(GetAttribReturn::new(code, Vec::new())),
        ScardCall::GetTransmitCountCall(_) => Box::new(GetTransmitCountReturn::new(code, 0)),
        ScardCall::GetDeviceTypeIdCall(_) => Box::new(GetDeviceTypeIdReturn::new(code, 0)),
        ScardCall::ReadCacheCall(_) => Box::new(ReadCacheReturn::new(code, Vec::new())),
        ScardCall::GetReaderIconCall(_) => Box::new(GetReaderIconReturn::new(code, Vec::new())),
        ScardCall::EstablishContextCall(_) => Box::new(EstablishContextReturn::new(code, ScardContext::from_native(0))),
        ScardCall::ListReaderGroupsCall(_) | ScardCall::ListReadersCall(_) => {
            Box::new(ListReadersReturn::new(code, Vec::new(), list_cs(ioctl)))
        }
        ScardCall::GetStatusChangeCall(_) => Box::new(GetStatusChangeReturn::new(code, Vec::new())),
        ScardCall::ConnectCall(_) => Box::new(ConnectReturn::new(
            code,
            ScardHandle::from_native(ScardContext::from_native(0), 0),
            undef,
        )),
        ScardCall::ReconnectCall(_) => Box::new(ReconnectReturn::new(code, undef)),
        ScardCall::TransmitCall(_) => Box::new(TransmitReturn::new(code, None, Vec::new())),
        ScardCall::StatusCall(_) => Box::new(StatusReturn::new(
            code,
            Vec::new(),
            CardState::Unknown,
            undef,
            [0u8; 32],
            0,
            status_cs(ioctl),
        )),
        ScardCall::StateCall(_) => Box::new(StateReturn::new(code, CardState::Unknown, undef, Vec::new())),
        _ => Box::new(LongReturn::new(code)),
    }
}

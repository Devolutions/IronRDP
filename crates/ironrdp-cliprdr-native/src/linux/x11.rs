//! XFixes selection notifications and ICCCM selection transfers.
//!
//! Every conversion uses its own requestor window, so a timed-out owner's late
//! reply cannot satisfy a newer request. Large selections use the INCR protocol.
//!
//! See [ICCCM section 2.7.2, INCR Properties].
//!
//! [ICCCM section 2.7.2, INCR Properties]: https://www.x.org/releases/X11R7.7/doc/xorg-docs/icccm/icccm.html

use core::time::Duration;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Instant;

use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property,
    SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

use super::os::{OsClipboard, PNG, TEXT, Transfer};
use super::worker::{Command, MAX_TRANSFER_BYTES, PASTE_TIMEOUT};

// Fits the X11 core request size even without BIG-REQUESTS.
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_TRANSFERS: usize = 16;
type ReadReply = mpsc::Sender<Result<Option<Vec<u8>>, String>>;
type XResult<T> = Result<T, Box<dyn core::error::Error + Send + Sync>>;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD, TARGETS, TIMESTAMP, INCR, UTF8_STRING,
        TEXT_UTF8: b"text/plain;charset=utf-8",
        TEXT_PLAIN: b"text/plain",
        PNG: b"image/png",
        TRANSFER: b"IRONRDP_CLIPBOARD_TRANSFER",
    }
}

enum Request {
    Read(String, ReadReply),
    Offer(Vec<String>, u64, mpsc::Sender<Result<(), String>>),
    IsOwner(mpsc::Sender<bool>),
    Clear,
    Complete(SelectionRequestEvent, u64, Option<Vec<u8>>),
    Shutdown,
}

pub struct X11Clipboard {
    commands: mpsc::Sender<Request>,
    mime_types: Arc<Mutex<Vec<String>>>,
    thread: Option<JoinHandle<()>>,
}

impl X11Clipboard {
    pub fn open(events: mpsc::Sender<Command>) -> Result<Self, String> {
        let (commands, receiver) = mpsc::channel();
        let mime_types = Arc::new(Mutex::new(Vec::new()));
        let state =
            State::connect(events, commands.clone(), Arc::clone(&mime_types)).map_err(|error| error.to_string())?;
        let thread = std::thread::Builder::new()
            .name("cliprdr-x11".into())
            .spawn(move || {
                if let Err(error) = state.run(receiver) {
                    tracing::warn!(%error, "X11 clipboard worker stopped");
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            commands,
            mime_types,
            thread: Some(thread),
        })
    }
}

impl OsClipboard for X11Clipboard {
    fn is_owner(&self) -> bool {
        let (sender, receiver) = mpsc::channel();
        if self.commands.send(Request::IsOwner(sender)).is_err() {
            return false;
        }
        receiver.recv_timeout(PASTE_TIMEOUT).unwrap_or(false)
    }

    fn mime_types(&self) -> Vec<String> {
        self.mime_types
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn read(&mut self, mime: &str) -> Result<Option<Vec<u8>>, String> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .send(Request::Read(mime.to_owned(), sender))
            .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(PASTE_TIMEOUT + Duration::from_secs(1))
            .map_err(|error| error.to_string())?
    }

    fn offer(&mut self, mimes: &[String], generation: u64) -> Result<(), String> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .send(Request::Offer(mimes.to_vec(), generation, sender))
            .map_err(|error| error.to_string())?;
        receiver
            .recv_timeout(PASTE_TIMEOUT)
            .map_err(|error| error.to_string())?
    }

    fn clear(&mut self) -> Result<(), String> {
        self.commands.send(Request::Clear).map_err(|error| error.to_string())
    }
}

impl Drop for X11Clipboard {
    fn drop(&mut self) {
        let _ = self.commands.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum ReadKind {
    Targets,
    Data(ReadReply),
}

struct Incoming {
    target: Atom,
    kind: ReadKind,
    bytes: Vec<u8>,
    incremental: bool,
    deadline: Instant,
}

struct PendingPaste {
    request: SelectionRequestEvent,
    deadline: Instant,
    expired: bool,
}

struct Outgoing {
    target: Atom,
    data: Vec<u8>,
    offset: usize,
    deadline: Instant,
}

struct State {
    connection: RustConnection,
    window: Window,
    root: Window,
    atoms: Atoms,
    events: mpsc::Sender<Command>,
    commands: mpsc::Sender<Request>,
    mime_types: Arc<Mutex<Vec<String>>>,
    targets: Vec<Atom>,
    owner: Window,
    owns: bool,
    generation: u64,
    timestamp: u32,
    incoming: HashMap<Window, Incoming>,
    outgoing: HashMap<(Window, Atom), Outgoing>,
    pending_pastes: HashMap<(Window, Atom), PendingPaste>,
    deferred: VecDeque<Event>,
}

impl State {
    fn connect(
        events: mpsc::Sender<Command>,
        commands: mpsc::Sender<Request>,
        mime_types: Arc<Mutex<Vec<String>>>,
    ) -> XResult<Self> {
        let (connection, screen) = x11rb::connect(None)?;
        connection.xfixes_query_version(5, 0)?.reply()?;
        let root = connection.setup().roots[screen].root;
        let window = connection.generate_id()?;
        connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )?
            .check()?;
        let atoms = Atoms::new(&connection)?.reply()?;
        connection
            .xfixes_select_selection_input(
                window,
                atoms.CLIPBOARD,
                SelectionEventMask::SET_SELECTION_OWNER
                    | SelectionEventMask::SELECTION_WINDOW_DESTROY
                    | SelectionEventMask::SELECTION_CLIENT_CLOSE,
            )?
            .check()?;
        let owner = connection.get_selection_owner(atoms.CLIPBOARD)?.reply()?.owner;
        let mut state = Self {
            connection,
            window,
            root,
            atoms,
            events,
            commands,
            mime_types,
            targets: Vec::new(),
            owner,
            owns: false,
            generation: 0,
            timestamp: CURRENT_TIME,
            incoming: HashMap::new(),
            outgoing: HashMap::new(),
            pending_pastes: HashMap::new(),
            deferred: VecDeque::new(),
        };
        state.timestamp = state.server_timestamp()?;
        if owner != NONE {
            state.begin_read(state.atoms.TARGETS, ReadKind::Targets)?;
        }
        state.connection.flush()?;
        Ok(state)
    }

    fn run(mut self, receiver: mpsc::Receiver<Request>) -> XResult<()> {
        loop {
            for _ in 0..64 {
                let event = match self.deferred.pop_front() {
                    Some(event) => Some(event),
                    None => self.connection.poll_for_event()?,
                };
                let Some(event) = event else {
                    break;
                };
                // BadWindow from an application that closed mid-paste is local to
                // that transfer; it must not stop clipboard synchronization.
                if let Err(error) = self.event(event) {
                    tracing::debug!(%error, "X11 clipboard event failed");
                }
            }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(Request::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(request) => {
                    if let Err(error) = self.request(request) {
                        tracing::debug!(%error, "X11 clipboard request failed");
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            let expired: Vec<_> = self
                .incoming
                .iter()
                .filter(|(_, read)| read.deadline <= now)
                .map(|(window, _)| *window)
                .collect();
            for window in expired {
                self.finish_read(window, Err("X11 clipboard read timed out".into()))?;
            }
            self.outgoing.retain(|_, write| now < write.deadline);
            let mut refused = Vec::new();
            for paste in self.pending_pastes.values_mut() {
                if !paste.expired && paste.deadline <= now {
                    paste.expired = true;
                    refused.push(paste.request);
                }
            }
            for request in refused {
                let _ = self.notify(request, NONE);
            }
            // Keep expired pending slots until Complete is consumed, so cached
            // responses already queued on the channel remain within the budget.
            self.connection.flush()?;
        }
        self.clear()?;
        Ok(())
    }

    fn request(&mut self, request: Request) -> XResult<()> {
        match request {
            Request::Read(mime, sender) => {
                let target = if mime == PNG {
                    self.targets.contains(&self.atoms.PNG).then_some(self.atoms.PNG)
                } else {
                    [
                        self.atoms.UTF8_STRING,
                        self.atoms.TEXT_UTF8,
                        self.atoms.TEXT_PLAIN,
                        AtomEnum::STRING.into(),
                    ]
                    .into_iter()
                    .find(|target| self.targets.contains(target))
                };
                if let Some(target) = target.filter(|_| !self.owns && self.owner != NONE) {
                    self.begin_read(target, ReadKind::Data(sender))?;
                } else {
                    let _ = sender.send(Ok(None));
                }
            }
            Request::IsOwner(sender) => {
                let owner = self
                    .connection
                    .get_selection_owner(self.atoms.CLIPBOARD)?
                    .reply()?
                    .owner;
                let _ = sender.send(owner == self.window);
            }
            Request::Offer(mimes, generation, sender) => {
                let result = (|| -> XResult<()> {
                    self.generation = generation;
                    self.targets.clear();
                    for mime in mimes {
                        if mime == PNG {
                            self.targets.push(self.atoms.PNG);
                        }
                        if mime == TEXT {
                            self.targets
                                .extend([self.atoms.UTF8_STRING, self.atoms.TEXT_UTF8, self.atoms.TEXT_PLAIN]);
                        }
                    }
                    self.timestamp = self.server_timestamp()?;
                    self.connection
                        .set_selection_owner(self.window, self.atoms.CLIPBOARD, self.timestamp)?
                        .check()?;
                    self.owner = self
                        .connection
                        .get_selection_owner(self.atoms.CLIPBOARD)?
                        .reply()?
                        .owner;
                    self.owns = self.owner == self.window;
                    let readers: Vec<_> = self.incoming.keys().copied().collect();
                    for window in readers {
                        self.finish_read(window, Err("clipboard selection changed while reading".into()))?;
                    }
                    if !self.owns {
                        return Err("clipboard ownership was not acquired".into());
                    }
                    Ok(())
                })();
                let _ = sender.send(result.map_err(|error| error.to_string()));
            }
            Request::Clear => self.clear()?,
            Request::Complete(request, generation, data) => {
                let property = if request.property == NONE {
                    request.target
                } else {
                    request.property
                };
                let Some(pending) = self.pending_pastes.remove(&(request.requestor, property)) else {
                    return Ok(());
                };
                if pending.expired {
                    return Ok(());
                }
                if generation != self.generation || !self.owns {
                    self.notify(request, NONE)?;
                } else if let Some(data) = data.filter(|d| d.len() <= MAX_TRANSFER_BYTES) {
                    let property = if request.property == NONE {
                        request.target
                    } else {
                        request.property
                    };
                    if data.len() <= CHUNK_BYTES {
                        self.connection
                            .change_property8(PropMode::REPLACE, request.requestor, property, request.target, &data)?
                            .check()?;
                        self.notify(request, property)?;
                    } else if self.outgoing.len() < MAX_TRANSFERS {
                        self.connection
                            .change_window_attributes(
                                request.requestor,
                                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                            )?
                            .check()?;
                        let length = u32::try_from(data.len())?;
                        self.connection
                            .change_property32(
                                PropMode::REPLACE,
                                request.requestor,
                                property,
                                self.atoms.INCR,
                                &[length],
                            )?
                            .check()?;
                        self.outgoing.insert(
                            (request.requestor, property),
                            Outgoing {
                                target: request.target,
                                data,
                                offset: 0,
                                deadline: Instant::now() + PASTE_TIMEOUT,
                            },
                        );
                        self.notify(request, property)?;
                    } else {
                        self.notify(request, NONE)?;
                    }
                } else {
                    self.notify(request, NONE)?;
                }
            }
            Request::Shutdown => {}
        }
        Ok(())
    }

    // ICCCM uses the event timestamp that triggered ownership. RDP commands
    // have no X timestamp, so obtain one from PropertyNotify and retain all
    // other events for normal dispatch.
    fn server_timestamp(&mut self) -> XResult<u32> {
        self.connection
            .change_property8(
                PropMode::APPEND,
                self.window,
                self.atoms.TIMESTAMP,
                AtomEnum::INTEGER,
                &[],
            )?
            .check()?;
        self.connection.flush()?;
        loop {
            let event = self.connection.wait_for_event()?;
            if let Event::PropertyNotify(property) = &event {
                if property.window == self.window && property.atom == self.atoms.TIMESTAMP {
                    return Ok(property.time);
                }
            }
            self.deferred.push_back(event);
        }
    }

    fn clear(&mut self) -> XResult<()> {
        if self
            .connection
            .get_selection_owner(self.atoms.CLIPBOARD)?
            .reply()?
            .owner
            == self.window
        {
            self.connection
                .set_selection_owner(NONE, self.atoms.CLIPBOARD, self.timestamp)?
                .check()?;
        }
        self.owns = false;
        Ok(())
    }

    fn begin_read(&mut self, target: Atom, kind: ReadKind) -> XResult<()> {
        let window = self.connection.generate_id()?;
        self.connection
            .create_window(
                COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )?
            .check()?;
        self.connection
            .convert_selection(
                window,
                self.atoms.CLIPBOARD,
                target,
                self.atoms.TRANSFER,
                self.timestamp,
            )?
            .check()?;
        self.incoming.insert(
            window,
            Incoming {
                target,
                kind,
                bytes: Vec::new(),
                incremental: false,
                deadline: Instant::now() + PASTE_TIMEOUT,
            },
        );
        Ok(())
    }

    fn event(&mut self, event: Event) -> XResult<()> {
        match event {
            Event::XfixesSelectionNotify(event) if event.selection == self.atoms.CLIPBOARD => {
                if self
                    .connection
                    .get_selection_owner(self.atoms.CLIPBOARD)?
                    .reply()?
                    .owner
                    != event.owner
                {
                    return Ok(());
                }
                self.owner = event.owner;
                if event.owner == self.window {
                    self.owns = true;
                    return Ok(());
                }
                self.timestamp = event.selection_timestamp;
                self.owns = false;
                self.targets.clear();
                let readers: Vec<_> = self.incoming.keys().copied().collect();
                for window in readers {
                    self.finish_read(window, Err("clipboard selection changed while reading".into()))?;
                }
                if event.owner != NONE {
                    self.begin_read(self.atoms.TARGETS, ReadKind::Targets)?;
                } else {
                    self.mime_types
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clear();
                    let _ = self.events.send(Command::LocalChanged);
                }
            }
            Event::SelectionNotify(event)
                if event.selection == self.atoms.CLIPBOARD
                    && self
                        .incoming
                        .get(&event.requestor)
                        .is_some_and(|read| read.target == event.target) =>
            {
                if event.property != self.atoms.TRANSFER {
                    self.finish_read(event.requestor, Err("clipboard conversion refused".into()))?;
                } else {
                    self.read_property(event.requestor)?;
                }
            }
            Event::PropertyNotify(event) => {
                if event.state == Property::NEW_VALUE && self.incoming.get(&event.window).is_some_and(|r| r.incremental)
                {
                    self.read_property(event.window)?;
                }
                if event.state == Property::DELETE {
                    if let Some(mut write) = self.outgoing.remove(&(event.window, event.atom)) {
                        let end = write.data.len().min(write.offset.saturating_add(CHUNK_BYTES));
                        let chunk = &write.data[write.offset..end];
                        self.connection
                            .change_property8(PropMode::REPLACE, event.window, event.atom, write.target, chunk)?
                            .check()?;
                        if !chunk.is_empty() {
                            write.offset = end;
                            write.deadline = Instant::now() + PASTE_TIMEOUT;
                            self.outgoing.insert((event.window, event.atom), write);
                        }
                    }
                }
            }
            Event::SelectionRequest(request) => self.selection_request(request)?,
            Event::SelectionClear(event) if event.selection == self.atoms.CLIPBOARD => {
                // A clear from an earlier owner can be queued while Offer is
                // obtaining its timestamp. Consult the server before applying
                // it to a selection that has since been reacquired.
                self.owns = self
                    .connection
                    .get_selection_owner(self.atoms.CLIPBOARD)?
                    .reply()?
                    .owner
                    == self.window;
            }
            _ => {}
        }
        Ok(())
    }

    fn read_property(&mut self, window: Window) -> XResult<()> {
        let reply = self
            .connection
            .get_property(
                true,
                window,
                self.atoms.TRANSFER,
                AtomEnum::ANY,
                0,
                u32::try_from(MAX_TRANSFER_BYTES / 4 + 1)?,
            )?
            .reply()?;
        let Some(read) = self.incoming.get_mut(&window) else {
            return Ok(());
        };
        if reply.type_ == self.atoms.INCR {
            let size = reply.value32().and_then(|mut values| values.next()).unwrap_or(u32::MAX);
            if read.incremental || MAX_TRANSFER_BYTES < usize::try_from(size)? {
                self.finish_read(window, Err("X11 clipboard data exceeds the size limit".into()))?;
            } else {
                read.incremental = true;
            }
            return Ok(());
        }
        if reply.bytes_after != 0 || MAX_TRANSFER_BYTES < read.bytes.len().saturating_add(reply.value.len()) {
            self.finish_read(window, Err("X11 clipboard data exceeds the size limit".into()))?;
            return Ok(());
        }
        let expected_type = if read.target == self.atoms.TARGETS {
            AtomEnum::ATOM.into()
        } else {
            read.target
        };
        let expected_format = if read.target == self.atoms.TARGETS { 32 } else { 8 };
        if reply.type_ != expected_type || reply.format != expected_format {
            self.finish_read(window, Err("invalid X11 clipboard property type".into()))?;
            return Ok(());
        }
        let finished = !read.incremental || reply.value.is_empty();
        read.bytes.extend_from_slice(&reply.value);
        if finished {
            let bytes = core::mem::take(&mut read.bytes);
            self.finish_read(window, Ok(bytes))?;
        }
        Ok(())
    }

    fn finish_read(&mut self, window: Window, result: Result<Vec<u8>, String>) -> XResult<()> {
        let Some(read) = self.incoming.remove(&window) else {
            return Ok(());
        };
        self.connection.destroy_window(window)?.check()?;
        match read.kind {
            ReadKind::Data(sender) => {
                let result = result.map(|bytes| {
                    if read.target == AtomEnum::STRING.into() {
                        bytes.into_iter().map(char::from).collect::<String>().into_bytes()
                    } else {
                        bytes
                    }
                });
                let _ = sender.send(result.map(Some));
            }
            ReadKind::Targets => {
                if self.owns {
                    return Ok(());
                }
                self.targets = result
                    .unwrap_or_default()
                    .chunks_exact(4)
                    .map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
                let mut mimes = Vec::new();
                if [
                    self.atoms.UTF8_STRING,
                    self.atoms.TEXT_UTF8,
                    self.atoms.TEXT_PLAIN,
                    AtomEnum::STRING.into(),
                ]
                .iter()
                .any(|target| self.targets.contains(target))
                {
                    mimes.push(TEXT.to_owned());
                }
                if self.targets.contains(&self.atoms.PNG) {
                    mimes.push(PNG.to_owned());
                }
                *self
                    .mime_types
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = mimes;
                let _ = self.events.send(Command::LocalChanged);
            }
        }
        Ok(())
    }

    fn selection_request(&mut self, request: SelectionRequestEvent) -> XResult<()> {
        if request.selection != self.atoms.CLIPBOARD || !self.owns {
            return self.notify(request, NONE);
        }
        // X timestamps wrap at 32 bits. We cannot render a selection from
        // before our current ownership interval, including queued old requests.
        if request.time != CURRENT_TIME && 0x7fff_ffff < request.time.wrapping_sub(self.timestamp) {
            return self.notify(request, NONE);
        }
        let property = if request.property == NONE {
            request.target
        } else {
            request.property
        };
        if request.target == self.atoms.TARGETS {
            let mut targets = self.targets.clone();
            targets.extend([self.atoms.TARGETS, self.atoms.TIMESTAMP]);
            self.connection
                .change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::ATOM, &targets)?
                .check()?;
            return self.notify(request, property);
        }
        if request.target == self.atoms.TIMESTAMP {
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::INTEGER,
                    &[self.timestamp],
                )?
                .check()?;
            return self.notify(request, property);
        }
        if !self.targets.contains(&request.target) {
            return self.notify(request, NONE);
        }
        // Reserve before notifying the protocol worker: cached replies can
        // otherwise enqueue unbounded full clipboard buffers before we consume
        // their Complete commands. A slot covers waiting, queued and INCR data.
        let key = (request.requestor, property);
        if MAX_TRANSFERS <= self.pending_pastes.len() + self.outgoing.len()
            || self.pending_pastes.contains_key(&key)
            || self.outgoing.contains_key(&key)
        {
            return self.notify(request, NONE);
        }
        self.pending_pastes.insert(
            key,
            PendingPaste {
                request,
                deadline: Instant::now() + PASTE_TIMEOUT,
                expired: false,
            },
        );
        let mime = if request.target == self.atoms.PNG { PNG } else { TEXT };
        let commands = self.commands.clone();
        let generation = self.generation;
        let transfer = Transfer::new(mime.to_owned(), generation, move |data| {
            let _ = commands.send(Request::Complete(request, generation, data));
        });
        let _ = self.events.send(Command::Paste(transfer));
        Ok(())
    }

    fn notify(&self, request: SelectionRequestEvent, property: Atom) -> XResult<()> {
        let notify = SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property,
        };
        self.connection
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)?
            .check()?;
        Ok(())
    }
}

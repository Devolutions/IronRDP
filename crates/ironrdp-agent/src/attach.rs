//! `ironrdp-agent attach`: an interactive terminal view of the daemon's session.
//!
//! The frame is polled with screenshot requests and drawn with a terminal graphics protocol.
//! Outside the local menu, terminal mouse and keyboard events are forwarded as input requests.

use core::ops::Range;
use core::time::Duration;
use std::collections::HashSet;
use std::io::Write as _;

use anyhow::Context as _;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode,
    KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use crossterm::{cursor, execute, queue, terminal};
use ironrdp_input::MouseButton;
use ironrdp_rpc::ipc::{ConnState, KeyInput, MAX_UNICODE_TEXT_CHARS, Payload, Request, Response, StatusInfo};
use ironrdp_rpc::transport::{self, Endpoint};

use crate::terminal_image::{self, Protocol};

/// Delay before refreshing the frame after forwarding input, so the result shows up quickly.
const INPUT_REFRESH_DELAY: Duration = Duration::from_millis(80);

/// Quiet period after the last terminal resize before `--fit` resizes the remote desktop.
const FIT_DEBOUNCE: Duration = Duration::from_millis(500);

/// How long a status-line message stays visible.
const MESSAGE_DURATION: Duration = Duration::from_secs(4);

const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Ctrl+] detaches, like telnet. Terminals report it as `]`, its control code, or `5`.
const HOTKEY_DETACH: [char; 3] = [']', '\x1d', '5'];

/// Ctrl+\ resizes the remote desktop to the terminal. Terminals report it as `\`, its control
/// code, or `4`.
const HOTKEY_FIT: [char; 3] = ['\\', '\x1c', '4'];

/// Kitty control keys that replace the previous frame in place and suppress terminal replies.
const KITTY_KEYS: &str = "i=1,p=1,q=2,C=1,";

/// One wheel notch, in the units `Request::Wheel` expects.
const WHEEL_NOTCH: i16 = 120;

const SCANCODE_CTRL: u16 = 0x1D;
const SCANCODE_SHIFT: u16 = 0x2A;
const SCANCODE_ALT: u16 = 0x38;
const SCANCODE_WIN: u16 = 0xE05B;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiAction {
    Menu,
    Fit,
    ToggleAutoFit,
    Refresh,
    Detach,
    Disconnect,
    ConfirmDisconnect,
    CancelDisconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuMode {
    Closed,
    Options,
    ConfirmDisconnect,
}

struct Footer {
    text: String,
    buttons: Vec<(Range<u16>, UiAction)>,
}

impl Footer {
    fn new(
        columns: u16,
        menu_open: bool,
        auto_fit: bool,
        resolution: Option<(u16, u16)>,
        message: Option<&str>,
    ) -> Self {
        let buttons = [
            (if menu_open { "[Back]" } else { "[Menu]" }, UiAction::Menu),
            ("[Fit]", UiAction::Fit),
            ("[Detach]", UiAction::Detach),
        ];
        let visible: &[usize] = if columns >= 21 {
            &[0, 1, 2]
        } else if columns >= 15 {
            &[0, 2]
        } else if columns >= 8 {
            &[2]
        } else {
            &[]
        };
        let mut text = String::new();
        let mut hits = Vec::new();
        for &index in visible {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = u16::try_from(text.len()).expect("footer fits in terminal columns");
            text.push_str(buttons[index].0);
            let end = u16::try_from(text.len()).expect("footer fits in terminal columns");
            hits.push((start..end, buttons[index].1));
        }
        if !text.is_empty() {
            text.push_str(" | ");
        }
        text.push_str(if auto_fit { "Auto-fit: ON" } else { "Auto-fit: OFF" });
        if let Some((width, height)) = resolution {
            text.push_str(&format!("  {width}x{height}"));
        }
        if let Some(message) = message {
            text.push_str(" | ");
            text.extend(message.chars().map(|ch| if ch.is_control() { ' ' } else { ch }));
        }
        Self {
            text: text.chars().take(usize::from(columns)).collect(),
            buttons: hits,
        }
    }

    fn action_at(&self, column: u16) -> Option<UiAction> {
        self.buttons
            .iter()
            .find_map(|(range, action)| range.contains(&column).then_some(*action))
    }
}

fn menu_action_at(row: u16, confirming_disconnect: bool) -> Option<UiAction> {
    if confirming_disconnect {
        return match row {
            2 => Some(UiAction::ConfirmDisconnect),
            3 => Some(UiAction::CancelDisconnect),
            _ => None,
        };
    }
    match row {
        2 => Some(UiAction::Fit),
        3 => Some(UiAction::ToggleAutoFit),
        4 => Some(UiAction::Refresh),
        5 => Some(UiAction::Detach),
        6 => Some(UiAction::Disconnect),
        7 => Some(UiAction::Menu),
        _ => None,
    }
}

fn menu_key_action(key: KeyEvent, confirming_disconnect: bool) -> Option<UiAction> {
    if key.kind != KeyEventKind::Press || !key.modifiers.is_empty() {
        return None;
    }
    if confirming_disconnect {
        return match key.code {
            KeyCode::Char('y') => Some(UiAction::ConfirmDisconnect),
            KeyCode::Char('n') | KeyCode::Esc => Some(UiAction::CancelDisconnect),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Char('1') => Some(UiAction::Fit),
        KeyCode::Char('2') => Some(UiAction::ToggleAutoFit),
        KeyCode::Char('3') => Some(UiAction::Refresh),
        KeyCode::Char('4') => Some(UiAction::Detach),
        KeyCode::Char('5') => Some(UiAction::Disconnect),
        KeyCode::Esc => Some(UiAction::Menu),
        _ => None,
    }
}

pub(crate) struct Options {
    pub(crate) protocol: Protocol,
    pub(crate) interval: Duration,
    pub(crate) cell_size: Option<(u32, u32)>,
    /// Resize the remote desktop to the terminal on attach and whenever the terminal is resized.
    pub(crate) fit: bool,
}

pub(crate) async fn run(endpoint: &Endpoint, options: Options) -> anyhow::Result<()> {
    use std::io::IsTerminal as _;
    use tokio::time::Instant;

    anyhow::ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "attach requires an interactive terminal"
    );
    let protocol = terminal_image::resolve_protocol(options.protocol)?;
    let cell_size = options
        .cell_size
        .or_else(terminal_image::cell_size)
        .unwrap_or(terminal_image::FALLBACK_CELL_SIZE);

    // Fail before taking over the terminal when there is nothing to show.
    if let Some(status) = ended_session(endpoint).await? {
        anyhow::bail!("no active session (state: {:?})", status.state);
    }
    let mut frame = Some(
        fetch_frame(endpoint)
            .await?
            .map_err(|message| anyhow::anyhow!("{message}"))?,
    );

    let _guard = TerminalGuard::enter(protocol)?;

    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while let Ok(event) = event::read() {
            if events_tx.send(event).is_err() {
                break;
            }
        }
    });

    let mut view = View {
        protocol,
        cell_size,
        layout: None,
        resolution: None,
        drawn_png: Vec::new(),
        message: None,
        menu_mode: MenuMode::Closed,
    };
    let refresh = tokio::time::sleep(Duration::ZERO);
    tokio::pin!(refresh);
    // Terminal resizes arrive in bursts while a window is dragged, so fitting is debounced.
    let mut auto_fit = options.fit;
    let mut fit_at = auto_fit.then(Instant::now);
    let mut local_click = None;
    let mut remote_pressed = HashSet::new();
    let mut disconnected = false;
    let mut ended = None;
    let mut session_poll = tokio::time::interval_at(Instant::now() + SESSION_POLL_INTERVAL, SESSION_POLL_INTERVAL);
    session_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = session_poll.tick() => {
                // Screenshots may retain the last frame, even after the session ends.
                if let Some(status) = ended_session(endpoint).await? {
                    ended = Some(status);
                    break;
                }
            }
            event = events.recv() => {
                let Some(event) = event else { break };
                let mut action = None;
                let requests = match event {
                    Event::Key(key) if is_hotkey(&key, HOTKEY_DETACH) => {
                        release_pressed_buttons(endpoint, &mut remote_pressed).await?;
                        break;
                    }
                    Event::Key(key) if is_hotkey(&key, HOTKEY_FIT) => {
                        action = Some(UiAction::Fit);
                        Vec::new()
                    }
                    Event::Key(key) if view.menu_mode != MenuMode::Closed => {
                        action = menu_key_action(key, view.menu_mode == MenuMode::ConfirmDisconnect);
                        Vec::new()
                    }
                    Event::Key(key) => {
                        let events = key_requests(key);
                        if events.is_empty() { Vec::new() } else { vec![Request::KeyBatch { events }] }
                    }
                    Event::Mouse(mouse) => {
                        let (columns, rows) = terminal::size()?;
                        let footer = view.footer(auto_fit, columns);
                        let (clicked, requests) = route_mouse(
                            mouse, rows, &footer, view.menu_mode, view.layout, &mut local_click, &mut remote_pressed
                        );
                        action = clicked;
                        requests
                    }
                    Event::Paste(_) if view.menu_mode != MenuMode::Closed => Vec::new(),
                    Event::Paste(text) => paste_requests(&text),
                    Event::Resize(..) => {
                        view.layout = None;
                        if view.menu_mode != MenuMode::Closed {
                            view.draw_menu(auto_fit)?;
                        } else {
                            refresh.as_mut().reset(Instant::now());
                        }
                        if auto_fit {
                            fit_at = Some(Instant::now() + FIT_DEBOUNCE);
                        }
                        continue;
                    }
                    _ => continue,
                };
                if let Some(action) = action {
                    match action {
                        UiAction::Menu => {
                            match view.menu_mode {
                                MenuMode::ConfirmDisconnect => {
                                    view.menu_mode = MenuMode::Options;
                                    view.draw_menu(auto_fit)?;
                                }
                                MenuMode::Options => {
                                    view.close_menu(auto_fit)?;
                                    refresh.as_mut().reset(Instant::now());
                                }
                                MenuMode::Closed => {
                                    view.menu_mode = MenuMode::Options;
                                    view.draw_menu(auto_fit)?;
                                }
                            }
                        }
                        UiAction::Fit => {
                            if view.menu_mode != MenuMode::Closed {
                                view.close_menu(auto_fit)?;
                            }
                            fit_at = Some(Instant::now());
                            refresh.as_mut().reset(Instant::now());
                        }
                        UiAction::ToggleAutoFit => {
                            auto_fit = !auto_fit;
                            fit_at = auto_fit.then(Instant::now);
                            if view.menu_mode != MenuMode::Closed {
                                view.draw_menu(auto_fit)?;
                            } else {
                                view.draw_status(auto_fit)?;
                            }
                        }
                        UiAction::Refresh => {
                            if view.menu_mode != MenuMode::Closed {
                                view.close_menu(auto_fit)?;
                            }
                            view.layout = None;
                            view.drawn_png.clear();
                            refresh.as_mut().reset(Instant::now());
                        }
                        UiAction::Detach => {
                            release_pressed_buttons(endpoint, &mut remote_pressed).await?;
                            break;
                        }
                        UiAction::Disconnect => {
                            view.menu_mode = MenuMode::ConfirmDisconnect;
                            view.draw_menu(auto_fit)?;
                        }
                        UiAction::ConfirmDisconnect => {
                            release_pressed_buttons(endpoint, &mut remote_pressed).await?;
                            view.set_message("disconnecting session...".to_owned());
                            view.draw_menu(auto_fit)?;
                            if let Err(error) = crate::daemon_lifecycle::disconnect_session(endpoint, None).await {
                                view.set_message(format!("disconnect failed: {error}"));
                                view.draw_menu(auto_fit)?;
                            } else {
                                disconnected = true;
                                break;
                            }
                        }
                        UiAction::CancelDisconnect => {
                            view.menu_mode = MenuMode::Options;
                            view.draw_menu(auto_fit)?;
                        }
                    }
                    continue;
                }
                if requests.is_empty() {
                    continue;
                }
                for request in &requests {
                    if let Response::Err(error) = transport::send_request(endpoint, request).await? {
                        view.set_message(error.to_string());
                    }
                }
                let soon = Instant::now() + INPUT_REFRESH_DELAY;
                if refresh.deadline() > soon {
                    refresh.as_mut().reset(soon);
                }
            }
            () = tokio::time::sleep_until(fit_at.unwrap_or_else(Instant::now)), if fit_at.is_some() && view.menu_mode == MenuMode::Closed => {
                fit_at = None;
                let (columns, rows) = terminal::size()?;
                let message = match fit_size(columns, rows.saturating_sub(1), cell_size) {
                    Some((width, height)) => match transport::send_request(endpoint, &Request::Resize { width, height }).await? {
                        Response::Ok(_) => format!("resizing remote desktop to {width}x{height}"),
                        Response::Err(error) => format!("resize failed: {error}"),
                    },
                    None => "terminal is too small to fit".to_owned(),
                };
                view.set_message(message);
                view.draw_status(auto_fit)?;
            }
            () = &mut refresh => {
                if view.menu_mode != MenuMode::Closed {
                    refresh.as_mut().reset(Instant::now() + options.interval);
                    continue;
                }
                let fetched = match frame.take() {
                    Some(frame) => Ok(frame),
                    None => fetch_frame(endpoint).await?,
                };
                match fetched {
                    Ok(frame) => view.draw(&frame)?,
                    Err(message) => view.set_message(message),
                }
                view.draw_status(auto_fit)?;
                refresh.as_mut().reset(Instant::now() + options.interval);
            }
        }
    }

    drop(_guard);
    if let Some(status) = ended {
        if status.state == ConnState::Failed {
            anyhow::bail!(
                "session failed: {}",
                status.message.as_deref().unwrap_or("connection failed")
            );
        }
        writeln!(
            std::io::stdout(),
            "session ended (state: {:?}); daemon remains running",
            status.state
        )?;
    } else if disconnected {
        std::io::stdout().write_all(b"session disconnected; daemon remains running\n")?;
    }
    Ok(())
}

async fn ended_session(endpoint: &Endpoint) -> anyhow::Result<Option<StatusInfo>> {
    let status = crate::daemon_lifecycle::probe(endpoint)
        .await?
        .context("daemon stopped while attached")?;
    Ok(matches!(
        status.state,
        ConnState::NoSession | ConnState::Disconnected | ConnState::Failed
    )
    .then_some(status))
}

struct Frame {
    width: u16,
    height: u16,
    png: Vec<u8>,
}

/// Requests a screenshot; the inner error is a daemon-side failure worth showing in the status line.
async fn fetch_frame(endpoint: &Endpoint) -> anyhow::Result<Result<Frame, String>> {
    match transport::send_request(endpoint, &Request::Screenshot).await? {
        Response::Ok(Payload::Screenshot { width, height, png }) => Ok(Ok(Frame { width, height, png })),
        Response::Ok(_) => anyhow::bail!("unexpected response to screenshot request"),
        Response::Err(error) => Ok(Err(error.to_string())),
    }
}

async fn release_pressed_buttons(endpoint: &Endpoint, pressed: &mut HashSet<event::MouseButton>) -> anyhow::Result<()> {
    for button in pressed.clone() {
        match transport::send_request(
            endpoint,
            &Request::MouseButton {
                button: remote_button(button),
                pressed: false,
            },
        )
        .await?
        {
            Response::Ok(_) => {
                pressed.remove(&button);
            }
            Response::Err(error) => anyhow::bail!("release remote mouse button: {error}"),
        }
    }
    Ok(())
}

struct View {
    protocol: Protocol,
    cell_size: (u32, u32),
    layout: Option<Layout>,
    resolution: Option<(u16, u16)>,
    drawn_png: Vec<u8>,
    /// Transient status message and when it was set.
    message: Option<(String, std::time::Instant)>,
    menu_mode: MenuMode,
}

impl View {
    fn set_message(&mut self, message: String) {
        self.message = Some((message, std::time::Instant::now()));
    }

    fn draw(&mut self, frame: &Frame) -> anyhow::Result<()> {
        self.resolution = Some((frame.width, frame.height));
        let (columns, rows) = terminal::size()?;
        // The last row holds the status line.
        let layout = Layout::new(
            frame.width,
            frame.height,
            columns,
            rows.saturating_sub(1),
            self.cell_size,
            self.protocol,
        );

        let mut out = std::io::stdout().lock();
        if layout != self.layout {
            self.layout = layout;
            self.drawn_png.clear();
            queue!(out, terminal::Clear(terminal::ClearType::All))?;
        }
        let Some(layout) = layout else {
            drop(out);
            self.set_message("terminal is too small".to_owned());
            return Ok(());
        };
        if frame.png == self.drawn_png {
            return Ok(());
        }

        queue!(out, cursor::MoveTo(0, 0))?;
        terminal_image::write_image(
            &mut out,
            &frame.png,
            self.protocol,
            layout.columns,
            layout.image_width,
            KITTY_KEYS,
        )?;
        out.flush()?;
        self.drawn_png.clone_from(&frame.png);
        Ok(())
    }

    fn footer(&self, auto_fit: bool, columns: u16) -> Footer {
        let message = self
            .message
            .as_ref()
            .and_then(|(message, since)| (since.elapsed() < MESSAGE_DURATION).then_some(message.as_str()));
        Footer::new(
            columns,
            self.menu_mode != MenuMode::Closed,
            auto_fit,
            self.resolution,
            message,
        )
    }

    fn draw_status(&self, auto_fit: bool) -> anyhow::Result<()> {
        let (columns, rows) = terminal::size()?;
        let footer = self.footer(auto_fit, columns);

        let mut out = std::io::stdout().lock();
        queue!(
            out,
            cursor::MoveTo(0, rows.saturating_sub(1)),
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;
        write!(
            out,
            "\x1b[7m{:width$}\x1b[0m",
            footer.text,
            width = usize::from(columns)
        )?;
        out.flush()?;
        Ok(())
    }

    fn draw_menu(&mut self, auto_fit: bool) -> anyhow::Result<()> {
        self.layout = None;
        self.drawn_png.clear();
        let (columns, rows) = terminal::size()?;
        let mut lines = if self.menu_mode == MenuMode::ConfirmDisconnect {
            vec![
                " Disconnect the current RDP session?".to_owned(),
                " The daemon will keep running.".to_owned(),
                " y  Disconnect session and leave attach".to_owned(),
                " n  Cancel; return to the menu".to_owned(),
                " Esc  Cancel".to_owned(),
            ]
        } else {
            vec![
                " IronRDP terminal attachment".to_owned(),
                " Click an option or press its number:".to_owned(),
                " 1  Fit desktop to terminal (Ctrl+\\)".to_owned(),
                format!(
                    " 2  Auto-fit: {} (resize with window)",
                    if auto_fit { "ON" } else { "OFF" }
                ),
                " 3  Refresh frame".to_owned(),
                " 4  Detach (Ctrl+], session keeps running)".to_owned(),
                " 5  Disconnect session (confirmation required)".to_owned(),
                " Esc  Back to desktop".to_owned(),
            ]
        };
        if self.menu_mode == MenuMode::ConfirmDisconnect
            && let Some((message, since)) = &self.message
            && since.elapsed() < MESSAGE_DURATION
        {
            lines.push(format!(" {message}"));
        }
        let mut out = std::io::stdout().lock();
        if self.protocol == Protocol::Kitty {
            out.write_all(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\")?;
        }
        queue!(out, terminal::Clear(terminal::ClearType::All))?;
        for (row, line) in lines.into_iter().take(usize::from(rows.saturating_sub(1))).enumerate() {
            let row = u16::try_from(row).expect("menu fits in terminal rows");
            queue!(out, cursor::MoveTo(0, row))?;
            let visible: String = line
                .chars()
                .map(|ch| if ch.is_control() { ' ' } else { ch })
                .take(usize::from(columns.saturating_sub(1)))
                .collect();
            out.write_all(visible.as_bytes())?;
        }
        out.flush()?;
        drop(out);
        self.draw_status(auto_fit)
    }

    fn close_menu(&mut self, auto_fit: bool) -> anyhow::Result<()> {
        self.menu_mode = MenuMode::Closed;
        self.layout = None;
        self.drawn_png.clear();
        let mut out = std::io::stdout().lock();
        queue!(out, terminal::Clear(terminal::ClearType::All))?;
        out.flush()?;
        drop(out);
        self.draw_status(auto_fit)
    }
}

/// Restores the terminal on every exit path, including errors and panics.
struct TerminalGuard {
    protocol: Protocol,
}

impl TerminalGuard {
    fn enter(protocol: Protocol) -> anyhow::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self { protocol };
        execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            EnableMouseCapture,
            EnableBracketedPaste,
            terminal::Clear(terminal::ClearType::All)
        )?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        if self.protocol == Protocol::Kitty {
            let _ = out.write_all(b"\x1b_Ga=d,d=I,i=1,q=2\x1b\\");
        }
        let _ = execute!(
            out,
            DisableBracketedPaste,
            DisableMouseCapture,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Placement of the remote frame in the terminal, anchored at the top-left cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    remote_width: u16,
    remote_height: u16,
    cell_width: u32,
    cell_height: u32,
    /// Cells spanned horizontally by the image.
    columns: u16,
    /// Displayed image size in terminal pixels.
    image_width: u32,
    image_height: u32,
}

impl Layout {
    /// Fits the frame into `columns × rows` cells, preserving the aspect ratio.
    ///
    /// Sixel output is never upscaled. Kitty and iTerm2 scale images to whole cells, so their
    /// width is rounded down to a cell boundary.
    fn new(
        remote_width: u16,
        remote_height: u16,
        columns: u16,
        rows: u16,
        (cell_width, cell_height): (u32, u32),
        protocol: Protocol,
    ) -> Option<Self> {
        if remote_width == 0 || remote_height == 0 || cell_width == 0 || cell_height == 0 {
            return None;
        }
        let (remote_w, remote_h) = (u64::from(remote_width), u64::from(remote_height));
        let cell_w = u64::from(cell_width);
        let box_width = u64::from(columns) * cell_w;
        // Sixel bands are 6 pixels tall, so the last band may overshoot by up to 5 pixels.
        let box_height = (u64::from(rows) * u64::from(cell_height)).saturating_sub(5);

        let mut width = box_width.min(box_height * remote_w / remote_h);
        let columns = if protocol == Protocol::Sixel {
            width = width.min(remote_w);
            width.div_ceil(cell_w)
        } else {
            width /= cell_w;
            width *= cell_w;
            width / cell_w
        };
        let height = remote_h * width / remote_w;
        if width == 0 || height == 0 {
            return None;
        }

        Some(Self {
            remote_width,
            remote_height,
            cell_width,
            cell_height,
            columns: u16::try_from(columns).ok()?,
            image_width: u32::try_from(width).ok()?,
            image_height: u32::try_from(height).ok()?,
        })
    }

    /// Maps the centre of a terminal cell to remote desktop coordinates, or `None` outside the image.
    fn remote_position(&self, column: u16, row: u16) -> Option<(u16, u16)> {
        let x = u64::from(column) * u64::from(self.cell_width) + u64::from(self.cell_width / 2);
        let y = u64::from(row) * u64::from(self.cell_height) + u64::from(self.cell_height / 2);
        let (image_width, image_height) = (u64::from(self.image_width), u64::from(self.image_height));
        if x >= image_width || y >= image_height {
            return None;
        }
        let remote_x = x * u64::from(self.remote_width) / image_width;
        let remote_y = y * u64::from(self.remote_height) / image_height;
        Some((u16::try_from(remote_x).ok()?, u16::try_from(remote_y).ok()?))
    }
}

fn is_hotkey(key: &KeyEvent, hotkey: [char; 3]) -> bool {
    key.kind != KeyEventKind::Release
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char(ch) if hotkey.contains(&ch))
}

/// Remote desktop size that fills `columns × rows` terminal cells.
///
/// Sixel bands are 6 pixels tall, so the height leaves room for the last band to overshoot. The
/// server may still adjust the size (even width, 200..=8192 pixels).
fn fit_size(columns: u16, rows: u16, (cell_width, cell_height): (u32, u32)) -> Option<(u16, u16)> {
    let width = u32::from(columns).checked_mul(cell_width)?;
    let height = u32::from(rows).checked_mul(cell_height)?.checked_sub(5)?;
    let width = u16::try_from(width.min(8192)).ok()?;
    let height = u16::try_from(height.min(8192)).ok()?;
    (width >= 200 && height >= 200).then_some((width & !1, height))
}

fn mouse_requests(layout: &Layout, event: MouseEvent) -> Vec<Request> {
    let position = layout.remote_position(event.column, event.row);
    let mut requests: Vec<Request> = position.map(|(x, y)| Request::MouseMove { x, y }).into_iter().collect();
    let wheel = |delta, horizontal| Request::Wheel { delta, horizontal };

    match event.kind {
        MouseEventKind::Moved | MouseEventKind::Drag(_) => {}
        // Presses and wheel turns outside the image are dropped; releases always go through so
        // a button never stays stuck down.
        _ if position.is_none() && !matches!(event.kind, MouseEventKind::Up(_)) => {}
        MouseEventKind::Down(pressed) => requests.push(Request::MouseButton {
            button: remote_button(pressed),
            pressed: true,
        }),
        MouseEventKind::Up(released) => requests.push(Request::MouseButton {
            button: remote_button(released),
            pressed: false,
        }),
        MouseEventKind::ScrollUp => requests.push(wheel(WHEEL_NOTCH, false)),
        MouseEventKind::ScrollDown => requests.push(wheel(-WHEEL_NOTCH, false)),
        MouseEventKind::ScrollLeft => requests.push(wheel(-WHEEL_NOTCH, true)),
        MouseEventKind::ScrollRight => requests.push(wheel(WHEEL_NOTCH, true)),
    }
    requests
}

fn remote_button(button: event::MouseButton) -> MouseButton {
    match button {
        event::MouseButton::Left => MouseButton::Left,
        event::MouseButton::Right => MouseButton::Right,
        event::MouseButton::Middle => MouseButton::Middle,
    }
}

fn route_mouse(
    mouse: MouseEvent,
    rows: u16,
    footer: &Footer,
    menu_mode: MenuMode,
    layout: Option<Layout>,
    local_click: &mut Option<event::MouseButton>,
    remote_pressed: &mut HashSet<event::MouseButton>,
) -> (Option<UiAction>, Vec<Request>) {
    let footer_row = rows.saturating_sub(1);
    let menu_open = menu_mode != MenuMode::Closed;
    match mouse.kind {
        MouseEventKind::Up(button) if *local_click == Some(button) => {
            *local_click = None;
            let requests = if remote_pressed.remove(&button) {
                vec![Request::MouseButton {
                    button: remote_button(button),
                    pressed: false,
                }]
            } else {
                Vec::new()
            };
            (None, requests)
        }
        MouseEventKind::Up(button) if (menu_open || mouse.row == footer_row) && remote_pressed.remove(&button) => (
            None,
            vec![Request::MouseButton {
                button: remote_button(button),
                pressed: false,
            }],
        ),
        MouseEventKind::Down(button) if menu_open || mouse.row == footer_row => {
            *local_click = Some(button);
            let action = (button == event::MouseButton::Left)
                .then(|| {
                    if mouse.row == footer_row {
                        footer.action_at(mouse.column)
                    } else {
                        menu_action_at(mouse.row, menu_mode == MenuMode::ConfirmDisconnect)
                    }
                })
                .flatten();
            (action, Vec::new())
        }
        _ if menu_open || local_click.is_some() || mouse.row == footer_row => (None, Vec::new()),
        _ => {
            let requests = layout.map(|layout| mouse_requests(&layout, mouse)).unwrap_or_default();
            if let MouseEventKind::Down(button) = mouse.kind
                && !requests.is_empty()
            {
                remote_pressed.insert(button);
            }
            if let MouseEventKind::Up(button) = mouse.kind {
                remote_pressed.remove(&button);
            }
            (None, requests)
        }
    }
}

/// Translates a terminal key press into input requests.
///
/// Plain and shifted characters are sent as Unicode so any keyboard layout works. Keys without a
/// character, and characters combined with Ctrl, Alt, or Win, are sent as US set-1 scancodes with
/// the modifiers pressed around them, because shortcuts are layout-independent key positions.
fn key_requests(key: KeyEvent) -> Vec<KeyInput> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }

    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let win = modifiers.contains(KeyModifiers::SUPER);
    let mut shift = modifiers.contains(KeyModifiers::SHIFT);

    let scancode = match key.code {
        KeyCode::Char(ch) => {
            // Windows reports AltGr as Ctrl+Alt; the character it produced is what the user wants.
            let altgr_symbol = ctrl && alt && !ch.is_ascii_alphanumeric();
            match char_scancode(ch) {
                Some(scancode) if (ctrl || alt || win) && !altgr_symbol => scancode,
                _ => {
                    return vec![
                        KeyInput::Unicode { ch, pressed: true },
                        KeyInput::Unicode { ch, pressed: false },
                    ];
                }
            }
        }
        KeyCode::Null => 0x39,
        KeyCode::BackTab => {
            shift = true;
            0x0F
        }
        code => match special_scancode(code) {
            Some(scancode) => scancode,
            None => return Vec::new(),
        },
    };

    let held: Vec<u16> = [
        (ctrl, SCANCODE_CTRL),
        (alt, SCANCODE_ALT),
        (shift, SCANCODE_SHIFT),
        (win, SCANCODE_WIN),
    ]
    .into_iter()
    .filter_map(|(active, scancode)| active.then_some(scancode))
    .collect();

    let press = |scancode| KeyInput::Scancode {
        scancode,
        pressed: true,
    };
    let release = |scancode| KeyInput::Scancode {
        scancode,
        pressed: false,
    };
    held.iter()
        .copied()
        .map(press)
        .chain([press(scancode), release(scancode)])
        .chain(held.iter().rev().copied().map(release))
        .collect()
}

fn special_scancode(code: KeyCode) -> Option<u16> {
    Some(match code {
        KeyCode::Esc => 0x01,
        KeyCode::Backspace => 0x0E,
        KeyCode::Tab => 0x0F,
        KeyCode::Enter => 0x1C,
        KeyCode::Up => 0xE048,
        KeyCode::PageUp => 0xE049,
        KeyCode::Left => 0xE04B,
        KeyCode::Right => 0xE04D,
        KeyCode::Home => 0xE047,
        KeyCode::End => 0xE04F,
        KeyCode::Down => 0xE050,
        KeyCode::PageDown => 0xE051,
        KeyCode::Insert => 0xE052,
        KeyCode::Delete => 0xE053,
        KeyCode::F(n @ 1..=10) => 0x3A + u16::from(n),
        KeyCode::F(11) => 0x57,
        KeyCode::F(12) => 0x58,
        _ => return None,
    })
}

/// US layout set-1 scancode of the key that types `ch` (ignoring case).
fn char_scancode(ch: char) -> Option<u16> {
    const LETTERS: [u16; 26] = [
        0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13,
        0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
    ];

    let ch = ch.to_ascii_lowercase();
    Some(match ch {
        'a'..='z' => LETTERS[usize::from(u8::try_from(ch).ok()? - b'a')],
        '1'..='9' => 0x02 + u16::from(u8::try_from(ch).ok()? - b'1'),
        '0' => 0x0B,
        ' ' => 0x39,
        '-' => 0x0C,
        '=' => 0x0D,
        '[' => 0x1A,
        ']' => 0x1B,
        ';' => 0x27,
        '\'' => 0x28,
        '`' => 0x29,
        '\\' => 0x2B,
        ',' => 0x33,
        '.' => 0x34,
        '/' => 0x35,
        _ => return None,
    })
}

fn paste_requests(text: &str) -> Vec<Request> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(MAX_UNICODE_TEXT_CHARS)
        .map(|chunk| Request::UnicodeText {
            text: chunk.iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn session_poll_distinguishes_terminal_states_from_reconnection() {
        let endpoint = transport::default_endpoint_named(&format!(
            "ir-attach-states-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut listener = transport::Listener::bind(&endpoint).unwrap();
        let states = [
            ConnState::Connecting,
            ConnState::Connected,
            ConnState::Disconnecting,
            ConnState::NoSession,
            ConnState::Disconnected,
            ConnState::Failed,
        ];
        let server = async {
            for state in states {
                let mut stream = listener.accept().await.unwrap();
                let request: Request = transport::read_message(&mut stream).await.unwrap();
                assert!(matches!(request, Request::Status));
                transport::write_message(
                    &mut stream,
                    &Response::Ok(Payload::Status(StatusInfo {
                        state,
                        destination: Some("server.example:3389".to_owned()),
                        width: Some(800),
                        height: Some(600),
                        message: Some("session outcome".to_owned()),
                        credentials_loaded: false,
                        untrusted_certificate: None,
                    })),
                )
                .await
                .unwrap();
            }
        };
        let client = async {
            for (index, state) in states.into_iter().enumerate() {
                let ended = ended_session(&endpoint).await.unwrap();
                if index < 3 {
                    assert!(ended.is_none(), "{state:?} must not detach during reconnection");
                } else {
                    let ended = ended.unwrap();
                    assert_eq!(ended.state, state);
                    assert_eq!(ended.message.as_deref(), Some("session outcome"));
                }
            }
        };
        tokio::join!(server, client);
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn scancodes(requests: &[KeyInput]) -> Vec<(u16, bool)> {
        requests
            .iter()
            .map(|request| match request {
                KeyInput::Scancode { scancode, pressed } => (*scancode, *pressed),
                _ => panic!("expected only scancode requests"),
            })
            .collect()
    }

    #[test]
    fn layout_fits_height_constrained_box() {
        // 120×30 cells of 10×20 px leave 1200×575 px; a 1920×1080 frame is height-bound.
        let layout = Layout::new(1920, 1080, 120, 29, (10, 20), Protocol::Sixel).expect("layout");
        assert_eq!(layout.image_width, 1022);
        assert_eq!(layout.image_height, 574);
        assert_eq!(layout.columns, 103);
    }

    #[test]
    fn layout_never_upscales_sixel_but_rounds_kitty_to_cells() {
        let sixel = Layout::new(640, 480, 200, 60, (10, 20), Protocol::Sixel).expect("layout");
        assert_eq!((sixel.image_width, sixel.image_height, sixel.columns), (640, 480, 64));

        let kitty = Layout::new(1000, 500, 80, 40, (9, 18), Protocol::Kitty).expect("layout");
        assert_eq!(kitty.image_width % 9, 0);
        assert_eq!(u32::from(kitty.columns) * 9, kitty.image_width);
    }

    #[test]
    fn layout_rejects_degenerate_sizes() {
        assert_eq!(Layout::new(1920, 1080, 0, 29, (10, 20), Protocol::Sixel), None);
        assert_eq!(Layout::new(0, 1080, 120, 29, (10, 20), Protocol::Sixel), None);
    }

    #[test]
    fn remote_position_maps_cell_centres_and_rejects_outside() {
        let layout = Layout::new(1000, 400, 100, 20, (10, 20), Protocol::Sixel).expect("layout");
        assert_eq!((layout.image_width, layout.image_height), (987, 394));
        assert_eq!(layout.remote_position(0, 0), Some((5, 10)));
        assert_eq!(layout.remote_position(98, 19), Some((997, 395)));
        assert_eq!(layout.remote_position(99, 0), None);
        assert_eq!(layout.remote_position(0, 20), None);
    }

    #[test]
    fn plain_characters_are_sent_as_unicode() {
        let requests = key_requests(key(KeyCode::Char('\u{e9}'), KeyModifiers::NONE));
        assert_eq!(
            requests,
            [
                KeyInput::Unicode {
                    ch: '\u{e9}',
                    pressed: true
                },
                KeyInput::Unicode {
                    ch: '\u{e9}',
                    pressed: false
                },
            ]
        );
        let shifted = key_requests(key(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(shifted[0], KeyInput::Unicode { ch: 'A', pressed: true });
    }

    #[test]
    fn shortcuts_wrap_scancodes_in_modifiers() {
        let requests = key_requests(key(KeyCode::Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT));
        assert_eq!(
            scancodes(&requests),
            [
                (0x1D, true),
                (0x2A, true),
                (0x2E, true),
                (0x2E, false),
                (0x2A, false),
                (0x1D, false)
            ]
        );
        assert_eq!(
            key_requests(key(
                KeyCode::Char('x'),
                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::SUPER
            ))
            .len(),
            ironrdp_rpc::ipc::MAX_KEY_BATCH_EVENTS
        );
    }

    #[test]
    fn altgr_symbols_stay_unicode() {
        let requests = key_requests(key(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT));
        assert_eq!(requests[0], KeyInput::Unicode { ch: '@', pressed: true });
    }

    #[test]
    fn special_keys_use_extended_scancodes() {
        assert_eq!(
            scancodes(&key_requests(key(KeyCode::Delete, KeyModifiers::NONE))),
            [(0xE053, true), (0xE053, false)]
        );
        assert_eq!(
            scancodes(&key_requests(key(KeyCode::BackTab, KeyModifiers::SHIFT))),
            [(0x2A, true), (0x0F, true), (0x0F, false), (0x2A, false)]
        );
        assert_eq!(special_scancode(KeyCode::F(1)), Some(0x3B));
        assert_eq!(special_scancode(KeyCode::F(10)), Some(0x44));
    }

    #[test]
    fn releases_are_ignored_and_ctrl_bracket_detaches() {
        let mut release = key(KeyCode::Char('a'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert!(key_requests(release).is_empty());
        assert!(is_hotkey(
            &key(KeyCode::Char(']'), KeyModifiers::CONTROL),
            HOTKEY_DETACH
        ));
        assert!(!is_hotkey(&key(KeyCode::Char(']'), KeyModifiers::NONE), HOTKEY_DETACH));
        assert!(is_hotkey(
            &key(KeyCode::Char('\x1c'), KeyModifiers::CONTROL),
            HOTKEY_FIT
        ));
        assert!(!is_hotkey(
            &key(KeyCode::Char('\\'), KeyModifiers::CONTROL | KeyModifiers::ALT),
            HOTKEY_FIT
        ));
    }

    #[test]
    fn fit_size_fills_the_terminal_and_yields_a_matching_layout() {
        let (width, height) = fit_size(155, 39, (10, 20)).expect("size");
        assert_eq!((width, height), (1550, 775));
        let layout = Layout::new(width, height, 155, 39, (10, 20), Protocol::Sixel).expect("layout");
        assert_eq!((layout.image_width, layout.image_height), (1550, 775));

        assert_eq!(fit_size(81, 30, (9, 18)), Some((728, 535)));
        assert_eq!(fit_size(10, 5, (10, 20)), None);
    }

    #[test]
    fn mouse_clicks_move_then_press_and_releases_always_pass() {
        let layout = Layout::new(1000, 400, 100, 20, (10, 20), Protocol::Sixel).expect("layout");
        let event = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };

        assert_eq!(
            mouse_requests(&layout, event(MouseEventKind::Down(event::MouseButton::Left), 0, 0)),
            [
                Request::MouseMove { x: 5, y: 10 },
                Request::MouseButton {
                    button: MouseButton::Left,
                    pressed: true
                },
            ]
        );
        assert!(mouse_requests(&layout, event(MouseEventKind::Down(event::MouseButton::Left), 99, 0)).is_empty());
        assert_eq!(
            mouse_requests(&layout, event(MouseEventKind::Up(event::MouseButton::Right), 99, 0)),
            [Request::MouseButton {
                button: MouseButton::Right,
                pressed: false
            }]
        );
        assert_eq!(
            mouse_requests(&layout, event(MouseEventKind::ScrollDown, 1, 1))[1],
            Request::Wheel {
                delta: -WHEEL_NOTCH,
                horizontal: false
            }
        );
    }

    #[test]
    fn footer_actions_remain_clickable_when_space_is_limited() {
        let full = Footer::new(80, false, true, Some((1280, 720)), None);
        assert_eq!(full.action_at(0), Some(UiAction::Menu));
        assert_eq!(full.action_at(5), Some(UiAction::Menu));
        assert_eq!(full.action_at(6), None);
        assert_eq!(full.action_at(7), Some(UiAction::Fit));
        assert_eq!(full.action_at(13), Some(UiAction::Detach));
        assert!(full.text.contains("Auto-fit: ON  1280x720"));

        let narrow = Footer::new(15, true, false, None, None);
        assert_eq!(narrow.action_at(0), Some(UiAction::Menu));
        assert_eq!(narrow.action_at(7), Some(UiAction::Detach));
        assert_eq!(narrow.action_at(15), None);
        assert_eq!(
            Footer::new(8, false, false, None, None).action_at(0),
            Some(UiAction::Detach)
        );
        assert_eq!(Footer::new(7, false, false, None, None).action_at(0), None);
    }

    #[test]
    fn footer_sanitizes_status_messages() {
        let footer = Footer::new(60, false, false, None, Some("resize failed:\n\x1b[31m"));
        assert!(!footer.text.contains('\n'));
        assert!(!footer.text.contains('\x1b'));
        assert_eq!(footer.action_at(13), Some(UiAction::Detach));
    }

    #[test]
    fn menu_actions_use_only_modal_keys_and_rows() {
        for (number, action) in [
            ('1', UiAction::Fit),
            ('2', UiAction::ToggleAutoFit),
            ('3', UiAction::Refresh),
            ('4', UiAction::Detach),
            ('5', UiAction::Disconnect),
        ] {
            assert_eq!(
                menu_key_action(key(KeyCode::Char(number), KeyModifiers::NONE), false),
                Some(action)
            );
        }
        assert_eq!(
            menu_key_action(key(KeyCode::Esc, KeyModifiers::NONE), false),
            Some(UiAction::Menu)
        );
        assert_eq!(
            menu_key_action(key(KeyCode::Char('x'), KeyModifiers::NONE), false),
            None
        );
        assert_eq!(
            menu_key_action(key(KeyCode::Char('1'), KeyModifiers::CONTROL), false),
            None
        );
        let mut repeat = key(KeyCode::Char('2'), KeyModifiers::NONE);
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(menu_key_action(repeat, false), None);
        assert_eq!(menu_action_at(2, false), Some(UiAction::Fit));
        assert_eq!(menu_action_at(5, false), Some(UiAction::Detach));
        assert_eq!(menu_action_at(6, false), Some(UiAction::Disconnect));
        assert_eq!(menu_action_at(7, false), Some(UiAction::Menu));
        assert_eq!(menu_action_at(1, false), None);
    }

    #[test]
    fn disconnect_confirmation_requires_an_explicit_yes() {
        assert_eq!(
            menu_key_action(key(KeyCode::Char('y'), KeyModifiers::NONE), true),
            Some(UiAction::ConfirmDisconnect)
        );
        assert_eq!(
            menu_key_action(key(KeyCode::Char('n'), KeyModifiers::NONE), true),
            Some(UiAction::CancelDisconnect)
        );
        assert_eq!(
            menu_key_action(key(KeyCode::Esc, KeyModifiers::NONE), true),
            Some(UiAction::CancelDisconnect)
        );
        assert_eq!(menu_key_action(key(KeyCode::Enter, KeyModifiers::NONE), true), None);
        assert_eq!(menu_key_action(key(KeyCode::Char('5'), KeyModifiers::NONE), true), None);
        assert_eq!(menu_action_at(2, true), Some(UiAction::ConfirmDisconnect));
        assert_eq!(menu_action_at(3, true), Some(UiAction::CancelDisconnect));
        assert_eq!(menu_action_at(6, true), None);
    }

    #[test]
    fn footer_and_menu_clicks_never_send_remote_input() {
        let layout = Layout::new(1000, 400, 100, 19, (10, 20), Protocol::Sixel);
        let footer = Footer::new(100, false, false, Some((1000, 400)), None);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let mut local_click = None;
        let mut remote_pressed = HashSet::new();

        let (action, requests) = route_mouse(
            mouse(MouseEventKind::Down(event::MouseButton::Left), 13, 19),
            20,
            &footer,
            MenuMode::Closed,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(action, Some(UiAction::Detach));
        assert!(requests.is_empty());
        assert!(
            route_mouse(
                mouse(MouseEventKind::Up(event::MouseButton::Left), 13, 19),
                20,
                &footer,
                MenuMode::Closed,
                layout,
                &mut local_click,
                &mut remote_pressed,
            )
            .1
            .is_empty()
        );

        let (action, requests) = route_mouse(
            mouse(MouseEventKind::Down(event::MouseButton::Left), 2, 3),
            20,
            &footer,
            MenuMode::Options,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(action, Some(UiAction::ToggleAutoFit));
        assert!(requests.is_empty());

        let _ = route_mouse(
            mouse(MouseEventKind::Up(event::MouseButton::Left), 2, 3),
            20,
            &footer,
            MenuMode::Options,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        let (action, requests) = route_mouse(
            mouse(MouseEventKind::Down(event::MouseButton::Left), 2, 6),
            20,
            &footer,
            MenuMode::Options,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(action, Some(UiAction::Disconnect));
        assert!(requests.is_empty());
        let _ = route_mouse(
            mouse(MouseEventKind::Up(event::MouseButton::Left), 2, 6),
            20,
            &footer,
            MenuMode::ConfirmDisconnect,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        let (action, requests) = route_mouse(
            mouse(MouseEventKind::Down(event::MouseButton::Left), 2, 2),
            20,
            &footer,
            MenuMode::ConfirmDisconnect,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(action, Some(UiAction::ConfirmDisconnect));
        assert!(requests.is_empty());
    }

    #[test]
    fn remote_drag_released_on_footer_does_not_stick() {
        let layout = Layout::new(1000, 400, 100, 19, (10, 20), Protocol::Sixel);
        let footer = Footer::new(100, false, false, None, None);
        let mouse = |kind, row| MouseEvent {
            kind,
            column: 2,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let mut local_click = None;
        let mut remote_pressed = HashSet::new();
        let (_, down) = route_mouse(
            mouse(MouseEventKind::Down(event::MouseButton::Left), 1),
            20,
            &footer,
            MenuMode::Closed,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(
            down[1],
            Request::MouseButton {
                button: MouseButton::Left,
                pressed: true
            }
        );
        let (_, up) = route_mouse(
            mouse(MouseEventKind::Up(event::MouseButton::Left), 19),
            20,
            &footer,
            MenuMode::Closed,
            layout,
            &mut local_click,
            &mut remote_pressed,
        );
        assert_eq!(
            up,
            [Request::MouseButton {
                button: MouseButton::Left,
                pressed: false
            }]
        );
        assert!(remote_pressed.is_empty());
    }

    #[test]
    fn paste_is_split_into_bounded_chunks() {
        let text = "x".repeat(MAX_UNICODE_TEXT_CHARS + 1);
        let requests = paste_requests(&text);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1], Request::UnicodeText { text: "x".to_owned() });
    }
}

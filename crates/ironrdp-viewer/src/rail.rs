//! RemoteApp (RAIL) windows: one local window per server window.
//!
//! The server describes its application windows with Windowing Alternate Secondary
//! Drawing Orders ([MS-RDPERP] 2.2.1.3). Each window is shown as an undecorated local
//! window at the server's screen position, because the server draws the frame. Its
//! pixels come from the window's rectangle in the session frame, or from the graphics
//! pipeline surface the server mapped to the window when there is one.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use std::collections::BTreeMap;
use std::sync::Arc;

use ironrdp::input::MousePosition;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use ironrdp::pdu::window::{
    DesktopOrder, IconInfo, IconSource, MonitoredDesktop, ShowState, WindowIcon, WindowOrder, WindowOrderBody,
    WindowState, WindowingOrder,
};
use raw_window_handle::DisplayHandle;
use tracing::{debug, warn};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Cursor, Icon, Window, WindowAttributes, WindowId};

/// Upper bound on local windows created for server windows.
const MAX_RAIL_WINDOWS: usize = 256;

/// Upper bound on the decoded pixel bytes of icons kept for Cached Icon orders.
///
/// The client echoes the server's icon cache dimensions, which allow up to 255 caches of
/// 65535 entries, so the cache is bounded by memory rather than by entry count: 64 MiB holds
/// every icon of the 3 caches of 12 entries Windows servers use even at 256x256 pixels.
const MAX_CACHED_ICON_BYTES: usize = 64 * 1024 * 1024;

/// Upper bound on the pixels one window keeps from graphics pipeline surfaces: an 8K display.
pub const MAX_WINDOW_PIXELS: usize = 7680 * 4320;

/// Upper bound on the pixels all windows keep from graphics pipeline surfaces.
pub const MAX_TOTAL_WINDOW_PIXELS: usize = 4 * MAX_WINDOW_PIXELS;

const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// The local view of one server window, merged from incremental window orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailWindowModel {
    pub owner_window_id: Option<u32>,
    pub extended_style: u32,
    pub show_state: ShowState,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub skip_taskbar: bool,
}

impl Default for RailWindowModel {
    fn default() -> Self {
        Self {
            owner_window_id: None,
            extended_style: 0,
            show_state: ShowState::Show,
            title: String::new(),
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            skip_taskbar: false,
        }
    }
}

/// What a window order changed, so the local window is updated only where needed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelChanges {
    pub geometry: bool,
    pub title: bool,
    pub show_state: bool,
    /// Whether [`RailWindowModel::skips_taskbar`] changed.
    pub taskbar: bool,
}

impl RailWindowModel {
    /// Merges the fields present in a New or Existing Window order; absent fields keep their value.
    pub fn apply(&mut self, state: &WindowState) -> ModelChanges {
        let skipped_taskbar = self.skips_taskbar();
        let mut changes = ModelChanges::default();
        if let Some(owner) = state.owner_window_id {
            self.owner_window_id = (owner != 0).then_some(owner);
        }
        if let Some(style) = state.style {
            self.extended_style = style.extended_style;
        }
        if let Some(show_state) = state.show_state {
            changes.show_state = show_state != self.show_state;
            self.show_state = show_state;
        }
        if let Some(title) = &state.title {
            changes.title = *title != self.title;
            self.title.clone_from(title);
        }
        if let Some(offset) = state.window_offset {
            changes.geometry |= (offset.x, offset.y) != (self.x, self.y);
            self.x = offset.x;
            self.y = offset.y;
        }
        if let Some(size) = state.window_size {
            let size = (size.width.max(1), size.height.max(1));
            changes.geometry |= size != (self.width, self.height);
            (self.width, self.height) = size;
        }
        if let Some(taskbar_button) = state.taskbar_button {
            self.skip_taskbar = taskbar_button != 0;
        }
        changes.taskbar = skipped_taskbar != self.skips_taskbar();
        changes
    }

    pub fn is_visible(&self) -> bool {
        self.show_state != ShowState::Hide
    }

    /// Whether the window should stay off the local taskbar: tool windows, owned windows
    /// such as dialogs and menus, and windows the server hides from its taskbar.
    pub fn skips_taskbar(&self) -> bool {
        self.skip_taskbar || self.owner_window_id.is_some() || self.extended_style & WS_EX_TOOLWINDOW != 0
    }

    /// Whether the window takes focus when it opens: tool windows such as menus and
    /// tooltips, and windows the server marks as not activating, leave focus where it is,
    /// as they do on the server.
    pub fn activates_on_open(&self) -> bool {
        self.extended_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) == 0
    }
}

/// Translates a window-relative position by the window's screen origin, clamped to the
/// range of an RDP pointer coordinate.
pub fn window_point_to_desktop(origin: (i32, i32), local_x: f64, local_y: f64) -> Option<MousePosition> {
    if !local_x.is_finite() || !local_y.is_finite() {
        return None;
    }
    let to_coordinate = |origin: i32, local: f64| {
        #[expect(clippy::as_conversions, reason = "the value is finite and clamped to the u16 range")]
        let value = (f64::from(origin) + local.floor()).clamp(0.0, f64::from(u16::MAX)) as u16;
        value
    };
    Some(MousePosition {
        x: to_coordinate(origin.0, local_x),
        y: to_coordinate(origin.1, local_y),
    })
}

/// Copies a `width` by `height` rectangle at `(x, y)` out of a session frame; pixels outside
/// the frame are black.
pub fn crop_frame(
    frame: &[u32],
    (frame_width, frame_height): (u16, u16),
    (x, y): (i32, i32),
    (width, height): (u32, u32),
) -> Vec<u32> {
    // INVARIANT: every input fits in `i64`, and the clamps below keep each index inside `out` or `frame`.
    let (x, y) = (i64::from(x), i64::from(y));
    let (width, height) = (i64::from(width), i64::from(height));
    let (frame_width, frame_height) = (i64::from(frame_width), i64::from(frame_height));
    let index = |value: i64| usize::try_from(value).unwrap_or(0);

    let mut out = vec![0; index(width * height)];
    // The columns of the crop that fall inside the frame.
    let first = (-x).clamp(0, width);
    let last = (frame_width - x).clamp(0, width);
    if last <= first {
        return out;
    }
    for row in 0..height {
        let source_y = y + row;
        if !(0..frame_height).contains(&source_y) {
            continue;
        }
        let source_start = index(source_y * frame_width + x + first);
        if let Some(source) = frame.get(source_start..source_start + index(last - first)) {
            out[index(row * width + first)..index(row * width + last)].copy_from_slice(source);
        }
    }
    out
}

/// Window content painted by graphics pipeline surfaces mapped to the window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowPixels {
    pub width: u16,
    pub height: u16,
    /// `0x00RRGGBB` pixels, row-major.
    pub pixels: Vec<u32>,
}

impl WindowPixels {
    /// Writes an RGBA8888 region in window coordinates, resizing the content first when the
    /// presented size changed.
    pub fn apply(&mut self, (width, height): (u16, u16), region: &ExclusiveRectangle, rgba: &[u8]) {
        if (width, height) != (self.width, self.height) {
            *self = Self {
                width,
                height,
                pixels: vec![0; usize::from(width) * usize::from(height)],
            };
        }
        if !(region.left < region.right && region.top < region.bottom) {
            return;
        }
        let region_width = usize::from(region.right - region.left);
        for (row, source) in rgba.chunks_exact(region_width * 4).enumerate() {
            let y = usize::from(region.top) + row;
            if usize::from(region.bottom) <= y || usize::from(self.height) <= y {
                break;
            }
            for (column, pixel) in source.chunks_exact(4).enumerate() {
                let x = usize::from(region.left) + column;
                if usize::from(self.width) <= x {
                    break;
                }
                self.pixels[y * usize::from(self.width) + x] =
                    (u32::from(pixel[0]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[2]);
            }
        }
    }

    /// Copies the top-left `width` by `height` pixels; pixels outside the content are black.
    pub fn crop(&self, width: u32, height: u32) -> Vec<u32> {
        crop_frame(&self.pixels, (self.width, self.height), (0, 0), (width, height))
    }
}

/// Whether a window may keep `(width, height)` pixels of graphics pipeline content while the
/// other windows keep `other_windows_pixels`.
///
/// The size comes from the server's surface-to-window mapping, so it is bounded before the
/// window's pixel buffer is allocated.
/// Largest edge of a local RemoteApp window, in pixels.
const MAX_WINDOW_EDGE: u32 = 16384;

/// Bounds a server window size for a local window, whose surface and painting buffers are
/// allocated at that size: each edge stays within [`MAX_WINDOW_EDGE`] and the area within
/// [`MAX_WINDOW_PIXELS`], shrinking the height first.
pub fn bounded_window_size(width: u32, height: u32) -> (u32, u32) {
    let width = width.clamp(1, MAX_WINDOW_EDGE);
    let max_height =
        u32::try_from(MAX_WINDOW_PIXELS / usize::try_from(width).unwrap_or(usize::MAX)).unwrap_or(u32::MAX);
    (width, height.clamp(1, max_height.clamp(1, MAX_WINDOW_EDGE)))
}

/// Whether a local window's painting buffer fits [`MAX_WINDOW_PIXELS`].
fn fits_window_area(width: u32, height: u32) -> bool {
    usize::try_from(u64::from(width) * u64::from(height)).is_ok_and(|area| area <= MAX_WINDOW_PIXELS)
}

pub fn fits_window_pixel_budget((width, height): (u16, u16), other_windows_pixels: usize) -> bool {
    let pixels = usize::from(width) * usize::from(height);
    pixels <= MAX_WINDOW_PIXELS && other_windows_pixels.saturating_add(pixels) <= MAX_TOTAL_WINDOW_PIXELS
}

/// Converts a TS_ICON_INFO ([MS-RDPERP] 2.2.1.2.3) to RGBA8888 rows, top-down.
///
/// Color bitmaps are bottom-up DIB rows of 1, 4 or 8 bits per pixel (palette indexes),
/// 16 (RGB555), 24 or 32 (BGRA) bits per pixel; the stride is derived from each buffer's size
/// so packed and DWORD-aligned rows both work.
/// When the color bitmap carries no alpha, the AND mask decides transparency.
///
/// [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp
pub fn icon_to_rgba(icon: &IconInfo) -> Option<(Vec<u8>, u32, u32)> {
    let width = usize::from(icon.width);
    let height = usize::from(icon.height);
    if width == 0 || height == 0 {
        return None;
    }
    let bits_per_pixel = match icon.bpp {
        1 | 4 | 8 | 16 | 24 | 32 => usize::from(icon.bpp),
        _ => return None,
    };
    let color_stride = icon.bits_color.len() / height;
    if color_stride < (width * bits_per_pixel).div_ceil(8) {
        return None;
    }
    let palette = |index: u8| -> [u8; 3] {
        let offset = usize::from(index) * 4;
        icon.color_table
            .get(offset..offset + 3)
            .map_or([0, 0, 0], |bgr| [bgr[2], bgr[1], bgr[0]])
    };

    let mut rgba = vec![0; width * height * 4];
    for y in 0..height {
        let source = &icon.bits_color[(height - 1 - y) * color_stride..];
        for x in 0..width {
            let [r, g, b, a] = if bits_per_pixel < 8 {
                // Packed palette indexes, most significant bits first.
                let bit = x * bits_per_pixel;
                let shift = 8 - bits_per_pixel - bit % 8;
                let index = (source[bit / 8] >> shift) & ((1 << bits_per_pixel) - 1);
                let [r, g, b] = palette(index);
                [r, g, b, 0xFF]
            } else {
                pixel_to_rgba(&source[x * bits_per_pixel / 8..(x + 1) * bits_per_pixel / 8], palette)
            };
            rgba[(y * width + x) * 4..(y * width + x + 1) * 4].copy_from_slice(&[r, g, b, a]);
        }
    }

    let has_alpha = icon.bpp == 32 && rgba.chunks_exact(4).any(|pixel| pixel[3] != 0);
    let mask_stride = icon.bits_mask.len() / height;
    if !has_alpha && width.div_ceil(8) <= mask_stride {
        for y in 0..height {
            let mask = &icon.bits_mask[(height - 1 - y) * mask_stride..];
            for x in 0..width {
                let transparent = mask[x / 8] & (0x80 >> (x % 8)) != 0;
                rgba[(y * width + x) * 4 + 3] = if transparent { 0 } else { 0xFF };
            }
        }
    }

    Some((rgba, u32::from(icon.width), u32::from(icon.height)))
}

/// Converts one 8, 16 (RGB555), 24 or 32 (BGRA) bit icon pixel to RGBA8888.
fn pixel_to_rgba(pixel: &[u8], palette: impl Fn(u8) -> [u8; 3]) -> [u8; 4] {
    match *pixel {
        [index] => {
            let [r, g, b] = palette(index);
            [r, g, b, 0xFF]
        }
        [low, high] => {
            let value = u16::from_le_bytes([low, high]);
            let expand = |bits: u16| {
                let bits = u8::try_from(bits & 0x1F).unwrap_or(0);
                (bits << 3) | (bits >> 2)
            };
            [expand(value >> 10), expand(value >> 5), expand(value), 0xFF]
        }
        [b, g, r] => [r, g, b, 0xFF],
        [b, g, r, a] => [r, g, b, a],
        _ => [0, 0, 0, 0],
    }
}

type Surface = softbuffer::Surface<DisplayHandle<'static>, Arc<Window>>;

struct LocalWindow {
    window: Arc<Window>,
    surface: Surface,
    /// The RAIL window whose local window this one was created with as owner.
    owner: Option<u32>,
}

struct RailWindow {
    model: RailWindowModel,
    local: Option<LocalWindow>,
    pixels: Option<WindowPixels>,
    icon: Option<Icon>,
}

/// The local windows of a RemoteApp session.
#[derive(Default)]
pub(crate) struct RailWindows {
    windows: BTreeMap<u32, RailWindow>,
    /// Icons that arrived before their window's first state order.
    pending_icons: BTreeMap<u32, Icon>,
    /// Icons for Cached Icon orders, with their decoded size in bytes.
    icon_cache: BTreeMap<(u8, u16), (Icon, usize)>,
    icon_cache_bytes: usize,
    cursor: Option<Cursor>,
    /// Whether the server pointer is hidden; windows created later start the same way.
    cursor_hidden: bool,
}

impl RailWindows {
    /// Applies decoded windowing orders, creating, updating and closing local windows.
    pub(crate) fn apply_orders(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &softbuffer::Context<DisplayHandle<'static>>,
        orders: Vec<WindowingOrder>,
    ) {
        for order in orders {
            match order {
                WindowingOrder::Window(order) => self.apply_window_order(event_loop, context, order),
                WindowingOrder::Desktop(desktop) => {
                    if desktop.resets_windows() {
                        debug!("RAIL desktop reset; closing every RemoteApp window");
                        self.windows.clear();
                        self.pending_icons.clear();
                    }
                    if let DesktopOrder::ActivelyMonitored(desktop) = desktop {
                        if let Some(active_window_id) = desktop.active_window_id {
                            self.activate(active_window_id);
                        }
                        if desktop.z_order.is_some() {
                            // winit has no way to restack windows relative to each other.
                            debug!("RAIL z-order is not applied by the viewer");
                        }
                    }
                }
                WindowingOrder::NotifyIcon(order) => {
                    debug!(
                        window_id = order.window_id,
                        notify_icon_id = order.notify_icon_id,
                        "Notification icons are not shown by the viewer"
                    );
                }
            }
        }
    }

    fn apply_window_order(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &softbuffer::Context<DisplayHandle<'static>>,
        order: WindowOrder,
    ) {
        let window_id = order.window_id;
        match order.body {
            WindowOrderBody::Deleted => {
                self.windows.remove(&window_id);
                self.pending_icons.remove(&window_id);
            }
            WindowOrderBody::Icon(icon) => self.apply_icon(window_id, icon),
            WindowOrderBody::State(state) => {
                if !self.windows.contains_key(&window_id) {
                    if !order.is_new {
                        debug!(window_id, "Ignoring an update for an unknown RAIL window");
                        return;
                    }
                    // UNTESTED: a server cannot make the viewer hold more than MAX_RAIL_WINDOWS windows.
                    // This needs a winit event loop to exercise; keep the check before the insertion below.
                    if MAX_RAIL_WINDOWS <= self.windows.len() {
                        warn!(window_id, "Ignoring a RAIL window beyond the window limit");
                        return;
                    }
                }
                let pending_icons = &mut self.pending_icons;
                let window = self.windows.entry(window_id).or_insert_with(|| RailWindow {
                    model: RailWindowModel::default(),
                    local: None,
                    pixels: None,
                    icon: pending_icons.remove(&window_id),
                });
                let changes = window.model.apply(&state);
                self.sync_local_window(event_loop, context, window_id, changes);
            }
        }
    }

    fn apply_icon(&mut self, window_id: u32, icon: WindowIcon) {
        if icon.overlay {
            debug!(window_id, "Taskbar overlay icons are not shown by the viewer");
            return;
        }
        let local_icon = match &icon.icon {
            IconSource::Icon(info) => {
                let Some((local_icon, bytes)) = icon_to_rgba(info).and_then(|(rgba, width, height)| {
                    let bytes = rgba.len();
                    Icon::from_rgba(rgba, width, height)
                        .inspect_err(|error| debug!(%error, "Unable to create a RAIL window icon"))
                        .ok()
                        .map(|icon| (icon, bytes))
                }) else {
                    debug!(window_id, bpp = info.bpp, "Unsupported RAIL window icon");
                    return;
                };
                if info.cache_entry != IconInfo::NO_CACHE_ENTRY && info.cache_id != IconInfo::NO_CACHE_ID {
                    self.cache_icon((info.cache_id, info.cache_entry), local_icon.clone(), bytes);
                }
                local_icon
            }
            IconSource::Cached(cached) => match self.icon_cache.get(&(cached.cache_id, cached.cache_entry)) {
                Some((icon, _)) => icon.clone(),
                None => {
                    debug!(window_id, ?cached, "Unknown cached RAIL window icon");
                    return;
                }
            },
        };
        let Some(window) = self.windows.get_mut(&window_id) else {
            // The server may send a window's icon before its first state order.
            if self.pending_icons.contains_key(&window_id) {
                // The large icon is the one the taskbar shows; it replaces a queued small one.
                if icon.big {
                    self.pending_icons.insert(window_id, local_icon);
                }
            } else if self.pending_icons.len() < MAX_RAIL_WINDOWS {
                // UNTESTED: a server cannot queue icons for more than MAX_RAIL_WINDOWS unknown windows.
                // This needs a winit event loop to exercise; keep the bound on new entries only.
                self.pending_icons.insert(window_id, local_icon);
            } else {
                debug!(window_id, "Dropping an icon for an unknown RAIL window");
            }
            return;
        };
        // The large icon is the one the taskbar shows; the small one only fills a gap.
        if icon.big || window.icon.is_none() {
            if let Some(local) = &window.local {
                local.window.set_window_icon(Some(local_icon.clone()));
            }
            window.icon = Some(local_icon);
        }
    }

    /// Stores an icon for later Cached Icon orders, within [`MAX_CACHED_ICON_BYTES`].
    fn cache_icon(&mut self, key: (u8, u16), icon: Icon, bytes: usize) {
        let replaced = self.icon_cache.get(&key).map_or(0, |(_, bytes)| *bytes);
        let total = self.icon_cache_bytes - replaced + bytes;
        if MAX_CACHED_ICON_BYTES < total {
            debug!(?key, "Not caching a RAIL icon beyond the icon cache budget");
            return;
        }
        self.icon_cache.insert(key, (icon, bytes));
        self.icon_cache_bytes = total;
    }

    /// Brings the local window of the server's active window to the front when one of the
    /// RemoteApp windows already has focus, so a server change never takes focus from another
    /// local application.
    fn activate(&self, window_id: u32) {
        if window_id == MonitoredDesktop::NO_ACTIVE_WINDOW {
            return;
        }
        let has_focus = self
            .windows
            .values()
            .filter_map(|window| window.local.as_ref())
            .any(|local| local.window.has_focus());
        let target = self.windows.get(&window_id).and_then(|window| window.local.as_ref());
        if let Some(target) = target
            && has_focus
            && !target.window.has_focus()
        {
            target.window.focus_window();
        }
    }

    fn sync_local_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &softbuffer::Context<DisplayHandle<'static>>,
        window_id: u32,
        changes: ModelChanges,
    ) {
        let owner = self.attachable_owner(window_id);
        let cursor = self.cursor.clone();
        let cursor_hidden = self.cursor_hidden;
        let Some(window) = self.windows.get_mut(&window_id) else {
            return;
        };
        let model = &window.model;

        let Some(local) = &window.local else {
            if !model.is_visible() {
                return;
            }
            let owner_id = owner.as_ref().map(|(owner_id, _)| *owner_id);
            let owner_window = owner.map(|(_, window)| window);
            match create_local_window(event_loop, context, model, owner_window.as_deref()) {
                Ok(mut local) => {
                    local.owner = owner_id;
                    local.window.set_window_icon(window.icon.clone());
                    if let Some(cursor) = cursor {
                        local.window.set_cursor(cursor);
                    }
                    local.window.set_cursor_visible(!cursor_hidden);
                    window.local = Some(local);
                    self.attach_owned_windows(event_loop, context, window_id);
                }
                Err(error) => warn!(window_id, %error, "Unable to create a RemoteApp window"),
            }
            return;
        };

        if local.owner != owner.as_ref().map(|(owner_id, _)| *owner_id) {
            // An owner cannot be changed after a window is created, so the window and the
            // windows it owns are created again with the new owner.
            self.recreate_with_owned(event_loop, context, window_id);
            return;
        }
        if changes.title {
            local.window.set_title(&model.title);
        }
        if changes.taskbar {
            set_skip_taskbar(&local.window, model.skips_taskbar());
        }
        if changes.geometry {
            local.window.set_outer_position(PhysicalPosition::new(model.x, model.y));
            let (width, height) = bounded_window_size(model.width, model.height);
            let _ = local.window.request_inner_size(PhysicalSize::new(width, height));
        }
        if changes.show_state {
            match model.show_state {
                ShowState::Hide => local.window.set_visible(false),
                ShowState::Minimized => local.window.set_minimized(true),
                // The server sends the maximized geometry, which the window already follows.
                ShowState::Maximized | ShowState::Show => {
                    local.window.set_minimized(false);
                    local.window.set_visible(true);
                }
            }
        }
        local.window.request_redraw();
    }

    /// The owner a window's local window should be created with: its server owner's local
    /// window, when local windows take owners and the owner has one.
    ///
    /// A window whose owner chain leads back to itself gets no owner, so a cyclic ownership
    /// from the server cannot make owners and owned windows recreate each other forever.
    fn attachable_owner(&self, window_id: u32) -> Option<(u32, Arc<Window>)> {
        if !OWNER_WINDOWS_SUPPORTED {
            return None;
        }
        let owner_id = self.windows.get(&window_id)?.model.owner_window_id?;
        let mut ancestor = Some(owner_id);
        for _ in 0..=MAX_RAIL_WINDOWS {
            match ancestor {
                Some(id) if id == window_id => return None,
                Some(id) => ancestor = self.windows.get(&id).and_then(|window| window.model.owner_window_id),
                None => break,
            }
        }
        let owner = self.windows.get(&owner_id)?.local.as_ref()?;
        Some((owner_id, Arc::clone(&owner.window)))
    }

    /// Recreates the local windows owned by `owner_window_id` that were created while it had
    /// no local window, so they stay above it and minimize with it.
    ///
    /// The owner of a window cannot change after it is created, and the server may show an
    /// owned window before its owner.
    fn attach_owned_windows(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &softbuffer::Context<DisplayHandle<'static>>,
        owner_window_id: u32,
    ) {
        if !OWNER_WINDOWS_SUPPORTED {
            return;
        }
        let unattached = self
            .windows
            .iter()
            .filter(|(window_id, window)| {
                **window_id != owner_window_id
                    && window.model.owner_window_id == Some(owner_window_id)
                    && window
                        .local
                        .as_ref()
                        .is_some_and(|local| local.owner != Some(owner_window_id))
            })
            .map(|(window_id, _)| *window_id)
            .collect::<Vec<_>>();
        for window_id in unattached {
            self.recreate_with_owned(event_loop, context, window_id);
        }
    }

    /// Destroys the local windows of `window_id` and of every window it owns, directly or
    /// through other owned windows, then creates them again, owners first.
    ///
    /// Destroying a native owner also destroys the windows it owns, so they are dropped
    /// first and recreated with it rather than left pointing at destroyed windows.
    fn recreate_with_owned(
        &mut self,
        event_loop: &ActiveEventLoop,
        context: &softbuffer::Context<DisplayHandle<'static>>,
        window_id: u32,
    ) {
        let mut subtree = vec![window_id];
        let mut index = 0;
        while let Some(&owner_id) = subtree.get(index) {
            subtree.extend(
                self.windows
                    .iter()
                    .filter(|(id, window)| window.model.owner_window_id == Some(owner_id) && !subtree.contains(id))
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
            );
            index += 1;
        }
        // UNTESTED: owned windows are dropped before their owners and recreated after them, so no
        // native window outlives its owner and every owned window comes back. This needs a winit
        // event loop to exercise; keep both orders when changing this.
        for id in subtree.iter().rev() {
            if let Some(window) = self.windows.get_mut(id) {
                window.local = None;
            }
        }
        for id in subtree {
            if self.windows.get(&id).is_some_and(|window| window.local.is_none()) {
                self.sync_local_window(event_loop, context, id, ModelChanges::default());
            }
        }
    }

    /// Applies one graphics pipeline window update for the RAIL window it names.
    pub(crate) fn apply_surface_update(
        &mut self,
        window_id: u64,
        size: (u16, u16),
        region: &ExclusiveRectangle,
        rgba: &[u8],
    ) {
        let Ok(rail_window_id) = u32::try_from(window_id) else {
            debug!(window_id, "Graphics pipeline update for an unknown RAIL window");
            return;
        };
        let other_windows_pixels = self
            .windows
            .iter()
            .filter(|(id, _)| **id != rail_window_id)
            .filter_map(|(_, window)| window.pixels.as_ref())
            .map(|pixels| pixels.pixels.len())
            .sum();
        let Some(window) = self.windows.get_mut(&rail_window_id) else {
            debug!(window_id, "Graphics pipeline update for an unknown RAIL window");
            return;
        };
        if !fits_window_pixel_budget(size, other_windows_pixels) {
            debug!(
                window_id,
                ?size,
                "Graphics pipeline window content beyond the pixel budget"
            );
            // The window falls back to its rectangle of the session frame.
            window.pixels = None;
            return;
        }
        window.pixels.get_or_insert_default().apply(size, region, rgba);
        if let Some(local) = &window.local {
            local.window.request_redraw();
        }
    }

    /// Returns the model of a RAIL window.
    pub(crate) fn model(&self, window_id: u32) -> Option<&RailWindowModel> {
        self.windows.get(&window_id).map(|window| &window.model)
    }

    /// Returns the RAIL window ID of a local window.
    pub(crate) fn find(&self, id: WindowId) -> Option<u32> {
        self.windows.iter().find_map(|(window_id, window)| {
            window
                .local
                .as_ref()
                .is_some_and(|local| local.window.id() == id)
                .then_some(*window_id)
        })
    }

    pub(crate) fn request_redraw_all(&self) {
        for local in self.windows.values().filter_map(|window| window.local.as_ref()) {
            local.window.request_redraw();
        }
    }

    /// Applies a server pointer shape to every local window and shows the pointer.
    pub(crate) fn set_cursor(&mut self, cursor: Cursor) {
        for local in self.windows.values().filter_map(|window| window.local.as_ref()) {
            local.window.set_cursor(cursor.clone());
            local.window.set_cursor_visible(true);
        }
        self.cursor = Some(cursor);
        self.cursor_hidden = false;
    }

    pub(crate) fn set_cursor_visible(&mut self, visible: bool) {
        for local in self.windows.values().filter_map(|window| window.local.as_ref()) {
            local.window.set_cursor_visible(visible);
        }
        self.cursor_hidden = !visible;
    }

    /// Paints a RAIL window from its graphics pipeline content, or from the session frame.
    pub(crate) fn draw(&mut self, window_id: u32, frame: &[u32], frame_size: (u16, u16)) {
        let Some(window) = self.windows.get_mut(&window_id) else {
            return;
        };
        let Some(local) = window.local.as_mut() else {
            return;
        };
        let size = local.window.inner_size();
        let (Some(width), Some(height)) = (
            core::num::NonZeroU32::new(size.width),
            core::num::NonZeroU32::new(size.height),
        ) else {
            return;
        };
        if !fits_window_area(size.width, size.height) {
            // The platform may size a window past what it was asked for; painting it would
            // allocate a buffer beyond the window pixel budget.
            debug!(
                window_id,
                ?size,
                "RemoteApp window beyond the pixel budget is not painted"
            );
            return;
        }
        let pixels = match &window.pixels {
            Some(pixels) => pixels.crop(size.width, size.height),
            None if frame.is_empty() => return,
            None => crop_frame(
                frame,
                frame_size,
                (window.model.x, window.model.y),
                (size.width, size.height),
            ),
        };
        if let Err(error) = local.surface.resize(width, height) {
            debug!(window_id, %error, "Unable to resize a RemoteApp window surface");
            return;
        }
        match local.surface.buffer_mut() {
            Ok(mut buffer) => {
                buffer.copy_from_slice(&pixels);
                if let Err(error) = buffer.present() {
                    debug!(window_id, %error, "Unable to present a RemoteApp window");
                }
            }
            Err(error) => debug!(window_id, %error, "Unable to map a RemoteApp window surface"),
        }
    }
}

fn create_local_window(
    event_loop: &ActiveEventLoop,
    context: &softbuffer::Context<DisplayHandle<'static>>,
    model: &RailWindowModel,
    owner: Option<&Window>,
) -> anyhow::Result<LocalWindow> {
    let (width, height) = bounded_window_size(model.width, model.height);
    let attributes = WindowAttributes::default()
        .with_title(model.title.clone())
        .with_decorations(false)
        .with_position(PhysicalPosition::new(model.x, model.y))
        .with_inner_size(PhysicalSize::new(width, height))
        .with_active(model.activates_on_open());
    let attributes = windows_attributes(attributes, model, owner);
    let window = Arc::new(event_loop.create_window(attributes)?);
    if model.show_state == ShowState::Minimized {
        window.set_minimized(true);
    }
    let surface = softbuffer::Surface::new(context, Arc::clone(&window))
        .map_err(|error| anyhow::anyhow!("unable to create a window surface: {error}"))?;
    Ok(LocalWindow {
        window,
        surface,
        owner: None,
    })
}

/// Whether local windows take an owner window, which only the Windows backend supports.
const OWNER_WINDOWS_SUPPORTED: bool = cfg!(windows);

#[cfg(windows)]
fn windows_attributes(
    attributes: WindowAttributes,
    model: &RailWindowModel,
    owner: Option<&Window>,
) -> WindowAttributes {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use winit::platform::windows::WindowAttributesExtWindows as _;

    let attributes = attributes.with_skip_taskbar(model.skips_taskbar());
    let owner = owner
        .and_then(|owner| owner.window_handle().ok())
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        });
    match owner {
        // Owned windows stay above their owner and minimize with it, like on the server.
        Some(owner) => attributes.with_owner_window(owner),
        None => attributes,
    }
}

#[cfg(windows)]
fn set_skip_taskbar(window: &Window, skip: bool) {
    use winit::platform::windows::WindowExtWindows as _;

    window.set_skip_taskbar(skip);
}

#[cfg(not(windows))]
fn set_skip_taskbar(_window: &Window, _skip: bool) {}

#[cfg(not(windows))]
fn windows_attributes(
    attributes: WindowAttributes,
    _model: &RailWindowModel,
    _owner: Option<&Window>,
) -> WindowAttributes {
    attributes
}

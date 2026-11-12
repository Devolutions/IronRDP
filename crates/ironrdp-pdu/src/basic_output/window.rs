//! Windowing Alternate Secondary Drawing Orders.
//!
//! These orders are carried by slow-path and Fast-Path Orders updates as defined
//! in [MS-RDPERP] section 2.2.1.3.
//! Each order is decoded into a typed window, notification icon, or desktop order.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, InvalidFieldErr, ReadCursor, WriteCursor, cast_length, ensure_size,
    invalid_field_err,
};

use crate::geometry::ExclusiveRectangle;

const WINDOWING_ORDER_CONTROL_FLAGS: u8 = 0x2e;
const ALTERNATE_SECONDARY_HEADER_SIZE: usize = 1 /* controlFlags */ + 2 /* orderSize */;
const ORDER_FLAGS_SIZE: usize = 4 /* fieldsPresent */;

const WINDOW_TYPE: u32 = 0x0100_0000;
const NOTIFY_ICON_TYPE: u32 = 0x0200_0000;
const DESKTOP_TYPE: u32 = 0x0400_0000;
const ORDER_TYPE_MASK: u32 = WINDOW_TYPE | NOTIFY_ICON_TYPE | DESKTOP_TYPE;

const STATE_NEW: u32 = 0x1000_0000;
const DELETED: u32 = 0x2000_0000;
const ICON: u32 = 0x4000_0000;
const CACHED_ICON: u32 = 0x8000_0000;

const FIELD_APPBAR_EDGE: u32 = 0x0000_0001;
const FIELD_OWNER: u32 = 0x0000_0002;
const FIELD_TITLE: u32 = 0x0000_0004;
const FIELD_STYLE: u32 = 0x0000_0008;
const FIELD_SHOW: u32 = 0x0000_0010;
const FIELD_APPBAR_STATE: u32 = 0x0000_0040;
const FIELD_RESIZE_MARGIN_X: u32 = 0x0000_0080;
const FIELD_WND_RECTS: u32 = 0x0000_0100;
const FIELD_VISIBILITY: u32 = 0x0000_0200;
const FIELD_WND_SIZE: u32 = 0x0000_0400;
const FIELD_WND_OFFSET: u32 = 0x0000_0800;
const FIELD_VIS_OFFSET: u32 = 0x0000_1000;
const FIELD_ICON_BIG: u32 = 0x0000_2000;
const FIELD_CLIENT_AREA_OFFSET: u32 = 0x0000_4000;
const FIELD_WND_CLIENT_DELTA: u32 = 0x0000_8000;
const FIELD_CLIENT_AREA_SIZE: u32 = 0x0001_0000;
const FIELD_RP_CONTENT: u32 = 0x0002_0000;
const FIELD_ROOT_PARENT: u32 = 0x0004_0000;
const FIELD_ENFORCE_SERVER_ZORDER: u32 = 0x0008_0000;
const FIELD_ICON_OVERLAY: u32 = 0x0010_0000;
const FIELD_ICON_OVERLAY_NULL: u32 = 0x0020_0000;
const FIELD_OVERLAY_DESCRIPTION: u32 = 0x0040_0000;
const FIELD_TASKBAR_BUTTON: u32 = 0x0080_0000;
const FIELD_RESIZE_MARGIN_Y: u32 = 0x0800_0000;

const FIELD_NOTIFY_TIP: u32 = 0x0000_0001;
const FIELD_NOTIFY_INFO_TIP: u32 = 0x0000_0002;
const FIELD_NOTIFY_STATE: u32 = 0x0000_0004;
const FIELD_NOTIFY_VERSION: u32 = 0x0000_0008;

const FIELD_DESKTOP_NONE: u32 = 0x0000_0001;
const FIELD_DESKTOP_HOOKED: u32 = 0x0000_0002;
const FIELD_DESKTOP_ARC_COMPLETED: u32 = 0x0000_0004;
const FIELD_DESKTOP_ARC_BEGAN: u32 = 0x0000_0008;
const FIELD_DESKTOP_ZORDER: u32 = 0x0000_0010;
const FIELD_DESKTOP_ACTIVE_WND: u32 = 0x0000_0020;

const MAX_TITLE_SIZE: usize = 520;
const MAX_INFO_TIP_TEXT_SIZE: usize = 510;
const MAX_INFO_TIP_TITLE_SIZE: usize = 126;

/// A Windowing Alternate Secondary Drawing Order ([MS-RDPERP] 2.2.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowingOrder {
    Window(WindowOrder),
    NotifyIcon(NotifyIconOrder),
    Desktop(DesktopOrder),
}

impl WindowingOrder {
    const NAME: &'static str = "WindowingOrder";

    /// Returns whether this order requires extended Window List support
    /// (`TS_WINDOW_LEVEL_SUPPORTED_EX`).
    pub fn requires_extended_support(&self) -> bool {
        self.fields_present() & (FIELD_CLIENT_AREA_SIZE | FIELD_RP_CONTENT | FIELD_ROOT_PARENT) != 0
    }

    /// Returns the `fieldsPresentFlags` value this order is encoded with.
    pub fn fields_present(&self) -> u32 {
        match self {
            Self::Window(order) => order.fields_present(),
            Self::NotifyIcon(order) => order.fields_present(),
            Self::Desktop(order) => order.fields_present(),
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::Window(order) => order.body_size(),
            Self::NotifyIcon(order) => order.body_size(),
            Self::Desktop(order) => order.body_size(),
        }
    }
}

impl Encode for WindowingOrder {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let order_size: u16 = cast_length!("orderSize", self.size())?;
        dst.write_u8(WINDOWING_ORDER_CONTROL_FLAGS);
        dst.write_u16(order_size);
        dst.write_u32(self.fields_present());
        match self {
            Self::Window(order) => order.encode_body(dst),
            Self::NotifyIcon(order) => order.encode_body(dst),
            Self::Desktop(order) => order.encode_body(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE + self.body_size()
    }
}

impl<'de> Decode<'de> for WindowingOrder {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: ALTERNATE_SECONDARY_HEADER_SIZE);

        let control_flags = src.read_u8();
        if control_flags != WINDOWING_ORDER_CONTROL_FLAGS {
            return Err(invalid_field_err!(
                "controlFlags",
                "expected a windowing alternate secondary drawing order"
            ));
        }

        let order_size = usize::from(src.read_u16());
        if order_size < ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE {
            return Err(invalid_field_err!(
                "orderSize",
                "order is smaller than its common header"
            ));
        }

        ensure_size!(in: src, size: order_size - ALTERNATE_SECONDARY_HEADER_SIZE);
        let mut order_data = ReadCursor::new(src.read_slice(order_size - ALTERNATE_SECONDARY_HEADER_SIZE));
        let fields_present = order_data.read_u32();
        let order = match fields_present & ORDER_TYPE_MASK {
            WINDOW_TYPE => Self::Window(WindowOrder::decode_body(fields_present, &mut order_data)?),
            NOTIFY_ICON_TYPE => Self::NotifyIcon(NotifyIconOrder::decode_body(fields_present, &mut order_data)?),
            DESKTOP_TYPE => Self::Desktop(DesktopOrder::decode_body(fields_present, &mut order_data)?),
            _ => return Err(invalid_field_err!("fieldsPresent", "invalid windowing order type")),
        };
        if !order_data.eof() {
            return Err(invalid_field_err!("orderSize", "order contains trailing data"));
        }

        Ok(order)
    }
}

/// A Window Information order ([MS-RDPERP] 2.2.1.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOrder {
    pub window_id: u32,
    /// `WINDOW_ORDER_STATE_NEW`: the order creates the window, notification icon, or icon.
    pub is_new: bool,
    pub body: WindowOrderBody,
}

/// The content of a [`WindowOrder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowOrderBody {
    /// A New or Existing Window order ([MS-RDPERP] 2.2.1.3.1.2.1).
    State(Box<WindowState>),
    /// A Window Icon or Cached Icon order ([MS-RDPERP] 2.2.1.3.1.2.2 and 2.2.1.3.1.2.3).
    Icon(WindowIcon),
    /// A Deleted Window order ([MS-RDPERP] 2.2.1.3.1.2.4).
    Deleted,
}

/// The fields of a New or Existing Window order ([MS-RDPERP] 2.2.1.3.1.2.1).
///
/// A `None` field is absent from the order and leaves the client's current value unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowState {
    /// The owner window ID; `0` means the window has no owner.
    pub owner_window_id: Option<u32>,
    pub style: Option<WindowStyle>,
    pub show_state: Option<ShowState>,
    pub title: Option<String>,
    /// Client area offset in screen coordinates.
    pub client_offset: Option<WindowPoint>,
    pub client_area_size: Option<WindowSize>,
    /// Left and right resize margins.
    pub resize_margin_x: Option<ResizeMargins>,
    /// Top and bottom resize margins.
    pub resize_margin_y: Option<ResizeMargins>,
    pub rp_content: Option<bool>,
    pub root_parent_handle: Option<u32>,
    /// Window offset in screen coordinates.
    pub window_offset: Option<WindowPoint>,
    /// Offset of the client area relative to the window origin.
    pub client_delta: Option<WindowPoint>,
    pub window_size: Option<WindowSize>,
    /// The window shape, relative to the window origin.
    pub window_rects: Option<Vec<ExclusiveRectangle>>,
    /// Offset of the visibility region in screen coordinates.
    pub visible_offset: Option<WindowPoint>,
    /// The visible region, relative to [`Self::visible_offset`].
    pub visibility_rects: Option<Vec<ExclusiveRectangle>>,
    /// Accessible description of the taskbar overlay icon.
    pub overlay_description: Option<String>,
    /// `WINDOW_ORDER_FIELD_ICON_OVERLAY_NULL`: the taskbar overlay icon is removed.
    pub icon_overlay_null: bool,
    /// `0` shows the window on the taskbar; nonzero hides it.
    pub taskbar_button: Option<u8>,
    pub enforce_server_z_order: Option<bool>,
    pub app_bar_state: Option<u8>,
    pub app_bar_edge: Option<u8>,
}

/// Window style and extended style (`WS_*` and `WS_EX_*` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowStyle {
    pub style: u32,
    pub extended_style: u32,
}

/// A signed screen or window-relative position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPoint {
    pub x: i32,
    pub y: i32,
}

/// A window or client area size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

/// A pair of resize margins along one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResizeMargins {
    /// Left or top margin.
    pub start: u32,
    /// Right or bottom margin.
    pub end: u32,
}

/// A window show state ([MS-RDPERP] 2.2.1.3.1.2.1, `ShowState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShowState {
    Hide = 0,
    Minimized = 2,
    Maximized = 3,
    Show = 5,
}

impl ShowState {
    /// Returns the wire value.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Hide => 0,
            Self::Minimized => 2,
            Self::Maximized => 3,
            Self::Show => 5,
        }
    }
}

impl TryFrom<u8> for ShowState {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Hide),
            2 => Ok(Self::Minimized),
            3 => Ok(Self::Maximized),
            5 => Ok(Self::Show),
            _ => Err(value),
        }
    }
}

/// A Window Icon or Cached Icon order ([MS-RDPERP] 2.2.1.3.1.2.2 and 2.2.1.3.1.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIcon {
    /// `WINDOW_ORDER_FIELD_ICON_BIG`: the large (taskbar and Alt+Tab) icon rather than the small one.
    pub big: bool,
    /// `WINDOW_ORDER_FIELD_ICON_OVERLAY`: the taskbar overlay icon.
    pub overlay: bool,
    pub icon: IconSource,
}

/// An icon carried inline or referenced from the client's icon cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    Icon(IconInfo),
    Cached(CachedIconInfo),
}

impl IconSource {
    fn size(&self) -> usize {
        match self {
            Self::Icon(icon) => icon.size(),
            Self::Cached(_) => CachedIconInfo::SIZE,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::Icon(icon) => icon.encode(dst),
            Self::Cached(cached) => {
                cached.encode(dst);
                Ok(())
            }
        }
    }

    const fn flag(&self) -> u32 {
        match self {
            Self::Icon(_) => ICON,
            Self::Cached(_) => CACHED_ICON,
        }
    }
}

/// TS_ICON_INFO ([MS-RDPERP] 2.2.1.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconInfo {
    /// Icon cache entry to store the icon in; `0xFFFF` does not cache it.
    pub cache_entry: u16,
    /// Icon cache to store the icon in; `0xFF` does not cache it.
    pub cache_id: u8,
    /// Color depth of [`Self::bits_color`]: 1, 4, 8, 16, 24, or 32.
    pub bpp: u8,
    pub width: u16,
    pub height: u16,
    /// Monochrome AND mask, bottom-up with rows padded to 2 bytes.
    pub bits_mask: Vec<u8>,
    /// Palette for 1, 4, and 8 bpp icons, as `RGBQUAD` entries.
    pub color_table: Vec<u8>,
    /// Color bitmap, bottom-up with rows padded to 2 bytes.
    pub bits_color: Vec<u8>,
}

impl IconInfo {
    /// The `CacheEntry` and `CacheId` values that do not store the icon.
    pub const NO_CACHE_ENTRY: u16 = 0xFFFF;
    pub const NO_CACHE_ID: u8 = 0xFF;

    const fn has_color_table(bpp: u8) -> bool {
        matches!(bpp, 1 | 4 | 8)
    }

    fn size(&self) -> usize {
        2 /* CacheEntry */
            + 1 /* CacheId */
            + 1 /* Bpp */
            + 2 /* Width */
            + 2 /* Height */
            + if Self::has_color_table(self.bpp) { 2 /* CbColorTable */ } else { 0 }
            + 2 /* CbBitsMask */
            + 2 /* CbBitsColor */
            + self.bits_mask.len() /* BitsMask */
            + self.color_table.len() /* ColorTable */
            + self.bits_color.len() /* BitsColor */
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8 /* CacheEntry, CacheId, Bpp, Width, Height */);
        let cache_entry = src.read_u16();
        let cache_id = src.read_u8();
        let bpp = src.read_u8();
        if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
            return Err(invalid_field_err!("Bpp", "unsupported icon color depth"));
        }
        let width = src.read_u16();
        let height = src.read_u16();
        let color_table_len = if Self::has_color_table(bpp) {
            ensure_size!(in: src, size: 2 /* CbColorTable */);
            usize::from(src.read_u16())
        } else {
            0
        };
        ensure_size!(in: src, size: 4 /* CbBitsMask, CbBitsColor */);
        let mask_len = usize::from(src.read_u16());
        let color_len = usize::from(src.read_u16());
        ensure_size!(in: src, size: mask_len + color_table_len + color_len);
        let bits_mask = src.read_slice(mask_len).to_vec();
        let color_table = src.read_slice(color_table_len).to_vec();
        let bits_color = src.read_slice(color_len).to_vec();

        Ok(Self {
            cache_entry,
            cache_id,
            bpp,
            width,
            height,
            bits_mask,
            color_table,
            bits_color,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.bpp, 1 | 4 | 8 | 16 | 24 | 32) {
            return Err(invalid_field_err!("Bpp", "unsupported icon color depth"));
        }
        if !Self::has_color_table(self.bpp) && !self.color_table.is_empty() {
            return Err(invalid_field_err!(
                "ColorTable",
                "only 1, 4, and 8 bpp icons carry a color table"
            ));
        }
        let mask_len: u16 = cast_length!("CbBitsMask", self.bits_mask.len())?;
        let color_len: u16 = cast_length!("CbBitsColor", self.bits_color.len())?;

        dst.write_u16(self.cache_entry);
        dst.write_u8(self.cache_id);
        dst.write_u8(self.bpp);
        dst.write_u16(self.width);
        dst.write_u16(self.height);
        if Self::has_color_table(self.bpp) {
            let color_table_len: u16 = cast_length!("CbColorTable", self.color_table.len())?;
            dst.write_u16(color_table_len);
        }
        dst.write_u16(mask_len);
        dst.write_u16(color_len);
        dst.write_slice(&self.bits_mask);
        dst.write_slice(&self.color_table);
        dst.write_slice(&self.bits_color);
        Ok(())
    }
}

/// TS_CACHED_ICON_INFO ([MS-RDPERP] 2.2.1.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedIconInfo {
    pub cache_entry: u16,
    pub cache_id: u8,
}

impl CachedIconInfo {
    const SIZE: usize = 2 /* CacheEntry */ + 1 /* CacheId */;

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        Ok(Self {
            cache_entry: src.read_u16(),
            cache_id: src.read_u8(),
        })
    }

    fn encode(self, dst: &mut WriteCursor<'_>) {
        dst.write_u16(self.cache_entry);
        dst.write_u8(self.cache_id);
    }
}

impl WindowOrder {
    fn fields_present(&self) -> u32 {
        let new = if self.is_new { STATE_NEW } else { 0 };
        WINDOW_TYPE
            | new
            | match &self.body {
                WindowOrderBody::State(state) => state.fields_present(),
                WindowOrderBody::Icon(icon) => {
                    icon.icon.flag()
                        | if icon.big { FIELD_ICON_BIG } else { 0 }
                        | if icon.overlay { FIELD_ICON_OVERLAY } else { 0 }
                }
                WindowOrderBody::Deleted => DELETED,
            }
    }

    fn body_size(&self) -> usize {
        4 /* WindowId */
            + match &self.body {
                WindowOrderBody::State(state) => state.size(),
                WindowOrderBody::Icon(icon) => icon.icon.size(),
                WindowOrderBody::Deleted => 0,
            }
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.is_new && matches!(self.body, WindowOrderBody::Deleted) {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "deleted window order contains additional fields"
            ));
        }
        dst.write_u32(self.window_id);
        match &self.body {
            WindowOrderBody::State(state) => state.encode(dst),
            WindowOrderBody::Icon(icon) => icon.icon.encode(dst),
            WindowOrderBody::Deleted => Ok(()),
        }
    }

    fn decode_body(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        const ICON_FLAGS: u32 = FIELD_ICON_OVERLAY | FIELD_ICON_BIG;
        const ALLOWED: u32 = WINDOW_TYPE
            | FIELD_APPBAR_EDGE
            | FIELD_OWNER
            | FIELD_TITLE
            | FIELD_STYLE
            | FIELD_SHOW
            | FIELD_APPBAR_STATE
            | FIELD_RESIZE_MARGIN_X
            | FIELD_WND_RECTS
            | FIELD_VISIBILITY
            | FIELD_WND_SIZE
            | FIELD_WND_OFFSET
            | FIELD_VIS_OFFSET
            | FIELD_ICON_BIG
            | FIELD_CLIENT_AREA_OFFSET
            | FIELD_WND_CLIENT_DELTA
            | FIELD_CLIENT_AREA_SIZE
            | FIELD_RP_CONTENT
            | FIELD_ROOT_PARENT
            | FIELD_ENFORCE_SERVER_ZORDER
            | FIELD_ICON_OVERLAY
            | FIELD_ICON_OVERLAY_NULL
            | FIELD_OVERLAY_DESCRIPTION
            | FIELD_TASKBAR_BUTTON
            | FIELD_RESIZE_MARGIN_Y
            | STATE_NEW
            | DELETED
            | ICON
            | CACHED_ICON;

        if flags & !ALLOWED != 0 {
            return Err(invalid_field_err!("fieldsPresent", "unknown window order flag"));
        }
        ensure_size!(in: src, size: 4 /* WindowId */);
        let window_id = src.read_u32();
        let is_new = flags & STATE_NEW != 0;

        if flags & DELETED != 0 {
            if flags != WINDOW_TYPE | DELETED {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "deleted window order contains additional fields"
                ));
            }
            return Ok(Self {
                window_id,
                is_new,
                body: WindowOrderBody::Deleted,
            });
        }

        if flags & (ICON | CACHED_ICON) != 0 {
            if flags & (ICON | CACHED_ICON) == ICON | CACHED_ICON {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "window order contains both icon representations"
                ));
            }
            if flags & !(WINDOW_TYPE | STATE_NEW | ICON | CACHED_ICON | ICON_FLAGS) != 0 {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "invalid flags for a window icon order"
                ));
            }
            let icon = if flags & ICON != 0 {
                IconSource::Icon(IconInfo::decode(src)?)
            } else {
                IconSource::Cached(CachedIconInfo::decode(src)?)
            };
            return Ok(Self {
                window_id,
                is_new,
                body: WindowOrderBody::Icon(WindowIcon {
                    big: flags & FIELD_ICON_BIG != 0,
                    overlay: flags & FIELD_ICON_OVERLAY != 0,
                    icon,
                }),
            });
        }

        // ICON_BIG and ICON_OVERLAY carry no data of their own and only qualify an icon,
        // so without one they are ignored rather than rejected, as before typed decoding.
        // The decoded state does not keep them, so they do not survive re-encoding.

        Ok(Self {
            window_id,
            is_new,
            body: WindowOrderBody::State(Box::new(WindowState::decode(flags, src)?)),
        })
    }
}

impl WindowState {
    fn fields_present(&self) -> u32 {
        let mut flags = 0;
        let mut set = |present: bool, flag: u32| {
            if present {
                flags |= flag;
            }
        };
        set(self.owner_window_id.is_some(), FIELD_OWNER);
        set(self.style.is_some(), FIELD_STYLE);
        set(self.show_state.is_some(), FIELD_SHOW);
        set(self.title.is_some(), FIELD_TITLE);
        set(self.client_offset.is_some(), FIELD_CLIENT_AREA_OFFSET);
        set(self.client_area_size.is_some(), FIELD_CLIENT_AREA_SIZE);
        set(self.resize_margin_x.is_some(), FIELD_RESIZE_MARGIN_X);
        set(self.resize_margin_y.is_some(), FIELD_RESIZE_MARGIN_Y);
        set(self.rp_content.is_some(), FIELD_RP_CONTENT);
        set(self.root_parent_handle.is_some(), FIELD_ROOT_PARENT);
        set(self.window_offset.is_some(), FIELD_WND_OFFSET);
        set(self.client_delta.is_some(), FIELD_WND_CLIENT_DELTA);
        set(self.window_size.is_some(), FIELD_WND_SIZE);
        set(self.window_rects.is_some(), FIELD_WND_RECTS);
        set(self.visible_offset.is_some(), FIELD_VIS_OFFSET);
        set(self.visibility_rects.is_some(), FIELD_VISIBILITY);
        set(self.overlay_description.is_some(), FIELD_OVERLAY_DESCRIPTION);
        set(self.icon_overlay_null, FIELD_ICON_OVERLAY_NULL);
        set(self.taskbar_button.is_some(), FIELD_TASKBAR_BUTTON);
        set(self.enforce_server_z_order.is_some(), FIELD_ENFORCE_SERVER_ZORDER);
        set(self.app_bar_state.is_some(), FIELD_APPBAR_STATE);
        set(self.app_bar_edge.is_some(), FIELD_APPBAR_EDGE);
        flags
    }

    fn size(&self) -> usize {
        return sized(&self.owner_window_id, |_| 4 /* OwnerWindowId */)
            + sized(&self.style, |_| 8 /* Style, ExtendedStyle */)
            + sized(&self.show_state, |_| 1 /* ShowState */)
            + sized(&self.title, |title| unicode_string_size(title) /* TitleInfo */)
            + sized(&self.client_offset, |_| 8 /* ClientOffsetX, ClientOffsetY */)
            + sized(
                &self.client_area_size,
                |_| 8, /* ClientAreaWidth, ClientAreaHeight */
            )
            + sized(
                &self.resize_margin_x,
                |_| 8, /* WindowLeftResizeMargin, WindowRightResizeMargin */
            )
            + sized(
                &self.resize_margin_y,
                |_| 8, /* WindowTopResizeMargin, WindowBottomResizeMargin */
            )
            + sized(&self.rp_content, |_| 1 /* RPContent */)
            + sized(&self.root_parent_handle, |_| 4 /* RootParentHandle */)
            + sized(&self.window_offset, |_| 8 /* WindowOffsetX, WindowOffsetY */)
            + sized(
                &self.client_delta,
                |_| 8, /* WindowClientDeltaX, WindowClientDeltaY */
            )
            + sized(&self.window_size, |_| 8 /* WindowWidth, WindowHeight */)
            + sized(
                &self.window_rects,
                |rects| rectangles_size(rects), /* NumWindowRects, WindowRects */
            )
            + sized(&self.visible_offset, |_| 8 /* VisibleOffsetX, VisibleOffsetY */)
            + sized(&self.visibility_rects, |rects| {
                rectangles_size(rects) /* NumVisibilityRects, VisibilityRects */
            })
            + sized(&self.overlay_description, |description| {
                unicode_string_size(description) /* OverlayDescription */
            })
            + sized(&self.taskbar_button, |_| 1 /* TaskbarButton */)
            + sized(&self.enforce_server_z_order, |_| 1 /* EnforceServerZOrder */)
            + sized(&self.app_bar_state, |_| 1 /* AppBarState */)
            + sized(&self.app_bar_edge, |_| 1 /* AppBarEdge */);

        fn sized<T>(value: &Option<T>, size: impl FnOnce(&T) -> usize) -> usize {
            value.as_ref().map_or(0, size)
        }
    }

    fn decode(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let present = |flag: u32| flags & flag != 0;

        let owner_window_id = read_if(
            present(FIELD_OWNER),
            src,
            4, /* OwnerWindowId */
            ReadCursor::read_u32,
        )?;
        let style = read_if(
            present(FIELD_STYLE),
            src,
            8, /* Style, ExtendedStyle */
            |src| WindowStyle {
                style: src.read_u32(),
                extended_style: src.read_u32(),
            },
        )?;
        let show_state = read_if(present(FIELD_SHOW), src, 1 /* ShowState */, ReadCursor::read_u8)?
            .map(|value| {
                ShowState::try_from(value).map_err(|_| invalid_field_err!("ShowState", "invalid window show state"))
            })
            .transpose()?;
        let title = present(FIELD_TITLE)
            .then(|| read_unicode_string(src, MAX_TITLE_SIZE))
            .transpose()?;
        let client_offset = read_if(
            present(FIELD_CLIENT_AREA_OFFSET),
            src,
            8, /* ClientOffsetX, ClientOffsetY */
            read_point,
        )?;
        let client_area_size = read_if(
            present(FIELD_CLIENT_AREA_SIZE),
            src,
            8, /* ClientAreaWidth, ClientAreaHeight */
            read_size,
        )?;
        let resize_margin_x = read_if(
            present(FIELD_RESIZE_MARGIN_X),
            src,
            8, /* WindowLeftResizeMargin, WindowRightResizeMargin */
            read_margins,
        )?;
        let resize_margin_y = read_if(
            present(FIELD_RESIZE_MARGIN_Y),
            src,
            8, /* WindowTopResizeMargin, WindowBottomResizeMargin */
            read_margins,
        )?;
        let rp_content = read_if(
            present(FIELD_RP_CONTENT),
            src,
            1, /* RPContent */
            ReadCursor::read_u8,
        )?
        .map(|value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid_field_err!("RPContent", "invalid render plugin content")),
        })
        .transpose()?;
        let root_parent_handle = read_if(
            present(FIELD_ROOT_PARENT),
            src,
            4, /* RootParentHandle */
            ReadCursor::read_u32,
        )?;
        let window_offset = read_if(
            present(FIELD_WND_OFFSET),
            src,
            8, /* WindowOffsetX, WindowOffsetY */
            read_point,
        )?;
        let client_delta = read_if(
            present(FIELD_WND_CLIENT_DELTA),
            src,
            8, /* WindowClientDeltaX, WindowClientDeltaY */
            read_point,
        )?;
        let window_size = read_if(
            present(FIELD_WND_SIZE),
            src,
            8, /* WindowWidth, WindowHeight */
            read_size,
        )?;
        let window_rects = present(FIELD_WND_RECTS).then(|| read_rectangles(src)).transpose()?;
        let visible_offset = read_if(
            present(FIELD_VIS_OFFSET),
            src,
            8, /* VisibleOffsetX, VisibleOffsetY */
            read_point,
        )?;
        let visibility_rects = present(FIELD_VISIBILITY).then(|| read_rectangles(src)).transpose()?;
        let overlay_description = present(FIELD_OVERLAY_DESCRIPTION)
            .then(|| read_unicode_string(src, usize::from(u16::MAX)))
            .transpose()?;
        let taskbar_button = read_if(
            present(FIELD_TASKBAR_BUTTON),
            src,
            1, /* TaskbarButton */
            ReadCursor::read_u8,
        )?;
        let enforce_server_z_order = read_if(
            present(FIELD_ENFORCE_SERVER_ZORDER),
            src,
            1, /* EnforceServerZOrder */
            ReadCursor::read_u8,
        )?
        .map(|value| value != 0);
        let app_bar_state = read_if(
            present(FIELD_APPBAR_STATE),
            src,
            1, /* AppBarState */
            ReadCursor::read_u8,
        )?;
        let app_bar_edge = read_if(
            present(FIELD_APPBAR_EDGE),
            src,
            1, /* AppBarEdge */
            ReadCursor::read_u8,
        )?;

        Ok(Self {
            owner_window_id,
            style,
            show_state,
            title,
            client_offset,
            client_area_size,
            resize_margin_x,
            resize_margin_y,
            rp_content,
            root_parent_handle,
            window_offset,
            client_delta,
            window_size,
            window_rects,
            visible_offset,
            visibility_rects,
            overlay_description,
            icon_overlay_null: present(FIELD_ICON_OVERLAY_NULL),
            taskbar_button,
            enforce_server_z_order,
            app_bar_state,
            app_bar_edge,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if let Some(owner_window_id) = self.owner_window_id {
            dst.write_u32(owner_window_id);
        }
        if let Some(style) = self.style {
            dst.write_u32(style.style);
            dst.write_u32(style.extended_style);
        }
        if let Some(show_state) = self.show_state {
            dst.write_u8(show_state.as_u8());
        }
        if let Some(title) = &self.title {
            write_unicode_string(dst, title, MAX_TITLE_SIZE)?;
        }
        if let Some(point) = self.client_offset {
            write_point(dst, point);
        }
        if let Some(size) = self.client_area_size {
            write_size(dst, size);
        }
        if let Some(margins) = self.resize_margin_x {
            write_margins(dst, margins);
        }
        if let Some(margins) = self.resize_margin_y {
            write_margins(dst, margins);
        }
        if let Some(rp_content) = self.rp_content {
            dst.write_u8(u8::from(rp_content));
        }
        if let Some(handle) = self.root_parent_handle {
            dst.write_u32(handle);
        }
        if let Some(point) = self.window_offset {
            write_point(dst, point);
        }
        if let Some(point) = self.client_delta {
            write_point(dst, point);
        }
        if let Some(size) = self.window_size {
            write_size(dst, size);
        }
        if let Some(rects) = &self.window_rects {
            write_rectangles(dst, rects)?;
        }
        if let Some(point) = self.visible_offset {
            write_point(dst, point);
        }
        if let Some(rects) = &self.visibility_rects {
            write_rectangles(dst, rects)?;
        }
        if let Some(description) = &self.overlay_description {
            write_unicode_string(dst, description, usize::from(u16::MAX))?;
        }
        if let Some(value) = self.taskbar_button {
            dst.write_u8(value);
        }
        if let Some(value) = self.enforce_server_z_order {
            dst.write_u8(u8::from(value));
        }
        if let Some(value) = self.app_bar_state {
            dst.write_u8(value);
        }
        if let Some(value) = self.app_bar_edge {
            dst.write_u8(value);
        }
        Ok(())
    }
}

/// A Notification Icon Information order ([MS-RDPERP] 2.2.1.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyIconOrder {
    pub window_id: u32,
    pub notify_icon_id: u32,
    pub is_new: bool,
    /// `None` for a Deleted Notification Icon order ([MS-RDPERP] 2.2.1.3.2.2.3).
    pub state: Option<Box<NotifyIconState>>,
}

/// The fields of a New or Existing Notification Icon order ([MS-RDPERP] 2.2.1.3.2.2.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotifyIconState {
    /// `NOTIFYICON_VERSION`: 0, 3, or 4.
    pub version: Option<u32>,
    pub tooltip: Option<String>,
    pub info_tip: Option<NotifyIconInfoTip>,
    /// `1` hides the icon.
    pub state: Option<u32>,
    pub icon: Option<IconSource>,
}

/// TS_NOTIFY_ICON_INFOTIP ([MS-RDPERP] 2.2.1.3.2.2.2): a balloon tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyIconInfoTip {
    /// Display time in milliseconds.
    pub timeout: u32,
    /// `NIIF_*` flags; the low nibble selects the balloon icon.
    pub info_flags: u32,
    pub text: String,
    pub title: String,
}

impl NotifyIconInfoTip {
    pub const NIIF_INFO: u32 = 0x0000_0001;
}

impl NotifyIconOrder {
    fn fields_present(&self) -> u32 {
        let new = if self.is_new { STATE_NEW } else { 0 };
        NOTIFY_ICON_TYPE
            | new
            | match &self.state {
                Some(state) => {
                    let mut flags = state.icon.as_ref().map_or(0, IconSource::flag);
                    if state.version.is_some() {
                        flags |= FIELD_NOTIFY_VERSION;
                    }
                    if state.tooltip.is_some() {
                        flags |= FIELD_NOTIFY_TIP;
                    }
                    if state.info_tip.is_some() {
                        flags |= FIELD_NOTIFY_INFO_TIP;
                    }
                    if state.state.is_some() {
                        flags |= FIELD_NOTIFY_STATE;
                    }
                    flags
                }
                None => DELETED,
            }
    }

    fn body_size(&self) -> usize {
        4 /* WindowId */
            + 4 /* NotifyIconId */
            + self.state.as_ref().map_or(0, |state| {
                state.version.map_or(0, |_| 4 /* Version */)
                    + state.tooltip.as_deref().map_or(0, unicode_string_size)
                    + state.info_tip.as_ref().map_or(0, |tip| {
                        4 /* Timeout */
                            + 4 /* InfoFlags */
                            + unicode_string_size(&tip.text)
                            + unicode_string_size(&tip.title)
                    })
                    + state.state.map_or(0, |_| 4 /* State */)
                    + state.icon.as_ref().map_or(0, IconSource::size)
            })
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        dst.write_u32(self.window_id);
        dst.write_u32(self.notify_icon_id);
        let Some(state) = &self.state else {
            if self.is_new {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "deleted notification icon contains additional fields"
                ));
            }
            return Ok(());
        };
        if self.is_new && state.icon.is_none() {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "new notification icon has no icon representation"
            ));
        }
        if let Some(version) = state.version {
            validate_notify_icon_version(version)?;
            dst.write_u32(version);
        }
        if let Some(tooltip) = &state.tooltip {
            write_unicode_string(dst, tooltip, usize::from(u16::MAX))?;
        }
        if let Some(tip) = &state.info_tip {
            validate_notify_icon_info_flags(tip.info_flags)?;
            dst.write_u32(tip.timeout);
            dst.write_u32(tip.info_flags);
            write_unicode_string(dst, &tip.text, MAX_INFO_TIP_TEXT_SIZE)?;
            write_unicode_string(dst, &tip.title, MAX_INFO_TIP_TITLE_SIZE)?;
        }
        if let Some(value) = state.state {
            validate_notify_icon_state(value)?;
            dst.write_u32(value);
        }
        if let Some(icon) = &state.icon {
            icon.encode(dst)?;
        }
        Ok(())
    }

    fn decode_body(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        const ALLOWED: u32 = NOTIFY_ICON_TYPE
            | FIELD_NOTIFY_TIP
            | FIELD_NOTIFY_INFO_TIP
            | FIELD_NOTIFY_STATE
            | FIELD_NOTIFY_VERSION
            | STATE_NEW
            | DELETED
            | ICON
            | CACHED_ICON;
        if flags & !ALLOWED != 0 {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "unknown notification icon order flag"
            ));
        }
        ensure_size!(in: src, size: 8 /* WindowId, NotifyIconId */);
        let window_id = src.read_u32();
        let notify_icon_id = src.read_u32();
        let is_new = flags & STATE_NEW != 0;
        if flags & DELETED != 0 {
            if flags != NOTIFY_ICON_TYPE | DELETED {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "deleted notification icon contains additional fields"
                ));
            }
            return Ok(Self {
                window_id,
                notify_icon_id,
                is_new,
                state: None,
            });
        }
        if flags & (ICON | CACHED_ICON) == ICON | CACHED_ICON {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "notification icon contains both icon representations"
            ));
        }
        if is_new && flags & (ICON | CACHED_ICON) == 0 {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "new notification icon has no icon representation"
            ));
        }

        let version = read_if(
            flags & FIELD_NOTIFY_VERSION != 0,
            src,
            4, /* Version */
            ReadCursor::read_u32,
        )?;
        if let Some(version) = version {
            validate_notify_icon_version(version)?;
        }
        let tooltip = (flags & FIELD_NOTIFY_TIP != 0)
            .then(|| read_unicode_string(src, usize::from(u16::MAX)))
            .transpose()?;
        let info_tip = if flags & FIELD_NOTIFY_INFO_TIP != 0 {
            ensure_size!(in: src, size: 8 /* Timeout, InfoFlags */);
            let timeout = src.read_u32();
            let info_flags = src.read_u32();
            validate_notify_icon_info_flags(info_flags)?;
            Some(NotifyIconInfoTip {
                timeout,
                info_flags,
                text: read_unicode_string(src, MAX_INFO_TIP_TEXT_SIZE)?,
                title: read_unicode_string(src, MAX_INFO_TIP_TITLE_SIZE)?,
            })
        } else {
            None
        };
        let state = read_if(
            flags & FIELD_NOTIFY_STATE != 0,
            src,
            4, /* State */
            ReadCursor::read_u32,
        )?;
        if let Some(state) = state {
            validate_notify_icon_state(state)?;
        }
        let icon = if flags & ICON != 0 {
            Some(IconSource::Icon(IconInfo::decode(src)?))
        } else if flags & CACHED_ICON != 0 {
            Some(IconSource::Cached(CachedIconInfo::decode(src)?))
        } else {
            None
        };

        Ok(Self {
            window_id,
            notify_icon_id,
            is_new,
            state: Some(Box::new(NotifyIconState {
                version,
                tooltip,
                info_tip,
                state,
                icon,
            })),
        })
    }
}

/// A Desktop Information order ([MS-RDPERP] 2.2.1.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopOrder {
    /// A Non-Monitored Desktop order ([MS-RDPERP] 2.2.1.3.3.2.2): the server stopped
    /// monitoring the desktop, so the client discards every window and icon.
    NonMonitored,
    /// An Actively Monitored Desktop order ([MS-RDPERP] 2.2.1.3.3.2.1).
    ActivelyMonitored(MonitoredDesktop),
}

/// The fields of an Actively Monitored Desktop order ([MS-RDPERP] 2.2.1.3.3.2.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonitoredDesktop {
    /// `WINDOW_ORDER_FIELD_DESKTOP_HOOKED`: the server started monitoring the desktop.
    pub hooked: bool,
    /// `WINDOW_ORDER_FIELD_DESKTOP_ARC_BEGAN`: the server is resending the desktop state
    /// after an auto-reconnect, so the client discards its windows first.
    pub arc_began: bool,
    /// `WINDOW_ORDER_FIELD_DESKTOP_ARC_COMPLETED`: the auto-reconnect resend finished.
    pub arc_completed: bool,
    /// The active window, or `0xFFFFFFFF` when no RAIL window is active.
    pub active_window_id: Option<u32>,
    /// The window z-order, topmost first.
    pub z_order: Option<Vec<u32>>,
}

impl MonitoredDesktop {
    /// The `ActiveWindowId` value that means no RAIL window is active.
    pub const NO_ACTIVE_WINDOW: u32 = 0xFFFF_FFFF;
}

impl DesktopOrder {
    /// Returns whether the client discards its windows and notification icons
    /// before applying this order.
    pub fn resets_windows(&self) -> bool {
        match self {
            Self::NonMonitored => true,
            Self::ActivelyMonitored(desktop) => desktop.hooked && desktop.arc_began,
        }
    }

    fn fields_present(&self) -> u32 {
        DESKTOP_TYPE
            | match self {
                Self::NonMonitored => FIELD_DESKTOP_NONE,
                Self::ActivelyMonitored(desktop) => {
                    let mut flags = 0;
                    if desktop.hooked {
                        flags |= FIELD_DESKTOP_HOOKED;
                    }
                    if desktop.arc_began {
                        flags |= FIELD_DESKTOP_ARC_BEGAN;
                    }
                    if desktop.arc_completed {
                        flags |= FIELD_DESKTOP_ARC_COMPLETED;
                    }
                    if desktop.active_window_id.is_some() {
                        flags |= FIELD_DESKTOP_ACTIVE_WND;
                    }
                    if desktop.z_order.is_some() {
                        flags |= FIELD_DESKTOP_ZORDER;
                    }
                    flags
                }
            }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::NonMonitored => 0,
            Self::ActivelyMonitored(desktop) => {
                desktop.active_window_id.map_or(0, |_| 4 /* ActiveWindowId */)
                    + desktop
                        .z_order
                        .as_ref()
                        .map_or(0, |ids| 1 /* NumWindowIds */ + ids.len() * 4 /* WindowIds */)
            }
        }
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let Self::ActivelyMonitored(desktop) = self else {
            return Ok(());
        };
        Self::validate_flags(self.fields_present())?;
        if let Some(active_window_id) = desktop.active_window_id {
            dst.write_u32(active_window_id);
        }
        if let Some(ids) = &desktop.z_order {
            let count: u8 = cast_length!("NumWindowIds", ids.len())?;
            dst.write_u8(count);
            for id in ids {
                dst.write_u32(*id);
            }
        }
        Ok(())
    }

    fn validate_flags<E: InvalidFieldErr>(flags: u32) -> Result<(), E> {
        if flags & FIELD_DESKTOP_ARC_BEGAN != 0 && flags & FIELD_DESKTOP_HOOKED == 0 {
            return Err(invalid_field_err!("fieldsPresent", "desktop ARC began requires hooked"));
        }
        if flags & FIELD_DESKTOP_ARC_COMPLETED != 0 && flags != DESKTOP_TYPE | FIELD_DESKTOP_ARC_COMPLETED {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "desktop ARC completed contains additional fields"
            ));
        }
        Ok(())
    }

    fn decode_body(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        const ALLOWED: u32 = DESKTOP_TYPE
            | FIELD_DESKTOP_NONE
            | FIELD_DESKTOP_HOOKED
            | FIELD_DESKTOP_ARC_COMPLETED
            | FIELD_DESKTOP_ARC_BEGAN
            | FIELD_DESKTOP_ZORDER
            | FIELD_DESKTOP_ACTIVE_WND;
        if flags & !ALLOWED != 0 {
            return Err(invalid_field_err!("fieldsPresent", "unknown desktop order flag"));
        }
        if flags & FIELD_DESKTOP_NONE != 0 {
            if flags != DESKTOP_TYPE | FIELD_DESKTOP_NONE {
                return Err(invalid_field_err!(
                    "fieldsPresent",
                    "non-monitored desktop contains additional fields"
                ));
            }
            return Ok(Self::NonMonitored);
        }
        Self::validate_flags(flags)?;

        let active_window_id = read_if(
            flags & FIELD_DESKTOP_ACTIVE_WND != 0,
            src,
            4, /* ActiveWindowId */
            ReadCursor::read_u32,
        )?;
        let z_order = if flags & FIELD_DESKTOP_ZORDER != 0 {
            ensure_size!(in: src, size: 1 /* NumWindowIds */);
            let count = usize::from(src.read_u8());
            ensure_size!(in: src, size: count * 4 /* WindowIds */);
            Some(core::iter::repeat_with(|| src.read_u32()).take(count).collect())
        } else {
            None
        };

        Ok(Self::ActivelyMonitored(MonitoredDesktop {
            hooked: flags & FIELD_DESKTOP_HOOKED != 0,
            arc_began: flags & FIELD_DESKTOP_ARC_BEGAN != 0,
            arc_completed: flags & FIELD_DESKTOP_ARC_COMPLETED != 0,
            active_window_id,
            z_order,
        }))
    }
}

fn validate_notify_icon_version<E: InvalidFieldErr>(version: u32) -> Result<(), E> {
    if matches!(version, 0 | 3 | 4) {
        Ok(())
    } else {
        Err(invalid_field_err!("Version", "unsupported notification icon version"))
    }
}

fn validate_notify_icon_info_flags<E: InvalidFieldErr>(info_flags: u32) -> Result<(), E> {
    const KNOWN: u32 = 0x0000_0003 /* icon */ | 0x0000_0010 /* NIIF_NOSOUND */ | 0x0000_0020 /* NIIF_LARGE_ICON */;

    if info_flags & !KNOWN == 0 {
        Ok(())
    } else {
        Err(invalid_field_err!(
            "InfoFlags",
            "invalid notification icon info tip flags"
        ))
    }
}

fn validate_notify_icon_state<E: InvalidFieldErr>(state: u32) -> Result<(), E> {
    if state & !1 /* NIS_HIDDEN */ == 0 {
        Ok(())
    } else {
        Err(invalid_field_err!("State", "unknown notification icon state"))
    }
}

fn read_if<'a, T>(
    present: bool,
    src: &mut ReadCursor<'a>,
    size: usize,
    read: impl FnOnce(&mut ReadCursor<'a>) -> T,
) -> DecodeResult<Option<T>> {
    if !present {
        return Ok(None);
    }
    ensure_size!(in: src, size: size);
    Ok(Some(read(src)))
}

fn read_point(src: &mut ReadCursor<'_>) -> WindowPoint {
    WindowPoint {
        x: src.read_i32(),
        y: src.read_i32(),
    }
}

fn write_point(dst: &mut WriteCursor<'_>, point: WindowPoint) {
    dst.write_i32(point.x);
    dst.write_i32(point.y);
}

fn read_size(src: &mut ReadCursor<'_>) -> WindowSize {
    WindowSize {
        width: src.read_u32(),
        height: src.read_u32(),
    }
}

fn write_size(dst: &mut WriteCursor<'_>, size: WindowSize) {
    dst.write_u32(size.width);
    dst.write_u32(size.height);
}

fn read_margins(src: &mut ReadCursor<'_>) -> ResizeMargins {
    ResizeMargins {
        start: src.read_u32(),
        end: src.read_u32(),
    }
}

fn write_margins(dst: &mut WriteCursor<'_>, margins: ResizeMargins) {
    dst.write_u32(margins.start);
    dst.write_u32(margins.end);
}

fn rectangles_size(rects: &[ExclusiveRectangle]) -> usize {
    2 /* count */ + rects.len() * 8 /* TS_RECTANGLE_16 */
}

fn read_rectangles(src: &mut ReadCursor<'_>) -> DecodeResult<Vec<ExclusiveRectangle>> {
    ensure_size!(in: src, size: 2 /* count */);
    let count = usize::from(src.read_u16());
    ensure_size!(in: src, size: count * 8 /* TS_RECTANGLE_16 */);
    Ok(core::iter::repeat_with(|| ExclusiveRectangle {
        left: src.read_u16(),
        top: src.read_u16(),
        right: src.read_u16(),
        bottom: src.read_u16(),
    })
    .take(count)
    .collect())
}

fn write_rectangles(dst: &mut WriteCursor<'_>, rects: &[ExclusiveRectangle]) -> EncodeResult<()> {
    let count: u16 = cast_length!("count", rects.len())?;
    dst.write_u16(count);
    for rect in rects {
        dst.write_u16(rect.left);
        dst.write_u16(rect.top);
        dst.write_u16(rect.right);
        dst.write_u16(rect.bottom);
    }
    Ok(())
}

fn unicode_string_size(value: &str) -> usize {
    2 /* CbString */ + value.encode_utf16().count() * 2 /* String */
}

fn read_unicode_string(src: &mut ReadCursor<'_>, maximum_size: usize) -> DecodeResult<String> {
    ensure_size!(in: src, size: 2 /* CbString */);
    let size = usize::from(src.read_u16());
    if size % 2 != 0 || size > maximum_size {
        return Err(invalid_field_err!("CbString", "invalid UTF-16 string length"));
    }
    ensure_size!(in: src, size: size);
    let code_units = src
        .read_slice(size)
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&code_units).map_err(|_| invalid_field_err!("String", "invalid UTF-16 string"))
}

fn write_unicode_string(dst: &mut WriteCursor<'_>, value: &str, maximum_size: usize) -> EncodeResult<()> {
    let size = unicode_string_size(value) - 2 /* CbString */;
    if maximum_size < size {
        return Err(invalid_field_err!("CbString", "UTF-16 string is too long"));
    }
    let size: u16 = cast_length!("CbString", size)?;
    dst.write_u16(size);
    for code_unit in value.encode_utf16() {
        dst.write_u16(code_unit);
    }
    Ok(())
}

/// A decoded collection of Windowing Alternate Secondary Drawing Orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowingOrdersUpdate {
    pub orders: Vec<WindowingOrder>,
    /// Whether the source update also contains drawing orders outside the
    /// Windowing Alternate Secondary subset.
    ///
    /// Their framing is stateful, so decoding stops at the first such order
    /// instead of guessing its size.
    pub contains_non_window_orders: bool,
}

/// Returns the size of [`encode_slow_path_windowing_orders_update`]'s output for `orders`.
pub fn slow_path_windowing_orders_update_size(orders: &[WindowingOrder]) -> usize {
    2 /* updateType */
        + 2 /* pad2OctetsA */
        + 2 /* numberOrders */
        + 2 /* pad2OctetsB */
        + orders.iter().map(Encode::size).sum::<usize>()
}

/// Encodes windowing orders as a complete slow-path Orders update: a Slow-Path Graphics
/// Update ([MS-RDPBCGR] 2.2.9.1.1.3.1) of type `UPDATETYPE_ORDERS` whose data is an
/// Orders Update ([MS-RDPEGDI] 2.2.2.1).
///
/// [`try_decode_slow_path_windowing_orders`] decodes what follows the `updateType` field.
/// Orders produced by the decoder always encode. An error means an order was built with
/// a value the decoder would reject, or there are more than 65,535 orders.
///
/// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr
/// [MS-RDPEGDI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegdi
pub fn encode_slow_path_windowing_orders_update(
    orders: &[WindowingOrder],
    dst: &mut WriteCursor<'_>,
) -> EncodeResult<()> {
    const UPDATETYPE_ORDERS: u16 = 0x0000;

    ensure_size!(in: dst, size: slow_path_windowing_orders_update_size(orders));
    let order_count: u16 = cast_length!("numberOrders", orders.len())?;
    dst.write_u16(UPDATETYPE_ORDERS);
    dst.write_u16(0); // pad2OctetsA
    dst.write_u16(order_count);
    dst.write_u16(0); // pad2OctetsB
    for order in orders {
        order.encode(dst)?;
    }
    Ok(())
}

/// Parses a slow-path Orders update after its `updateType` field.
pub fn try_decode_slow_path_windowing_orders(src: &mut ReadCursor<'_>) -> DecodeResult<WindowingOrdersUpdate> {
    ensure_size!(in: src, size: 2 /* pad2OctetsA */ + 2 /* numberOrders */ + 2 /* pad2OctetsB */);
    src.advance(2);
    let order_count = usize::from(src.read_u16());
    src.advance(2);
    decode_orders(src, order_count)
}

/// Parses a Fast-Path Orders update after its Fast-Path framing.
pub fn try_decode_fast_path_windowing_orders(src: &mut ReadCursor<'_>) -> DecodeResult<WindowingOrdersUpdate> {
    ensure_size!(in: src, size: 2 /* numberOrders */);
    let order_count = usize::from(src.read_u16());
    decode_orders(src, order_count)
}

fn decode_orders(src: &mut ReadCursor<'_>, order_count: usize) -> DecodeResult<WindowingOrdersUpdate> {
    let mut orders =
        Vec::with_capacity(order_count.min(src.len() / (ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE)));
    for _ in 0..order_count {
        if src.remaining().first().copied() != Some(WINDOWING_ORDER_CONTROL_FLAGS) {
            return Ok(WindowingOrdersUpdate {
                orders,
                contains_non_window_orders: true,
            });
        }
        orders.push(WindowingOrder::decode(src)?);
    }
    if !src.eof() {
        return Err(invalid_field_err!("orderData", "orders update contains trailing data"));
    }

    Ok(WindowingOrdersUpdate {
        orders,
        contains_non_window_orders: false,
    })
}

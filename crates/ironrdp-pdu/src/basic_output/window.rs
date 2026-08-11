//! Windowing Alternate Secondary Drawing Orders.
//!
//! These orders are carried by slow-path and Fast-Path Orders updates as defined
//! in [MS-RDPERP] section 2.2.1.3.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use ironrdp_core::{DecodeResult, ReadCursor, ensure_size, invalid_field_err};

const WINDOWING_ORDER_CONTROL_FLAGS: u8 = 0x2e;
const ALTERNATE_SECONDARY_HEADER_SIZE: usize = 1 /* controlFlags */ + 2 /* orderSize */;
const ORDER_FLAGS_SIZE: usize = 4 /* fieldsPresent */;
const WINDOW_HEADER_SIZE: usize = ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE + 4 /* windowId */;
const NOTIFY_ICON_HEADER_SIZE: usize =
    ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE + 4 /* windowId */ + 4 /* notifyIconId */;
const DESKTOP_HEADER_SIZE: usize = ALTERNATE_SECONDARY_HEADER_SIZE + ORDER_FLAGS_SIZE;

const WINDOW_TYPE: u32 = 0x0100_0000;
const NOTIFY_ICON_TYPE: u32 = 0x0200_0000;
const DESKTOP_TYPE: u32 = 0x0400_0000;
const ORDER_TYPE_MASK: u32 = WINDOW_TYPE | NOTIFY_ICON_TYPE | DESKTOP_TYPE;

/// A validated Windowing Alternate Secondary Drawing Order.
///
/// The encoded data remains opaque so protocol consumers can apply their own
/// window-management policy without coupling this layer to a presentation API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowingOrder<'a> {
    pub encoded: &'a [u8],
    pub fields_present: u32,
}

macro_rules! skip_if {
    ($src:ident, $flags:expr, $flag:expr, $size:expr) => {
        if $flags & $flag != 0 {
            ensure_size!(in: $src, size: $size);
            $src.advance($size);
        }
    };
}

macro_rules! skip_unicode_if {
    ($src:ident, $flags:expr, $flag:expr, $maximum_size:expr) => {
        if $flags & $flag != 0 {
            skip_unicode($src, $maximum_size)?;
        }
    };
}

macro_rules! skip_rectangles_if {
    ($src:ident, $flags:expr, $flag:expr) => {
        if $flags & $flag != 0 {
            ensure_size!(in: $src, size: 2 /* Rectangle count */);
            let count = usize::from($src.read_u16());
            ensure_size!(in: $src, size: count * 8 /* TS_RECTANGLE_16 */);
            $src.advance(count * 8);
        }
    };
}

impl<'de> WindowingOrder<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: ALTERNATE_SECONDARY_HEADER_SIZE);

        let encoded = src.remaining();
        let control_flags = src.read_u8();
        if control_flags != WINDOWING_ORDER_CONTROL_FLAGS {
            return Err(invalid_field_err!(
                "controlFlags",
                "expected a windowing alternate secondary drawing order"
            ));
        }

        let order_size = usize::from(src.read_u16());
        if order_size < DESKTOP_HEADER_SIZE {
            return Err(invalid_field_err!(
                "orderSize",
                "order is smaller than its common header"
            ));
        }

        ensure_size!(in: src, size: order_size - ALTERNATE_SECONDARY_HEADER_SIZE);
        let order_data = src.read_slice(order_size - ALTERNATE_SECONDARY_HEADER_SIZE);
        let mut order_data = ReadCursor::new(order_data);
        ensure_size!(in: order_data, size: ORDER_FLAGS_SIZE);
        let fields_present = order_data.read_u32();
        let header_size = match fields_present & ORDER_TYPE_MASK {
            WINDOW_TYPE => WINDOW_HEADER_SIZE,
            NOTIFY_ICON_TYPE => NOTIFY_ICON_HEADER_SIZE,
            DESKTOP_TYPE => DESKTOP_HEADER_SIZE,
            _ => return Err(invalid_field_err!("fieldsPresent", "invalid windowing order type")),
        };
        if order_size < header_size {
            return Err(invalid_field_err!(
                "orderSize",
                "order is smaller than its typed header"
            ));
        }
        validate_order_fields(fields_present, &mut order_data)?;
        if !order_data.eof() {
            return Err(invalid_field_err!("orderSize", "order contains trailing data"));
        }

        Ok(Self {
            encoded: &encoded[..order_size],
            fields_present,
        })
    }

    /// Returns whether this order requires extended Window List support.
    pub const fn requires_extended_support(self) -> bool {
        self.fields_present
            & (0x0001_0000 /* CLIENTAREASIZE */ | 0x0002_0000 /* RPCONTENT */ | 0x0004_0000/* ROOTPARENT */)
            != 0
    }
}

fn validate_order_fields(fields_present: u32, src: &mut ReadCursor<'_>) -> DecodeResult<()> {
    match fields_present & ORDER_TYPE_MASK {
        WINDOW_TYPE => validate_window_fields(fields_present, src),
        NOTIFY_ICON_TYPE => validate_notification_icon_fields(fields_present, src),
        DESKTOP_TYPE => validate_desktop_fields(fields_present, src),
        _ => unreachable!("the order type was validated by the caller"),
    }
}

fn validate_window_fields(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<()> {
    const DELETED: u32 = 0x2000_0000;
    const ICON: u32 = 0x4000_0000;
    const CACHED_ICON: u32 = 0x8000_0000;
    const ICON_FLAGS: u32 = 0x0010_0000 | 0x0000_2000;
    const ALLOWED: u32 = WINDOW_TYPE
        | 0x0000_0001 /* APPBAR_EDGE */
        | 0x0000_0002 /* OWNER */
        | 0x0000_0004 /* TITLE */
        | 0x0000_0008 /* STYLE */
        | 0x0000_0010 /* SHOW */
        | 0x0000_0040 /* APPBAR_STATE */
        | 0x0000_0080 /* RESIZE_MARGIN_X */
        | 0x0000_0100 /* WNDRECTS */
        | 0x0000_0200 /* VISIBILITY */
        | 0x0000_0400 /* WNDSIZE */
        | 0x0000_0800 /* WNDOFFSET */
        | 0x0000_1000 /* VISOFFSET */
        | 0x0000_2000 /* ICON_BIG */
        | 0x0000_4000 /* CLIENTAREAOFFSET */
        | 0x0000_8000 /* CLIENTDELTA */
        | 0x0001_0000 /* CLIENTAREASIZE */
        | 0x0002_0000 /* RPCONTENT */
        | 0x0004_0000 /* ROOTPARENT */
        | 0x0008_0000 /* ENFORCE_SERVER_ZORDER */
        | 0x0010_0000 /* ICON_OVERLAY */
        | 0x0020_0000 /* ICON_OVERLAY_NULL */
        | 0x0040_0000 /* OVERLAY_DESCRIPTION */
        | 0x0080_0000 /* TASKBAR_BUTTON */
        | 0x0800_0000 /* RESIZE_MARGIN_Y */
        | 0x1000_0000 /* STATE_NEW */
        | DELETED
        | ICON
        | CACHED_ICON;

    if flags & !ALLOWED != 0 {
        return Err(invalid_field_err!("fieldsPresent", "unknown window order flag"));
    }
    ensure_size!(in: src, size: 4 /* WindowId */);
    src.advance(4);
    if flags & DELETED != 0 {
        if flags != WINDOW_TYPE | DELETED {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "deleted window order contains additional fields"
            ));
        }
        return Ok(());
    }
    if flags & (ICON | CACHED_ICON) != 0 {
        if flags & (ICON | CACHED_ICON) == ICON | CACHED_ICON {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "window order contains both icon representations"
            ));
        }
        if flags & !(WINDOW_TYPE | 0x1000_0000 | ICON | CACHED_ICON | ICON_FLAGS) != 0 {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "invalid flags for a window icon order"
            ));
        }
        return if flags & ICON != 0 {
            validate_icon(src)
        } else {
            ensure_size!(in: src, size: 3 /* CacheEntry, CacheId */);
            src.advance(3);
            Ok(())
        };
    }

    skip_if!(src, flags, 0x0000_0002, 4 /* OwnerWindowId */);
    skip_if!(src, flags, 0x0000_0008, 8 /* Style, ExtendedStyle */);
    if flags & 0x0000_0010 != 0 {
        ensure_size!(in: src, size: 1 /* ShowState */);
        if !matches!(src.read_u8(), 0 | 2 | 3 | 5) {
            return Err(invalid_field_err!("ShowState", "invalid window show state"));
        }
    }
    skip_unicode_if!(src, flags, 0x0000_0004, 520);
    skip_if!(src, flags, 0x0000_4000, 8 /* ClientOffset */);
    skip_if!(src, flags, 0x0001_0000, 8 /* ClientAreaSize */);
    skip_if!(src, flags, 0x0000_0080, 8 /* HorizontalResizeMargins */);
    skip_if!(src, flags, 0x0800_0000, 8 /* VerticalResizeMargins */);
    if flags & 0x0002_0000 != 0 {
        ensure_size!(in: src, size: 1 /* RPContent */);
        if src.read_u8() > 1 {
            return Err(invalid_field_err!("RPContent", "invalid render plugin content"));
        }
    }
    skip_if!(src, flags, 0x0004_0000, 4 /* RootParentHandle */);
    skip_if!(src, flags, 0x0000_0800, 8 /* WindowOffset */);
    skip_if!(src, flags, 0x0000_8000, 8 /* ClientDelta */);
    skip_if!(src, flags, 0x0000_0400, 8 /* WindowSize */);
    skip_rectangles_if!(src, flags, 0x0000_0100);
    skip_if!(src, flags, 0x0000_1000, 8 /* VisibleOffset */);
    skip_rectangles_if!(src, flags, 0x0000_0200);
    skip_unicode_if!(src, flags, 0x0040_0000, usize::from(u16::MAX));
    skip_if!(src, flags, 0x0080_0000, 1 /* TaskbarButton */);
    skip_if!(src, flags, 0x0008_0000, 1 /* EnforceServerZOrder */);
    skip_if!(src, flags, 0x0000_0040, 1 /* AppBarState */);
    skip_if!(src, flags, 0x0000_0001, 1 /* AppBarEdge */);
    Ok(())
}

fn validate_notification_icon_fields(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<()> {
    const DELETED: u32 = 0x2000_0000;
    const ICON: u32 = 0x4000_0000;
    const CACHED_ICON: u32 = 0x8000_0000;
    const ALLOWED: u32 = NOTIFY_ICON_TYPE | 0x0000_000F | 0x1000_0000 | DELETED | ICON | CACHED_ICON;
    if flags & !ALLOWED != 0 {
        return Err(invalid_field_err!(
            "fieldsPresent",
            "unknown notification icon order flag"
        ));
    }
    ensure_size!(in: src, size: 8 /* WindowId, NotifyIconId */);
    src.advance(8);
    if flags & DELETED != 0 {
        if flags != NOTIFY_ICON_TYPE | DELETED {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "deleted notification icon contains additional fields"
            ));
        }
        return Ok(());
    }
    if flags & (ICON | CACHED_ICON) == ICON | CACHED_ICON {
        return Err(invalid_field_err!(
            "fieldsPresent",
            "notification icon contains both icon representations"
        ));
    }
    if flags & 0x1000_0000 != 0 && flags & (ICON | CACHED_ICON) == 0 {
        return Err(invalid_field_err!(
            "fieldsPresent",
            "new notification icon has no icon representation"
        ));
    }
    if flags & 0x0000_0008 != 0 {
        ensure_size!(in: src, size: 4 /* Version */);
        if !matches!(src.read_u32(), 0 | 3 | 4) {
            return Err(invalid_field_err!("Version", "unsupported notification icon version"));
        }
    }
    skip_unicode_if!(src, flags, 0x0000_0001, usize::from(u16::MAX));
    if flags & 0x0000_0002 != 0 {
        ensure_size!(in: src, size: 8 /* Timeout, InfoFlags */);
        src.advance(4);
        let info_flags = src.read_u32();
        if info_flags & !0x33 != 0 || !matches!(info_flags & 0x0F, 0..=3) {
            return Err(invalid_field_err!(
                "InfoFlags",
                "invalid notification icon info tip flags"
            ));
        }
        skip_unicode(src, 510)?;
        skip_unicode(src, 126)?;
    }
    if flags & 0x0000_0004 != 0 {
        ensure_size!(in: src, size: 4 /* State */);
        if src.read_u32() & !1 != 0 {
            return Err(invalid_field_err!("State", "unknown notification icon state"));
        }
    }
    if flags & ICON != 0 {
        validate_icon(src)?;
    } else if flags & CACHED_ICON != 0 {
        ensure_size!(in: src, size: 3 /* CacheEntry, CacheId */);
        src.advance(3);
    }
    Ok(())
}

fn validate_desktop_fields(flags: u32, src: &mut ReadCursor<'_>) -> DecodeResult<()> {
    const ALLOWED: u32 = DESKTOP_TYPE | 0x3F;
    if flags & !ALLOWED != 0 {
        return Err(invalid_field_err!("fieldsPresent", "unknown desktop order flag"));
    }
    if flags & 1 != 0 {
        if flags != DESKTOP_TYPE | 1 {
            return Err(invalid_field_err!(
                "fieldsPresent",
                "non-monitored desktop contains additional fields"
            ));
        }
        return Ok(());
    }
    if flags & 8 != 0 && flags & 2 == 0 {
        return Err(invalid_field_err!("fieldsPresent", "desktop ARC began requires hooked"));
    }
    if flags & 4 != 0 && flags != DESKTOP_TYPE | 4 {
        return Err(invalid_field_err!(
            "fieldsPresent",
            "desktop ARC completed contains additional fields"
        ));
    }
    skip_if!(src, flags, 0x20, 4 /* ActiveWindowId */);
    if flags & 0x10 != 0 {
        ensure_size!(in: src, size: 1 /* NumWindowIds */);
        let count = usize::from(src.read_u8());
        ensure_size!(in: src, size: count * 4 /* WindowIds */);
        src.advance(count * 4);
    }
    Ok(())
}

fn validate_icon(src: &mut ReadCursor<'_>) -> DecodeResult<()> {
    ensure_size!(in: src, size: 8 /* fixed icon fields */);
    src.advance(3);
    let bpp = src.read_u8();
    if !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(invalid_field_err!("Bpp", "unsupported icon color depth"));
    }
    src.advance(4);
    let color_table_len = if matches!(bpp, 1 | 4 | 8) {
        ensure_size!(in: src, size: 2 /* CbColorTable */);
        usize::from(src.read_u16())
    } else {
        0
    };
    ensure_size!(in: src, size: 4 /* CbBitsMask, CbBitsColor */);
    let mask_len = usize::from(src.read_u16());
    let color_len = usize::from(src.read_u16());
    ensure_size!(in: src, size: color_table_len + mask_len + color_len);
    src.advance(color_table_len + mask_len + color_len);
    Ok(())
}

fn skip_unicode(src: &mut ReadCursor<'_>, maximum_size: usize) -> DecodeResult<()> {
    ensure_size!(in: src, size: 2 /* CbString */);
    let size = usize::from(src.read_u16());
    if size % 2 != 0 || size > maximum_size {
        return Err(invalid_field_err!("CbString", "invalid UTF-16 string length"));
    }
    ensure_size!(in: src, size: size);
    let bytes = src.read_slice(size);
    let code_units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&code_units).map_err(|_| invalid_field_err!("String", "invalid UTF-16 string"))?;
    Ok(())
}

/// A validated collection of Windowing Alternate Secondary Drawing Orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowingOrdersUpdate<'a> {
    pub orders: Vec<WindowingOrder<'a>>,
    /// Whether the source update also contains drawing orders outside the
    /// Windowing Alternate Secondary subset.
    ///
    /// Their framing is stateful, so they remain in the encoded update for the
    /// consumer instead of being discarded while scanning for window orders.
    pub contains_non_window_orders: bool,
}

/// Parses a slow-path Orders update after its `updateType` field.
///
pub fn try_decode_slow_path_windowing_orders<'a>(src: &mut ReadCursor<'a>) -> DecodeResult<WindowingOrdersUpdate<'a>> {
    ensure_size!(in: src, size: 2 /* pad2OctetsA */ + 2 /* numberOrders */ + 2 /* pad2OctetsB */);
    src.advance(2);
    let order_count = usize::from(src.read_u16());
    src.advance(2);
    decode_orders(src, order_count)
}

/// Parses a Fast-Path Orders update after its Fast-Path framing.
///
pub fn try_decode_fast_path_windowing_orders<'a>(src: &mut ReadCursor<'a>) -> DecodeResult<WindowingOrdersUpdate<'a>> {
    ensure_size!(in: src, size: 2 /* numberOrders */);
    let order_count = usize::from(src.read_u16());
    decode_orders(src, order_count)
}

fn decode_orders<'a>(src: &mut ReadCursor<'a>, order_count: usize) -> DecodeResult<WindowingOrdersUpdate<'a>> {
    let mut orders = Vec::with_capacity(order_count);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn deleted_window_order() -> [u8; 11] {
        [WINDOWING_ORDER_CONTROL_FLAGS, 11, 0, 0, 0, 0, 0x21, 7, 0, 0, 0]
    }

    #[test]
    fn parses_slow_path_windowing_order() {
        let order = deleted_window_order();
        let mut update = vec![0, 0, 1, 0, 0, 0];
        update.extend_from_slice(&order);

        let orders = try_decode_slow_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap();
        assert_eq!(orders.orders.len(), 1);
        assert_eq!(orders.orders[0].encoded, order.as_slice());
        assert!(!orders.orders[0].requires_extended_support());
    }

    #[test]
    fn parses_fast_path_windowing_order() {
        let order = deleted_window_order();
        let mut update = vec![1, 0];
        update.extend_from_slice(&order);

        let orders = try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap();
        assert_eq!(orders.orders[0].encoded, order.as_slice());
    }

    #[test]
    fn rejects_malformed_windowing_order() {
        let mut malformed = deleted_window_order();
        malformed[1] = 12;
        let mut update = vec![1, 0];
        update.extend_from_slice(&malformed);

        assert!(try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).is_err());
    }

    #[test]
    fn rejects_window_order_with_truncated_optional_field() {
        let mut order = deleted_window_order();
        order[3..7].copy_from_slice(&(WINDOW_TYPE | 0x0000_0004/* TITLE */).to_le_bytes());
        let mut update = vec![1, 0];
        update.extend_from_slice(&order);

        assert!(try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).is_err());
    }

    #[test]
    fn preserves_mixed_order_updates_for_consumers() {
        let mut update = vec![2, 0];
        update.extend_from_slice(&deleted_window_order());
        update.push(0);

        let orders = try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap();
        assert_eq!(orders.orders.len(), 1);
        assert!(orders.contains_non_window_orders);
    }
}

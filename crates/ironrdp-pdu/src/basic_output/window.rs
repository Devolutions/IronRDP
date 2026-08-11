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

/// A validated collection of Windowing Alternate Secondary Drawing Orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowingOrdersUpdate<'a> {
    pub orders: Vec<WindowingOrder<'a>>,
}

/// Parses a slow-path Orders update after its `updateType` field.
///
/// Returns `None` when the update contains a non-window order. Malformed
/// window orders return an error rather than forwarding a partial prefix.
pub fn try_decode_slow_path_windowing_orders<'a>(
    src: &mut ReadCursor<'a>,
) -> DecodeResult<Option<WindowingOrdersUpdate<'a>>> {
    ensure_size!(in: src, size: 2 /* pad2OctetsA */ + 2 /* numberOrders */ + 2 /* pad2OctetsB */);
    src.advance(2);
    let order_count = usize::from(src.read_u16());
    src.advance(2);
    decode_orders(src, order_count)
}

/// Parses a Fast-Path Orders update after its Fast-Path framing.
///
/// Returns `None` when the update contains a non-window order. Malformed
/// window orders return an error rather than forwarding a partial prefix.
pub fn try_decode_fast_path_windowing_orders<'a>(
    src: &mut ReadCursor<'a>,
) -> DecodeResult<Option<WindowingOrdersUpdate<'a>>> {
    ensure_size!(in: src, size: 2 /* numberOrders */);
    let order_count = usize::from(src.read_u16());
    decode_orders(src, order_count)
}

fn decode_orders<'a>(src: &mut ReadCursor<'a>, order_count: usize) -> DecodeResult<Option<WindowingOrdersUpdate<'a>>> {
    let mut orders = Vec::with_capacity(order_count);
    for _ in 0..order_count {
        if src.remaining().first().copied() != Some(WINDOWING_ORDER_CONTROL_FLAGS) {
            return Ok(None);
        }
        orders.push(WindowingOrder::decode(src)?);
    }
    if !src.eof() {
        return Err(invalid_field_err!("orderData", "orders update contains trailing data"));
    }

    Ok(Some(WindowingOrdersUpdate { orders }))
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

        let orders = try_decode_slow_path_windowing_orders(&mut ReadCursor::new(&update))
            .unwrap()
            .unwrap();
        assert_eq!(orders.orders.len(), 1);
        assert_eq!(orders.orders[0].encoded, order.as_slice());
        assert!(!orders.orders[0].requires_extended_support());
    }

    #[test]
    fn parses_fast_path_windowing_order() {
        let order = deleted_window_order();
        let mut update = vec![1, 0];
        update.extend_from_slice(&order);

        let orders = try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update))
            .unwrap()
            .unwrap();
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
    fn skips_non_window_orders_without_partial_forwarding() {
        let mut update = vec![2, 0];
        update.extend_from_slice(&deleted_window_order());
        update.push(0);

        assert_eq!(
            try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap(),
            None
        );
    }
}

#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub const CHANNEL_NAME: &str = "URBDRC";

pub mod client;
pub mod pdu;

/// Error returned when a per-device USB interface ID conflicts with an RDPEUSB default interface.
///
/// RDPEUSB reserves interface IDs `0x0..=0x3` for the built-in Capabilities, Device Sink, and
/// Channel Notification interfaces. A USB Device interface advertised in `ADD_DEVICE` must use a
/// dynamically allocated ID outside that range.
///
/// The inner value is retained so callers can recover ownership and retry with a different ID.
pub struct InvalidDeviceInterfaceId<T> {
    inner: T,
}

impl<T> InvalidDeviceInterfaceId<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> core::fmt::Debug for InvalidDeviceInterfaceId<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InvalidDeviceInterfaceId").finish_non_exhaustive()
    }
}

impl<T> core::error::Error for InvalidDeviceInterfaceId<T> {}

impl<T> core::fmt::Display for InvalidDeviceInterfaceId<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("USB device interface id conflicts with RDPEUSB default interfaces; expected id >= 0x00000004")
    }
}

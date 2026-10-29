//! Virtual desktop control channel.
//!
//! Windows 365 Cloud PCs use this channel in Boot and Switch mode to send the user back to the local desktop and to
//! open local Windows Settings pages, such as Bluetooth pairing, on the physical device.
//! The client negotiates the protocol version and passes server requests to a caller-supplied
//! [`VdcSwitchHandler`](client::VdcSwitchHandler).

/// Dynamic virtual channel name Windows 365 Cloud PCs open for Boot and Switch mode.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::VirtualDesktopControl::Switch";

pub mod client;
pub mod pdu;

#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![allow(clippy::arithmetic_side_effects)] // TODO: should we enable this lint back?

mod macros;

pub mod autodetect;
mod builder;
mod capabilities;
mod clipboard;
mod display;
mod echo;
mod encoder;
mod error;
#[cfg(feature = "egfx")]
mod gfx;
mod handler;
#[cfg(feature = "helper")]
mod helper;
mod rdpdr;
mod rdpei;
mod server;
mod sound;
#[cfg(feature = "usb")]
mod urbdrc;

pub use clipboard::CliprdrServerFactory;
pub use display::{
    BitmapUpdate, ColorPointer, DesktopSize, DisplayUpdate, Framebuffer, LargePointer, PixelFormat, RGBAPointer,
    RdpServerDisplay, RdpServerDisplayUpdates,
};
pub use echo::{EchoDvcBridge, EchoRoundTripMeasurement, EchoServerHandle, EchoServerMessage};
pub use error::{ServerError, ServerErrorExt, ServerErrorKind, ServerResult, ServerResultExt};
#[cfg(feature = "egfx")]
pub use gfx::{EgfxServerMessage, GfxDvcBridge, GfxServerFactory, GfxServerHandle};
pub use handler::{KeyboardEvent, MouseButton, MouseEvent, RdpServerInputHandler};
#[cfg(feature = "helper")]
pub use helper::TlsIdentityCtx;
pub use ironrdp_acceptor::Acceptor;
pub use ironrdp_pdu::rdp::server_error_info::ErrorInfo;
pub use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
#[cfg(feature = "usb")]
pub use ironrdp_rdpeusb::io::{CompletionData, DeviceAnnounce, DeviceText, InternalIoControlPacket};
pub use rdpdr::{NoopRdpdrServerBackend, RdpdrServerBackend, RdpdrServerFactory, RdpdrServerMessage};
pub use rdpei::{
    CsReadyFlags, CsReadyPdu, DismissHoveringTouchContactPdu, PenContact, PenContactDataFlags, PenContactFields,
    PenContactFlags, PenEventPdu, PenFlags, PenFrame, RdpInputProtocolVersion, RdpeiHandler, RdpeiServer,
    RdpeiServerFactory, ScReadyFeatures, TouchContact, TouchContactDataFlags, TouchContactFields, TouchContactFlags,
    TouchEventPdu, TouchFrame,
};
pub use server::{
    AutoReconnectCookieHandle, ConnectionHandler, ConnectionInfo, CredentialDecision, CredentialValidationError,
    CredentialValidator, Credentials, ErrorInfoDisconnectHandle, ExactMatchCredentialValidator, PostConnectionAction,
    RdpServer, RdpServerOptions, RdpServerSecurity, ServerEvent, ServerEventSender, StaticChannelFactory, TransportTls,
    pick_remotefx_entropy_coder,
};
pub use sound::{RdpsndServerHandler, RdpsndServerMessage, SoundServerFactory};
#[cfg(feature = "usb")]
pub use urbdrc::{
    CompletionFut, DeviceFactory, PendingHandle, PendingRequest, RawPending, RdpUsbDeviceAnnounceInfo, UsbDeviceHandle,
    UsbRedirDevice, UsbRequestCompletion,
};
#[cfg(feature = "__bench")]
pub mod bench {
    pub mod encoder {
        pub mod rfx {
            pub use crate::encoder::rfx::bench::{rfx_enc, rfx_enc_tile};
        }

        pub use crate::encoder::{UpdateEncoder, UpdateEncoderCodecs};
    }
}

#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

/// Channel name from [2.1] Transport.
///
/// [2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegt/
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Geometry::v08.01";

pub mod client;
pub mod pdu;

pub use client::{GeometryClient, GeometryEvent, GeometryObserver};
pub use pdu::{GeometryRect, MappedGeometry, MappedGeometryPacket, RegionData};

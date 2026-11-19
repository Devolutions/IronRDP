#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
// The `cpal` example's dev-dependencies are unused by the normal library build,
// so the suppression must also apply when `cfg(test)` is not set.
#![allow(unused_crate_dependencies)]

#[cfg(all(windows, feature = "aac"))]
mod aac;
#[cfg(feature = "capture")]
pub mod capture;
pub mod cpal;
pub mod device;
pub mod error;
#[cfg(windows)]
mod session_volume;

#[cfg(feature = "capture")]
pub use capture::{CaptureStopSignal, RdpeaiCaptureBackend, is_pcm_capture_format, take_capture_packets};
pub use device::{AudioDeviceInfo, DeviceSelector, Direction, list_devices};
pub use error::{RdpsndNativeError, RdpsndNativeErrorKind, RdpsndNativeResult};

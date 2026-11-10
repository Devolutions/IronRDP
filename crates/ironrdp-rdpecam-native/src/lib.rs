#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[cfg(windows)]
mod media_foundation;

#[cfg(windows)]
pub use media_foundation::{
    CameraDeviceInfo, CameraError, CameraEvent, CameraOptions, CameraWatcher, EncodingQuality, MediaFoundationCamera,
    enumerate_cameras,
};

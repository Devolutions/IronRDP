#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::WindowsInput;

#[cfg(all(windows, feature = "__test"))]
#[doc(hidden)]
pub mod __test {
    use ironrdp_rdpei::pdu::{CsReadyFlags, TouchContactFlags};

    pub fn frame_timestamps(
        frequency: u64,
        now: u64,
        previous: u64,
        flags: CsReadyFlags,
        encode_time: u32,
        offsets: &[u64],
    ) -> anyhow::Result<Vec<u64>> {
        crate::windows::frame_timestamps(frequency, now, previous, flags, encode_time, offsets.iter().copied())
    }

    pub fn touch_orientation(orientation: u32) -> anyhow::Result<u32> {
        crate::windows::touch_orientation(orientation)
    }

    pub fn pointer_flags(flags: TouchContactFlags) -> u32 {
        crate::windows::pointer_flags(flags).0
    }
}

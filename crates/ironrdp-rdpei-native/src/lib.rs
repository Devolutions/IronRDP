#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[cfg(windows)]
mod error;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use error::{WindowsInputError, WindowsInputErrorKind, WindowsInputResult};

#[cfg(windows)]
pub use windows::WindowsInput;

#[cfg(all(windows, feature = "__test"))]
#[doc(hidden)]
pub mod __test {
    use ironrdp_rdpei::pdu::{CsReadyFlags, PenContact, PenFlags, TouchContactFlags};
    use windows::Win32::UI::Input::Pointer::POINTER_PEN_INFO;

    pub fn frame_timestamps(
        frequency: u64,
        now: u64,
        previous: u64,
        continuing: bool,
        flags: CsReadyFlags,
        encode_time: u32,
        offsets: &[u64],
    ) -> crate::WindowsInputResult<Vec<u64>> {
        let mut offsets = offsets.iter().copied();
        crate::windows::frame_timestamps(frequency, now, previous, continuing, flags, encode_time, &mut offsets)
    }

    pub fn touch_orientation(orientation: Option<u32>) -> crate::WindowsInputResult<u32> {
        crate::windows::touch_orientation(orientation)
    }

    pub fn touch_contact_rect(
        x: i32,
        y: i32,
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
        orientation: Option<u32>,
    ) -> crate::WindowsInputResult<(i32, i32, i32, i32)> {
        let rect = crate::windows::touch_contact_rect(x, y, left, top, right, bottom, orientation)?;
        Ok((rect.left, rect.top, rect.right, rect.bottom))
    }

    pub fn pen_departures_first(contacts: &mut [PenContact]) {
        crate::windows::prioritize_pen_departures(contacts);
    }

    pub fn pen_flags(flags: PenFlags) -> u32 {
        let mut native = POINTER_PEN_INFO::default();
        crate::windows::set_pen_flags(&mut native, flags);
        native.penFlags
    }

    pub fn pointer_flags(flags: TouchContactFlags) -> u32 {
        crate::windows::pointer_flags(flags).0
    }
}

use crate::{DesktopSize, RdpServerOptions};
use ironrdp_pdu::rdp::capability_sets;

pub fn capabilities(_opts: &RdpServerOptions, size: DesktopSize) -> Vec<capability_sets::CapabilitySet> {
    vec![
        capability_sets::CapabilitySet::General(general_capabilities()),
        capability_sets::CapabilitySet::Bitmap(bitmap_capabilities(&size)),
        capability_sets::CapabilitySet::Order(order_capabilities()),
        capability_sets::CapabilitySet::Pointer(pointer_capabilities()),
        capability_sets::CapabilitySet::Input(input_capabilities()),
        capability_sets::CapabilitySet::VirtualChannel(virtual_channel_capabilities()),
        capability_sets::CapabilitySet::MultiFragmentUpdate(multifragment_update()),
    ]
}

fn general_capabilities() -> capability_sets::General {
    capability_sets::General {
        major_platform_type: capability_sets::MajorPlatformType::Unspecified,
        minor_platform_type: capability_sets::MinorPlatformType::Unspecified,
        extra_flags: capability_sets::GeneralExtraFlags::empty(),
        refresh_rect_support: false,
        suppress_output_support: false,
    }
}

fn bitmap_capabilities(size: &DesktopSize) -> capability_sets::Bitmap {
    capability_sets::Bitmap {
        pref_bits_per_pix: 32,
        desktop_width: size.width,
        desktop_height: size.height,
        desktop_resize_flag: false,
        drawing_flags: capability_sets::BitmapDrawingFlags::empty(),
    }
}

fn order_capabilities() -> capability_sets::Order {
    capability_sets::Order::new(
        capability_sets::OrderFlags::empty(),
        capability_sets::OrderSupportExFlags::empty(),
        2048,
        224,
    )
}

fn pointer_capabilities() -> capability_sets::Pointer {
    capability_sets::Pointer {
        color_pointer_cache_size: 2048,
        pointer_cache_size: 2048,
    }
}

fn input_capabilities() -> capability_sets::Input {
    capability_sets::Input {
        input_flags: capability_sets::InputFlags::SCANCODES
            | capability_sets::InputFlags::MOUSEX
            | capability_sets::InputFlags::FASTPATH_INPUT
            | capability_sets::InputFlags::UNICODE
            | capability_sets::InputFlags::FASTPATH_INPUT_2,
        keyboard_layout: 0,
        keyboard_type: None,
        keyboard_subtype: 0,
        keyboard_function_key: 128,
        keyboard_ime_filename: "".into(),
    }
}

fn virtual_channel_capabilities() -> capability_sets::VirtualChannel {
    capability_sets::VirtualChannel {
        flags: capability_sets::VirtualChannelFlags::NO_COMPRESSION,
        chunk_size: None,
    }
}

fn multifragment_update() -> capability_sets::MultifragmentUpdate {
    capability_sets::MultifragmentUpdate {
        max_request_size: u32::MAX,
    }
}

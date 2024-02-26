use ironrdp_pdu::rdp::capability_sets::{self, GeneralExtraFlags};

use crate::{DesktopSize, RdpServerOptions};

pub(crate) fn capabilities(_opts: &RdpServerOptions, size: DesktopSize) -> Vec<capability_sets::CapabilitySet> {
    vec![
        capability_sets::CapabilitySet::General(general_capabilities()),
        capability_sets::CapabilitySet::Bitmap(bitmap_capabilities(&size)),
        capability_sets::CapabilitySet::Order(order_capabilities()),
        capability_sets::CapabilitySet::SurfaceCommands(surface_capabilities()),
        capability_sets::CapabilitySet::Pointer(pointer_capabilities()),
        capability_sets::CapabilitySet::Input(input_capabilities()),
        capability_sets::CapabilitySet::VirtualChannel(virtual_channel_capabilities()),
        capability_sets::CapabilitySet::MultiFragmentUpdate(multifragment_update()),
        capability_sets::CapabilitySet::BitmapCodecs(bitmap_codecs()),
    ]
}

fn general_capabilities() -> capability_sets::General {
    capability_sets::General {
        extra_flags: GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED,
        ..Default::default()
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

fn surface_capabilities() -> capability_sets::SurfaceCommands {
    capability_sets::SurfaceCommands {
        flags: capability_sets::CmdFlags::all(),
    }
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
            | capability_sets::InputFlags::MOUSE_RELATIVE
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
        // FIXME(#318): use an acceptable value for msctc.
        // What is the actual server max size?
        max_request_size: 16_777_215,
    }
}

fn bitmap_codecs() -> capability_sets::BitmapCodecs {
    capability_sets::BitmapCodecs(vec![
        capability_sets::Codec {
            id: 0,
            property: capability_sets::CodecProperty::RemoteFx(capability_sets::RemoteFxContainer::ServerContainer(1)),
        },
        capability_sets::Codec {
            id: 0,
            property: capability_sets::CodecProperty::ImageRemoteFx(
                capability_sets::RemoteFxContainer::ServerContainer(1),
            ),
        },
    ])
}

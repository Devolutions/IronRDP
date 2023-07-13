use ironrdp_connector::DesktopSize;
use ironrdp_pdu::rdp::capability_sets;

use super::RdpServerOptions;

pub fn capabilities(_opts: &RdpServerOptions, size: DesktopSize) -> Vec<capability_sets::CapabilitySet> {
    vec![
        capability_sets::CapabilitySet::General(general_capabilities()),
        capability_sets::CapabilitySet::Bitmap(bitmap_capabilities(&size)),
        capability_sets::CapabilitySet::Order(order_capabilities()),
        capability_sets::CapabilitySet::BitmapCache(bitmap_cache_capabilities()),
        capability_sets::CapabilitySet::BitmapCacheRev2(bitmap_cache_rev2_capabilities()),
        capability_sets::CapabilitySet::Pointer(pointer_capabilities()),
        capability_sets::CapabilitySet::Sound(sound_capabilities()),
        capability_sets::CapabilitySet::Input(input_capabilities()),
        capability_sets::CapabilitySet::Brush(brush_capabilities()),
        capability_sets::CapabilitySet::GlyphCache(glyph_cache_capabilities()),
        capability_sets::CapabilitySet::OffscreenBitmapCache(offscreen_capabilities()),
        capability_sets::CapabilitySet::VirtualChannel(virtual_channel_capabilities()),
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
        pref_bits_per_pix: 8,
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
        224, // TODO: ??
    )
}

fn bitmap_cache_capabilities() -> capability_sets::BitmapCache {
    capability_sets::BitmapCache {
        caches: [capability_sets::CacheEntry {
            entries: 128,
            max_cell_size: 1024,
        }; 3],
    }
}

fn bitmap_cache_rev2_capabilities() -> capability_sets::BitmapCacheRev2 {
    capability_sets::BitmapCacheRev2 {
        cache_flags: capability_sets::CacheFlags::empty(),
        num_cell_caches: 0,
        cache_cell_info: [capability_sets::CellInfo {
            num_entries: 0,
            is_cache_persistent: false,
        }; 5],
    }
}

fn pointer_capabilities() -> capability_sets::Pointer {
    capability_sets::Pointer {
        color_pointer_cache_size: 2048,
        pointer_cache_size: 2048,
    }
}

fn sound_capabilities() -> capability_sets::Sound {
    capability_sets::Sound {
        flags: capability_sets::SoundFlags::empty(),
    }
}

fn input_capabilities() -> capability_sets::Input {
    capability_sets::Input {
        input_flags: capability_sets::InputFlags::empty(),
        keyboard_layout: 0,
        keyboard_type: None,
        keyboard_subtype: 0,
        keyboard_function_key: 128,
        keyboard_ime_filename: "keyboard".into(),
    }
}

fn brush_capabilities() -> capability_sets::Brush {
    capability_sets::Brush {
        support_level: capability_sets::SupportLevel::Default,
    }
}

fn glyph_cache_capabilities() -> capability_sets::GlyphCache {
    capability_sets::GlyphCache {
        glyph_cache: [capability_sets::CacheDefinition {
            entries: 0,
            max_cell_size: 0,
        }; 10],
        frag_cache: capability_sets::CacheDefinition {
            entries: 0,
            max_cell_size: 0,
        },
        glyph_support_level: capability_sets::GlyphSupportLevel::None,
    }
}

fn offscreen_capabilities() -> capability_sets::OffscreenBitmapCache {
    capability_sets::OffscreenBitmapCache {
        is_supported: false,
        cache_size: 0,
        cache_entries: 0,
    }
}

fn virtual_channel_capabilities() -> capability_sets::VirtualChannel {
    capability_sets::VirtualChannel {
        flags: capability_sets::VirtualChannelFlags::NO_COMPRESSION,
        chunk_size: None,
    }
}

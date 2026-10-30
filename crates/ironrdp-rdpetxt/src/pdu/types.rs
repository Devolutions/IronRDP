//! Structures and enumerations shared by several PDUs ([2.2.1] Namespaces).
//!
//! Enumerations and bitmasks the crate does not act on are plain integers; their values are
//! listed in the referenced sections.
//!
//! [2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use ironrdp_core::{DecodeResult, EncodeResult, ReadCursor, WriteCursor, cast_length, invalid_field_err};

use super::{Guid, PduLayout, TextString};
use crate::wire::{Wire, wire_newtype, wire_struct};

wire_struct! {
    /// Details of a key event ([2.2.1.1] KeyEventHostInfo).
    ///
    /// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct KeyEventHostInfo {
        /// `RemoteTextModifierKeyStateFlag` bitmask.
        pub modifier_flags: u16,
        /// `InputEventFlag` bitmask.
        pub event_flags: u16,
        /// `InputEventFlag2` bitmask.
        pub event_flags2: u32,
        pub virtual_key: u16,
        /// UTF-16 code unit the key was translated into.
        pub character: u16,
        pub translation_flags: u16,
        pub device_id: u64,
        pub repeat_count: u16,
        pub scan_code: u16,
        pub is_extended_key: bool,
        pub is_menu_key: bool,
        pub was_key_down: bool,
        pub is_key_released: bool,
        pub timestamp_in_ms: u32,
        pub message_id: u32,
        pub key_event_attributes: KeyEventAttributes,
    }
}

wire_struct! {
    /// Touch keyboard attributes of a key event ([2.2.1.2] KeyEventAttributes).
    ///
    /// [2.2.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct KeyEventAttributes {
        pub id: u16,
        pub touch_x: i16,
        pub touch_y: i16,
    }
}

wire_struct! {
    /// A hotkey registered by a remote application ([2.2.1.4] HotKeyRegistrationData).
    ///
    /// [2.2.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct HotKeyRegistrationData {
        pub process_id: u32,
        pub thread_id: u32,
        /// `ALT` (0x0001), `CONTROL` (0x0002), `SHIFT` (0x0004), and `WIN` (0x0008) bitmask.
        pub modifiers: u16,
        pub virtual_key: u16,
    }
}

wire_struct! {
    /// A clause of a composition string ([2.2.1.5] CompositionClause).
    ///
    /// [2.2.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CompositionClause {
        pub pre_conversion_string: TextString,
        /// Characters of the composition that belong to this clause.
        pub range: EditControlRange,
    }
}

/// An input profile ([2.2.1.6] CoreInputProfile).
///
/// It takes 82 bytes when [`PduLayout::bcp_tag`] is set, and 74 bytes otherwise.
///
/// [2.2.1.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CoreInputProfile {
    pub langid: u16,
    pub clsid: Guid,
    pub guid_profile: Guid,
    pub catid: Guid,
    pub hkl: u32,
    pub klid: u32,
    pub lcid: u32,
    /// `InputProcessor` (1) or `KeyboardLayout` (2).
    pub profile_type: u32,
    pub caps: u32,
    pub flags: u32,
    /// Language identifier convertible to a BCP-47 tag.
    ///
    /// Introduced by version 1, minor bit 0x00000002: it is on the wire only under a
    /// [`PduLayout`] with `bcp_tag` set, and decodes as zero otherwise.
    pub bcp_tag: u64,
}

impl CoreInputProfile {
    /// Size without **bcpTag**.
    const BASE_SIZE: usize = 2 /* langid */ + 3 * 16 /* clsid, guidProfile, catid */ + 6 * 4 /* hkl .. flags */;
}

impl Wire for CoreInputProfile {
    fn wire_size(&self, layout: PduLayout) -> usize {
        Self::BASE_SIZE
            + if layout.bcp_tag {
                8 /* bcpTag */
            } else {
                0
            }
    }

    fn write(&self, dst: &mut WriteCursor<'_>, layout: PduLayout) -> EncodeResult<()> {
        self.langid.write(dst, layout)?;
        self.clsid.write(dst, layout)?;
        self.guid_profile.write(dst, layout)?;
        self.catid.write(dst, layout)?;
        self.hkl.write(dst, layout)?;
        self.klid.write(dst, layout)?;
        self.lcid.write(dst, layout)?;
        self.profile_type.write(dst, layout)?;
        self.caps.write(dst, layout)?;
        self.flags.write(dst, layout)?;
        if layout.bcp_tag {
            self.bcp_tag.write(dst, layout)?;
        }
        Ok(())
    }

    fn read(src: &mut ReadCursor<'_>, layout: PduLayout) -> DecodeResult<Self> {
        Ok(Self {
            langid: Wire::read(src, layout)?,
            clsid: Wire::read(src, layout)?,
            guid_profile: Wire::read(src, layout)?,
            catid: Wire::read(src, layout)?,
            hkl: Wire::read(src, layout)?,
            klid: Wire::read(src, layout)?,
            lcid: Wire::read(src, layout)?,
            profile_type: Wire::read(src, layout)?,
            caps: Wire::read(src, layout)?,
            flags: Wire::read(src, layout)?,
            bcp_tag: if layout.bcp_tag { Wire::read(src, layout)? } else { 0 },
        })
    }
}

wire_struct! {
    /// A region occluded by input UI ([2.2.1.7] CoreInputViewOcclusion).
    ///
    /// [2.2.1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct CoreInputViewOcclusion {
        pub event_id: u32,
        pub occluding_rect: TextInputRect,
        /// `Docked` (0), `Floating` (1), or `Overlay` (2).
        pub occlusion_kind: u32,
    }
}

wire_struct! {
    /// A range of caret positions, each placed before the character at that index
    /// ([2.2.1.8] EditControlRange).
    ///
    /// [2.2.1.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct EditControlRange {
        pub begin: u32,
        pub end: u32,
    }
}

wire_struct! {
    /// Settings of an edit control ([2.2.1.9] EditControlInfo).
    ///
    /// [2.2.1.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct EditControlInfo {
        /// Largest number of UTF-16 code units the control holds; `u32::MAX` means no limit.
        pub buffer_length: u32,
        pub edit_settings: u32,
        pub framework_type: u32,
        pub framework_version: u32,
        /// Edit control identifier, unique within its `TextInputClient`.
        pub id: u32,
        /// `InputScope` value.
        pub input_scope: u32,
        pub input_settings: u32,
        /// Unused and set to 0.
        pub visual_reference_id: u64,
    }
}

wire_struct! {
    /// A move of input focus between views ([2.2.1.10] NavigateFocusInfo).
    ///
    /// [2.2.1.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct NavigateFocusInfo {
        /// `NavigationReason` value.
        pub navigate_focus_reason: u32,
        pub origin: TextInputRect,
        pub start_target_view_id: u32,
        pub current_target_view_id: u32,
        pub sequence_number: Guid,
        pub start_time_stamp: u64,
    }
}

wire_struct! {
    /// Completion of a focus navigation ([2.2.1.11] NavigateFocusCompleteInfo).
    ///
    /// [2.2.1.11]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct NavigateFocusCompleteInfo {
        pub sequence_number: Guid,
        pub taken_focus: bool,
    }
}

wire_struct! {
    /// Configuration of a non-ComponentUI host ([2.2.1.12] NonCUIConfiguration).
    ///
    /// [2.2.1.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct NonCuiConfiguration {
        pub enable_non_cui_depart_focus: bool,
    }
}

wire_struct! {
    /// A rectangle ([2.2.1.13] TextInputRect).
    ///
    /// [2.2.1.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TextInputRect {
        pub left: u32,
        pub top: u32,
        pub right: u32,
        pub bottom: u32,
    }
}

wire_struct! {
    /// Visual formatting of text ([2.2.1.14] TextFormat).
    ///
    /// [2.2.1.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TextFormat {
        /// `CoreTextFormatUpdatingReason` value.
        pub reason: u32,
        pub set_background_color: bool,
        pub set_text_color: bool,
        pub set_underline_color: bool,
        pub set_underline_type: bool,
        /// `UnderlineType` value.
        pub underline_type: u8,
        /// `UIElementType` value.
        pub underline_color: u8,
        /// `UIElementType` value.
        pub background_color: u8,
        /// `UIElementType` value.
        pub text_color: u8,
    }
}

wire_struct! {
    /// Settings of a key target ([2.2.1.15] TextInputHostSettings).
    ///
    /// [2.2.1.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TextInputHostSettings {
        /// `TextInputHostType` value.
        pub host_type: u32,
        pub input_enabled_on_window_by_app: bool,
        pub is_owner_win32: bool,
        pub is_owner_app_frame: bool,
    }
}

/// Kind of notification acknowledged by `RDPTXT_ACKNOWLEDGE_OPERATION_PDU` and
/// `RDPTXT_ACKNOWLEDGE_HOST_OPERATION_PDU` ([2.2.2.21], [2.2.2.22]).
///
/// [2.2.2.21]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
/// [2.2.2.22]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AcknowledgementType(pub u32);

impl AcknowledgementType {
    pub const FOCUS_LOSS: Self = Self(0x0000_0000);
    pub const FOCUS_GAIN: Self = Self(0x0000_0001);
    pub const TEXT_CHANGE: Self = Self(0x0000_0002);
    pub const SELECTION_CHANGE: Self = Self(0x0000_0003);
    pub const UNDO: Self = Self(0x0000_0004);
    pub const START_NAVIGATE_FOCUS: Self = Self(0x0000_0005);
    pub const KEY_EVENT_CONSUMED: Self = Self(0x0000_0006);
    pub const KEY_EVENT_SKIPPED: Self = Self(0x0000_0007);
    pub const NAVIGATE_FOCUS: Self = Self(0x0000_0008);
    pub const DEPART_FOCUS: Self = Self(0x0000_0009);
    pub const COMPOSITION_TERMINATED: Self = Self(0x0000_000A);
    pub const CONVERSION_MODE_CHANGED: Self = Self(0x0000_000B);
    pub const FOCUS_LEAVE_COMPLETED: Self = Self(0x0000_000C);
}

/// Progress reported by `RDPTXT_ACKNOWLEDGE_KEY_EVENT_PDU` ([2.2.2.45]).
///
/// [2.2.2.45]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyEventAckType(pub u16);

impl KeyEventAckType {
    /// The application processed the key event synchronously.
    pub const COMPLETED: Self = Self(0x0001);
    /// The application received the key event and processes it asynchronously.
    pub const RAISED: Self = Self(0x0002);
}

/// Accelerator key routing stage of `RDPTXT_KEY_EVENT_PDU` ([2.2.2.1]).
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoutingStage(pub u8);

impl RoutingStage {
    pub const TUNNELING: Self = Self(0x00);
    pub const BUBBLING: Self = Self(0x01);
    /// A normal key event rather than a routed accelerator key.
    pub const NONE: Self = Self(0x02);
    pub const DIRECT: Self = Self(0x03);
}

/// Composition step of `RDPTXT_UPDATE_COMPOSITION_PDU` ([2.2.2.13]).
///
/// [2.2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompositionAction(pub u8);

impl CompositionAction {
    pub const ENTER_COMPOSITION: Self = Self(0x01);
    pub const LEAVE_COMPOSITION: Self = Self(0x02);
    pub const UPDATE_COMPOSITION: Self = Self(0x03);
}

/// Options of `RDPTXT_REPORT_CLIENT_OPTIONS_PDU` ([2.2.2.54]).
///
/// [2.2.2.54]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ClientOptions(pub u32);

impl ClientOptions {
    pub const NONE: Self = Self(0x0000_0000);
    /// The client reports the keys it sends to its own text input system, so the server does not
    /// route them back with `RDPTXT_SEND_KEY_TO_HOST_PDU`.
    pub const ENABLE_PREDICTED_KEY_REPORTING: Self = Self(0x0000_0001);
}

wire_newtype! {
    AcknowledgementType(u32);
    KeyEventAckType(u16);
    RoutingStage(u8);
    CompositionAction(u8);
    ClientOptions(u32);
}

/// Strings each preceded by their length in code units, preceded by their total size in bytes
/// (**candidateList** of [2.2.2.15] RDPTXT_RECONVERSION_CANDIDATES_PDU).
///
/// [2.2.2.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CandidateList(pub Vec<TextString>);

impl Wire for CandidateList {
    fn wire_size(&self, layout: PduLayout) -> usize {
        sized_list_size(&self.0, layout)
    }

    fn write(&self, dst: &mut WriteCursor<'_>, layout: PduLayout) -> EncodeResult<()> {
        write_sized_list(&self.0, "candidateListSize", dst, layout)
    }

    fn read(src: &mut ReadCursor<'_>, layout: PduLayout) -> DecodeResult<Self> {
        read_sized_list(src, "candidateListSize", layout).map(Self)
    }
}

/// Occlusions preceded by their total size in bytes (**occludingViews** of
/// [2.2.2.52] RDPTXT_OCCLUDING_VIEWS_PDU).
///
/// [2.2.2.52]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OcclusionList(pub Vec<CoreInputViewOcclusion>);

impl Wire for OcclusionList {
    fn wire_size(&self, layout: PduLayout) -> usize {
        sized_list_size(&self.0, layout)
    }

    fn write(&self, dst: &mut WriteCursor<'_>, layout: PduLayout) -> EncodeResult<()> {
        write_sized_list(&self.0, "occludingViewsSize", dst, layout)
    }

    fn read(src: &mut ReadCursor<'_>, layout: PduLayout) -> DecodeResult<Self> {
        read_sized_list(src, "occludingViewsSize", layout).map(Self)
    }
}

/// Size of `items` preceded by their 4-byte total size in bytes.
fn sized_list_size<T: Wire>(items: &[T], layout: PduLayout) -> usize {
    4 /* size */ + items.iter().map(|item| item.wire_size(layout)).sum::<usize>()
}

/// Writes the 4-byte total size in bytes of `items`, in field `field`, then `items`.
fn write_sized_list<T: Wire>(
    items: &[T],
    field: &'static str,
    dst: &mut WriteCursor<'_>,
    layout: PduLayout,
) -> EncodeResult<()> {
    let size: u32 = cast_length!(field, sized_list_size(items, layout) - 4 /* size */)?;
    size.write(dst, layout)?;
    items.iter().try_for_each(|item| item.write(dst, layout))
}

/// Reads a 4-byte total size in bytes, in field `field`, then the items filling that size.
fn read_sized_list<T: Wire>(src: &mut ReadCursor<'_>, field: &'static str, layout: PduLayout) -> DecodeResult<Vec<T>> {
    let size = usize::try_from(u32::read(src, layout)?).map_err(|_| invalid_field_err!(field, "too large", in: src))?;
    if src.len() < size {
        return Err(invalid_field_err!(field, "exceeds the remaining data", in: src));
    }
    let mut list = ReadCursor::new(src.read_slice(size));
    let mut items = Vec::new();
    while !list.is_empty() {
        items.push(T::read(&mut list, layout)?);
    }
    Ok(items)
}

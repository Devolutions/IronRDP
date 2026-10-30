//! PDUs sent from server to client.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::types::{
    CoreInputProfile, EditControlInfo, EditControlRange, HotKeyRegistrationData, KeyEventAckType, KeyEventHostInfo,
    TextInputHostSettings, TextInputRect,
};
use super::{Guid, TextString};
use crate::wire::wire_struct;

wire_struct! {
    /// [2.2.2.16] RDPTXT_DO_RECONVERSION_PDU.
    ///
    /// [2.2.2.16]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct DoReconversionPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        /// Requests the reconversion range and candidates.
        pub return_candidates: bool,
        /// Requests the reconversion range.
        pub return_range: bool,
    }
}

wire_struct! {
    /// [2.2.2.24] RDPTXT_REGISTER_REMOTE_TEXT_TARGET_PDU.
    ///
    /// [2.2.2.24]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RegisterRemoteTextTargetPdu {
        /// Identifier of the `TextInputClient`.
        pub text_target_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.25] RDPTXT_REGISTER_REMOTE_KEY_TARGET_PDU.
    ///
    /// [2.2.2.25]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RegisterRemoteKeyTargetPdu {
        /// Identifier of the `TextInputHost`.
        pub object_id: u32,
        /// Identifier of the `TextInputClient` on the same thread.
        pub text_target_id: u32,
        pub text_input_host_settings: TextInputHostSettings,
        pub view_instance_id: u64,
        pub window_instance_id: u64,
    }
}

wire_struct! {
    /// [2.2.2.26] RDPTXT_REGISTER_REMOTE_EDIT_CONTROL_PDU.
    ///
    /// [2.2.2.26]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RegisterRemoteEditControlPdu {
        pub app_name: TextString,
        pub edit_client_operation_id: u32,
        pub edit_control_id: u32,
        pub text_input_client_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.27] RDPTXT_REGISTER_REMOTE_COREINPUTVIEW_PDU.
    ///
    /// [2.2.2.27]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RegisterRemoteCoreInputViewPdu {
        pub object_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.28] RDPTXT_UNREGISTER_REMOTE_TEXT_TARGET_PDU.
    ///
    /// [2.2.2.28]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UnregisterRemoteTextTargetPdu {
        pub object_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.29] RDPTXT_UNREGISTER_REMOTE_KEY_TARGET_PDU.
    ///
    /// [2.2.2.29]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UnregisterRemoteKeyTargetPdu {
        pub object_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.30] RDPTXT_UNREGISTER_REMOTE_EDIT_CONTROL_PDU.
    ///
    /// [2.2.2.30]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UnregisterRemoteEditControlPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.31] RDPTXT_UNREGISTER_REMOTE_COREINPUTVIEW_PDU.
    ///
    /// [2.2.2.31]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UnregisterRemoteCoreInputViewPdu {
        pub object_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.32] RDPTXT_EDIT_CONTROL_FOCUS_PDU.
    ///
    /// [2.2.2.32]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EditControlFocusPdu {
        pub text_input_client_id: u32,
        /// Bounds of the control gaining focus.
        pub control_bounds: TextInputRect,
        pub edit_info: EditControlInfo,
        pub gaining_focus: bool,
        /// On focus gain, the control that lost focus.
        pub losing_focus_control_id: u32,
        /// On focus gain to another `TextInputHost`, the host that lost focus.
        pub losing_focus_text_input_host_id: u32,
        /// The change answers the oldest pending key event, which is not to be replayed.
        pub override_: bool,
    }
}

wire_struct! {
    /// [2.2.2.33] RDPTXT_HOST_FOCUS_PDU.
    ///
    /// [2.2.2.33]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HostFocusPdu {
        pub text_input_host_id: u32,
        /// Sequence number per host; 0 means the sequence does not apply.
        pub ordinal: u32,
        pub gaining_focus: bool,
        pub override_: bool,
    }
}

wire_struct! {
    /// [2.2.2.34] RDPTXT_HOST_FOREGROUND_PDU.
    ///
    /// [2.2.2.34]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HostForegroundPdu {
        /// Identifier of the `TextInputHost`.
        pub object_id: u32,
        pub window_instance_id: u64,
    }
}

wire_struct! {
    /// [2.2.2.35] RDPTXT_SELECTION_CHANGED_PDU.
    ///
    /// [2.2.2.35]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SelectionChangedPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub selection_begin: i32,
        pub selection_end: i32,
        pub override_: bool,
        /// Key event that caused the change, if any.
        pub origin_key: KeyEventHostInfo,
    }
}

wire_struct! {
    /// [2.2.2.36] RDPTXT_TEXT_CHANGED_PDU.
    ///
    /// [2.2.2.36]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TextChangedPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub replaced_text_range: EditControlRange,
        pub new_selection_range: EditControlRange,
        pub operation_id: u32,
        /// Length of the inserted text.
        pub text_length: u32,
        /// Key event that caused the change, if any.
        pub origin_key: KeyEventHostInfo,
        pub override_: bool,
        /// With `override_`, pending changes are kept and pending keys are not replayed.
        pub no_conflict: bool,
        /// Position of `updated_text_region1` in the edit buffer.
        pub offset1: i32,
        pub updated_text_region1: TextString,
        /// Position of `updated_text_region2` in the edit buffer, or -1 when absent.
        pub offset2: i32,
        pub updated_text_region2: TextString,
        /// One `KEY_STATE_FLAGS` byte per virtual key, typically 256.
        pub key_states: Vec<u8>,
    }
}

wire_struct! {
    /// [2.2.2.37] RDPTXT_CONTROL_CONFIGURATION_UPDATED_PDU.
    ///
    /// [2.2.2.37]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ControlConfigurationUpdatedPdu {
        pub text_input_client_id: u32,
        pub edit_info: EditControlInfo,
    }
}

wire_struct! {
    /// [2.2.2.38] RDPTXT_CONTROL_CONVERSION_MODE_UPDATED_PDU.
    ///
    /// [2.2.2.38]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ControlConversionModeUpdatedPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        /// `ImeConversionMode` value.
        pub conversion_mode: u32,
        /// Input method whose conversion mode changes, only if it is active.
        pub profile_id: Guid,
    }
}

wire_struct! {
    /// [2.2.2.39] RDPTXT_GEOMETRY_CHANGED_PDU.
    ///
    /// [2.2.2.39]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GeometryChangedPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub control_bounds: TextInputRect,
        pub range: EditControlRange,
        pub range_bounds: TextInputRect,
    }
}

wire_struct! {
    /// [2.2.2.40] RDPTXT_SOFTWARE_KEYBOARD_INVOCATION_SIGNALS_PDU.
    ///
    /// [2.2.2.40]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SoftwareKeyboardInvocationSignalsPdu {
        pub text_input_client_id: u32,
        pub control_bounds: TextInputRect,
        pub is_focus_in_edit: bool,
        pub last_touch_in_this_control: bool,
        pub manual_sip_invocation: bool,
        pub pen: bool,
        pub touch: bool,
    }
}

wire_struct! {
    /// [2.2.2.41] RDPTXT_ACTIVE_VIEW_CHANGED_PDU.
    ///
    /// [2.2.2.41]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ActiveViewChangedPdu {
        pub process_id: u32,
        pub thread_id: u32,
        pub window_id: u64,
    }
}

wire_struct! {
    /// [2.2.2.42] RDPTXT_FOREGROUND_HOST_INFO_UPDATED_PDU.
    ///
    /// [2.2.2.42]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ForegroundHostInfoUpdatedPdu {
        /// JSON description of the foreground `TextInputHost`.
        pub foreground_host_info: TextString,
    }
}

wire_struct! {
    /// [2.2.2.43] RDPTXT_NOTIFY_SERVER_VERSION_PDU.
    ///
    /// [2.2.2.43]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NotifyServerVersionPdu {
        /// Reserved; the server sends all zeros.
        pub container_id: Guid,
        pub version_major: u32,
        /// Bitmask of the updates introduced since `version_major`.
        pub version_minor: u32,
    }
}

wire_struct! {
    /// [2.2.2.44] RDPTXT_ACKNOWLEDGE_REMOTE_OPERATION_PDU.
    ///
    /// [2.2.2.44]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcknowledgeRemoteOperationPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub operation_id: u32,
        /// `HRESULT` of a failed operation.
        pub error_code: i32,
    }
}

wire_struct! {
    /// [2.2.2.45] RDPTXT_ACKNOWLEDGE_KEY_EVENT_PDU.
    ///
    /// [2.2.2.45]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcknowledgeKeyEventPdu {
        pub text_input_host_id: u32,
        pub key_event_id: u32,
        pub acknowledgement_type: KeyEventAckType,
    }
}

wire_struct! {
    /// [2.2.2.46] RDPTXT_INPUT_PROFILE_CHANGED_PDU.
    ///
    /// [2.2.2.46]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct InputProfileChangedPdu {
        pub text_input_client_id: u32,
        pub profile: CoreInputProfile,
    }
}

wire_struct! {
    /// [2.2.2.47] RDPTXT_VIEW_OCCLUSIONS_HANDLED_PDU.
    ///
    /// [2.2.2.47]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ViewOcclusionsHandledPdu {
        pub event_id: u32,
        pub handled: bool,
    }
}

wire_struct! {
    /// [2.2.2.48] RDPTXT_EDIT_CONTROL_TEXT_SEGMENT_PDU.
    ///
    /// [2.2.2.48]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct EditControlTextSegmentPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        /// TRUE populates `text` at `cp_start`; FALSE removes `cp_start..cp_end`.
        pub populate: bool,
        pub cp_start: i32,
        pub cp_end: i32,
        pub text: TextString,
    }
}

wire_struct! {
    /// [2.2.2.49] RDPTXT_COMPOSITION_TERMINATED_PDU.
    ///
    /// [2.2.2.49]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CompositionTerminatedPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub override_: bool,
    }
}

wire_struct! {
    /// [2.2.2.50] RDPTXT_SOFTWARE_KEYBOARD_POLICY_PDU.
    ///
    /// [2.2.2.50]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SoftwareKeyboardPolicyPdu {
        pub text_input_host_id: u32,
        pub keep_visible_on_focus_loss: bool,
    }
}

wire_struct! {
    /// [2.2.2.55] RDPTXT_SOFTWARE_KEYBOARD_VISIBILITY_PDU.
    ///
    /// [2.2.2.55]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SoftwareKeyboardVisibilityPdu {
        pub text_input_host_id: u32,
        /// `CoreInputViewKind` value.
        pub view_type: u32,
        pub visible: bool,
    }
}

wire_struct! {
    /// [2.2.2.56] RDPTXT_HOTKEY_REGISTRATION_PDU.
    ///
    /// [2.2.2.56]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct HotKeyRegistrationPdu {
        pub registration: HotKeyRegistrationData,
        pub enabled: bool,
    }
}

wire_struct! {
    /// [2.2.2.57] RDPTXT_REFRESH_CLIENT_PDU, which has no fields.
    ///
    /// [2.2.2.57]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct RefreshClientPdu {}
}

wire_struct! {
    /// [2.2.2.58] RDPTXT_UNDO_PENDING_KEY_EVENTS_PDU.
    ///
    /// [2.2.2.58]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UndoPendingKeyEventsPdu {
        pub undo_request_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.59] RDPTXT_SEND_KEY_TO_HOST_PDU.
    ///
    /// [2.2.2.59]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SendKeyToHostPdu {
        pub op_id: u32,
        /// Serialized `KeyPressInfo` ([2.2.1.3]), kept opaque.
        ///
        /// [2.2.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
        pub key_press_info: Vec<u8>,
    }
}

wire_struct! {
    /// [2.2.2.60] RDPTXT_REMOTE_TEXT_TARGET_THREAD_PROPERTIES_PDU.
    ///
    /// [2.2.2.60]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RemoteTextTargetThreadPropertiesPdu {
        pub text_input_client_id: u32,
        /// `IsWin32App` (0x00000001) bitmask.
        pub thread_properties: u32,
    }
}

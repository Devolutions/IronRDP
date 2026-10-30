//! PDUs sent from client to server.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use super::TextString;
use super::types::{
    AcknowledgementType, CandidateList, ClientOptions, CompositionAction, CompositionClause, CoreInputProfile,
    EditControlRange, KeyEventHostInfo, NavigateFocusCompleteInfo, NavigateFocusInfo, NonCuiConfiguration,
    OcclusionList, RoutingStage, TextFormat,
};
use crate::wire::wire_struct;

wire_struct! {
    /// [2.2.2.1] RDPTXT_KEY_EVENT_PDU.
    ///
    /// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct KeyEventPdu {
        pub text_input_host_id: u32,
        pub key_event_id: u32,
        pub routing_stage: RoutingStage,
        pub last_seen_key_event_id: u32,
        pub edit_control_id: u32,
        pub notify_framework: bool,
        pub key_event_info: KeyEventHostInfo,
        /// One `KEY_STATE_FLAGS` byte per virtual key, typically 256.
        pub key_states: Vec<u8>,
        pub key_text: TextString,
        pub dead_char: u16,
        pub key_name_text: TextString,
    }
}

wire_struct! {
    /// [2.2.2.2] RDPTXT_CHARACTER_EVENT_PDU.
    ///
    /// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CharacterEventPdu {
        pub text_input_host_id: u32,
        pub key_event_id: u32,
        pub edit_control_id: u32,
        pub key_down_event_id: u32,
        pub key_event_info: KeyEventHostInfo,
    }
}

wire_struct! {
    /// [2.2.2.3] RDPTXT_FOCUS_NAVIGATING_EVENT_PDU.
    ///
    /// [2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FocusNavigatingEventPdu {
        pub text_input_host_id: u32,
        pub navigate_focus_info: NavigateFocusInfo,
    }
}

wire_struct! {
    /// [2.2.2.4] RDPTXT_FOCUS_DEPART_COMPLETED_PDU.
    ///
    /// [2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FocusDepartCompletedPdu {
        pub text_input_host_id: u32,
        pub navigate_focus_complete_info: NavigateFocusCompleteInfo,
    }
}

wire_struct! {
    /// [2.2.2.5] RDPTXT_ENABLE_WINDOW_PDU.
    ///
    /// [2.2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EnableWindowPdu {
        pub text_input_host_id: u32,
        pub input_enabled: bool,
    }
}

wire_struct! {
    /// [2.2.2.6] RDPTXT_ACTIVATION_STATE_CHANGE_PDU.
    ///
    /// [2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ActivationStateChangePdu {
        pub text_input_host_id: u32,
        /// `ViewActivationState` value.
        pub activation_state: u32,
        pub activated_view: u32,
    }
}

wire_struct! {
    /// [2.2.2.7] RDPTXT_NON_COMPONENTUI_CONFIGURATION_CHANGE_PDU.
    ///
    /// [2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NonComponentUiConfigurationChangePdu {
        pub text_input_host_id: u32,
        pub non_cui_configuration: NonCuiConfiguration,
    }
}

wire_struct! {
    /// [2.2.2.8] RDPTXT_KEY_EVENT_PAYLOAD_PDU.
    ///
    /// [2.2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct KeyEventPayloadPdu {
        pub text_input_host_id: u32,
        pub edit_control_id: u32,
        pub key_event_id: u32,
        /// Reserved; set to 0.
        pub remote_key_event_id: u8,
        pub key_event_info: KeyEventHostInfo,
        /// TRUE for the bookend that opens the payload, FALSE for the one that closes it.
        pub begin_payload: bool,
        /// On the closing bookend, whether the input method handled the key event.
        pub handled: bool,
    }
}

wire_struct! {
    /// [2.2.2.9] RDPTXT_UPDATE_TEXT_PDU.
    ///
    /// [2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UpdateTextPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub replace_begin: u32,
        pub replace_end: i32,
        pub new_text: TextString,
    }
}

wire_struct! {
    /// [2.2.2.10] RDPTXT_UPDATE_TEXT_AND_SELECTION_PDU.
    ///
    /// [2.2.2.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UpdateTextAndSelectionPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub replace_begin: u32,
        pub replace_end: u32,
        pub new_text: TextString,
        pub selection_begin: u32,
        pub selection_end: u32,
    }
}

wire_struct! {
    /// [2.2.2.11] RDPTXT_SET_SELECTION_PDU.
    ///
    /// [2.2.2.11]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SetSelectionPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub selection_begin: u32,
        pub selection_end: u32,
        /// `CaretBindDirection`: `Previous` (0), `Next` (1), or `Neutral` (2).
        pub bind_direction: u32,
    }
}

wire_struct! {
    /// [2.2.2.12] RDPTXT_UPDATE_FORMAT_PDU.
    ///
    /// [2.2.2.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UpdateFormatPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub format_begin: u32,
        pub format_end: u32,
        /// Mutually exclusive with `format_basic`.
        pub format: TextFormat,
        /// `EditStyle` bitmask, mutually exclusive with `format`.
        pub format_basic: u32,
    }
}

wire_struct! {
    /// [2.2.2.13] RDPTXT_UPDATE_COMPOSITION_PDU.
    ///
    /// [2.2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UpdateCompositionPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub composition_action: CompositionAction,
        pub clauses: Vec<CompositionClause>,
    }
}

wire_struct! {
    /// [2.2.2.14] RDPTXT_SET_COMPOSITION_INFO_PDU.
    ///
    /// [2.2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SetCompositionInfoPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub text_input_host_id: u32,
        pub operation_id: u32,
        pub composition_range: EditControlRange,
        /// Text that replaces the composition when it ends.
        pub determined_text: TextString,
    }
}

wire_struct! {
    /// [2.2.2.15] RDPTXT_RECONVERSION_CANDIDATES_PDU.
    ///
    /// [2.2.2.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ReconversionCandidatesPdu {
        pub text_input_client_id: u32,
        pub candidate_list: CandidateList,
        pub edit_control_id: u32,
        pub reconvert_range: EditControlRange,
    }
}

wire_struct! {
    /// [2.2.2.17] RDPTXT_UPDATE_INPUT_LOCALE_PDU.
    ///
    /// [2.2.2.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UpdateInputLocalePdu {
        pub text_input_client_id: u32,
        pub text_input_host_id: u32,
        pub edit_control_id: u32,
        pub locale_id: u32,
        pub operation_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.18] RDPTXT_UPDATE_INPUT_PROFILE_PDU.
    ///
    /// [2.2.2.18]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct UpdateInputProfilePdu {
        pub text_input_client_id: u32,
        pub profile: CoreInputProfile,
        /// TRUE when initializing a remote application thread, FALSE when activating `profile`.
        pub initializing: bool,
    }
}

wire_struct! {
    /// [2.2.2.19] RDPTXT_UPDATE_MODE_PDU.
    ///
    /// [2.2.2.19]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct UpdateModePdu {
        pub text_input_host_id: u32,
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        /// `TextInputFeature` bitmask.
        pub features: u32,
        pub enabled: bool,
        pub custom_range: EditControlRange,
        pub prediction_mode_triggers: TextString,
        pub operation_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.20] RDPTXT_SET_CONVERSION_MODE_PDU.
    ///
    /// [2.2.2.20]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SetConversionModePdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        /// `ImeConversionMode` value.
        pub mode: u32,
    }
}

wire_struct! {
    /// [2.2.2.21] RDPTXT_ACKNOWLEDGE_OPERATION_PDU.
    ///
    /// [2.2.2.21]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcknowledgeOperationPdu {
        pub text_input_client_id: u32,
        pub edit_control_id: u32,
        pub acknowledgement_type: AcknowledgementType,
        pub operation_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.22] RDPTXT_ACKNOWLEDGE_HOST_OPERATION_PDU.
    ///
    /// [2.2.2.22]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcknowledgeHostOperationPdu {
        pub text_input_host_id: u32,
        pub acknowledgement_type: AcknowledgementType,
    }
}

wire_struct! {
    /// [2.2.2.23] RDPTXT_ACKNOWLEDGE_UNDO_PENDING_KEY_EVENTS_PDU.
    ///
    /// [2.2.2.23]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcknowledgeUndoPendingKeyEventsPdu {
        pub undo_request_id: u32,
    }
}

wire_struct! {
    /// [2.2.2.51] RDPTXT_SET_ENABLED_INPUT_PROFILES_PDU.
    ///
    /// [2.2.2.51]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SetEnabledInputProfilesPdu {
        pub profiles: Vec<CoreInputProfile>,
    }
}

wire_struct! {
    /// [2.2.2.52] RDPTXT_OCCLUDING_VIEWS_PDU.
    ///
    /// [2.2.2.52]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct OccludingViewsPdu {
        pub core_input_view_id: u32,
        pub occluding_views: OcclusionList,
        pub precalculated_occlusion: bool,
    }
}

wire_struct! {
    /// [2.2.2.53] RDPTXT_NOTIFY_CLIENT_VERSION_PDU.
    ///
    /// [2.2.2.53]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct NotifyClientVersionPdu {
        pub version_major: u32,
        /// Bitmask of the updates introduced since `version_major`.
        pub version_minor: u32,
    }
}

wire_struct! {
    /// [2.2.2.54] RDPTXT_REPORT_CLIENT_OPTIONS_PDU.
    ///
    /// [2.2.2.54]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ReportClientOptionsPdu {
        pub options: ClientOptions,
    }
}

wire_struct! {
    /// [2.2.2.61] RDPTXT_REREGISTRATION_REQUEST_PDU, which has no fields.
    ///
    /// [2.2.2.61]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ReregistrationRequestPdu {}
}

wire_struct! {
    /// [2.2.2.62] RDPTXT_REMOTE_INTEGRATION_STATUS_PDU.
    ///
    /// [2.2.2.62]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RemoteIntegrationStatusPdu {
        /// FALSE turns remote text input integration off while keeping the channels open.
        pub is_enabled: bool,
    }
}

wire_struct! {
    /// [2.2.2.63] RDPTXT_ERROR_REPORT_PDU.
    ///
    /// [2.2.2.63]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ErrorReportPdu {
        pub text_input_client_id: u32,
        pub text_input_host_id: u32,
        pub edit_control_id: u32,
    }
}

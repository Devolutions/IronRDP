//! Session notifications raised through `IMsTscAxEvents` and the modern `IRemoteDesktopClientEvents` facade.
//!
//! Each notification maps to at most one classic event DISPID and one modern facade event name.
//! Arguments are listed from the last `DISPPARAMS::rgvarg` slot to the first, which is type-library order for
//! every event except the two RemoteApp display events (see [`SessionNotification::RemoteProgramDisplayed`]).

use ironrdp_pdu::rdp::session_info::LogonErrorsInfo;

use super::VariantValue;

const DISPID_ON_REQUEST_CONTAINER_MINIMIZE: i32 = 14;
const DISPID_ON_REMOTE_PROGRAM_DISPLAYED: i32 = 21;
const DISPID_ON_LOGON_ERROR: i32 = 22;
const DISPID_ON_USER_NAME_ACQUIRED: i32 = 24;
const DISPID_ON_REMOTE_WINDOW_DISPLAYED: i32 = 29;
const DISPID_ON_NETWORK_STATUS_CHANGED: i32 = 32;

/// `remoteAppWindowDisplayed`, the `RemoteWindowDisplayedAttribute` of a projected RemoteApp window.
pub(super) const REMOTE_APP_WINDOW_DISPLAYED: u32 = 1;

/// Logon notification types at or above this value are `LOGON_MSG_*` codes and are reported instead of the data.
const LOGON_MESSAGE_TYPE_MINIMUM: u32 = 0xFFFF_FF00;

/// Network quality is unknown until the server reports a base round-trip time.
const NETWORK_QUALITY_UNKNOWN: u32 = 0;

/// The value reported for a network metric the server has not measured.
const NETWORK_METRIC_UNKNOWN: i32 = -1;

/// A notification raised on the creating apartment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SessionNotification {
    /// The connection-bar Minimize button asks the container to minimize.
    RequestContainerMinimize,
    /// The server reported a logon error or warning.
    ///
    /// The value is the `LOGON_MSG_*` notification type when the server sent one, and the notification data otherwise.
    LogonError(i32),
    /// The control acquired the account name from its own credential prompt.
    UserNameAcquired(String),
    /// The server reported new network characteristics.
    NetworkStatus {
        quality_level: u32,
        bandwidth_kbps: i32,
        rtt_ms: i32,
    },
    /// The first projected RemoteApp window became visible, or the last one was hidden.
    ///
    /// mstscax places `vbDisplayed` in `rgvarg[0]` and `uDisplayInformation` in `rgvarg[1]`, the reverse of the
    /// Automation convention; the control reproduces that layout so raw `IDispatch` sinks see the same slots.
    RemoteProgramDisplayed { displayed: bool, display_information: u32 },
    /// A projected RemoteApp window became visible or was hidden.
    ///
    /// Like [`Self::RemoteProgramDisplayed`], mstscax fills `rgvarg` in parameter order: `vbDisplayed`, then the
    /// window handle truncated to 32 bits, then the attribute. Hidden windows report a zero handle and attribute.
    RemoteWindowDisplayed {
        displayed: bool,
        window: u32,
        attribute: u32,
    },
}

impl SessionNotification {
    /// Builds the logon-error notification from a Save Session Info logon errors structure.
    pub(super) fn logon_error(errors: &LogonErrorsInfo) -> Self {
        let error_type = errors.error_type.as_u32();
        let value = if LOGON_MESSAGE_TYPE_MINIMUM <= error_type {
            error_type
        } else {
            errors.error_data.to_u32()
        };
        Self::LogonError(i32::from_ne_bytes(value.to_ne_bytes()))
    }

    /// Returns the classic `IMsTscAxEvents` DISPID and arguments; every notification has one.
    pub(super) fn classic_event(&self) -> (i32, Vec<VariantValue>) {
        match self {
            Self::RequestContainerMinimize => (DISPID_ON_REQUEST_CONTAINER_MINIMIZE, Vec::new()),
            Self::LogonError(error) => (DISPID_ON_LOGON_ERROR, vec![VariantValue::Integer(*error)]),
            Self::UserNameAcquired(user_name) => (
                DISPID_ON_USER_NAME_ACQUIRED,
                vec![VariantValue::String(user_name.clone())],
            ),
            Self::NetworkStatus {
                quality_level,
                bandwidth_kbps,
                rtt_ms,
            } => (
                DISPID_ON_NETWORK_STATUS_CHANGED,
                vec![
                    VariantValue::Unsigned(*quality_level),
                    VariantValue::Integer(*bandwidth_kbps),
                    VariantValue::Integer(*rtt_ms),
                ],
            ),
            Self::RemoteProgramDisplayed {
                displayed,
                display_information,
            } => (
                DISPID_ON_REMOTE_PROGRAM_DISPLAYED,
                vec![
                    VariantValue::Integer(i32::from_ne_bytes(display_information.to_ne_bytes())),
                    VariantValue::Bool(*displayed),
                ],
            ),
            Self::RemoteWindowDisplayed {
                displayed,
                window,
                attribute,
            } => (
                DISPID_ON_REMOTE_WINDOW_DISPLAYED,
                vec![
                    VariantValue::Unsigned(*attribute),
                    VariantValue::Unsigned(*window),
                    VariantValue::Bool(*displayed),
                ],
            ),
        }
    }

    /// Returns the modern facade event name and arguments, if the modern facade has this event.
    pub(super) fn modern_event(&self) -> Option<(&'static str, Vec<VariantValue>)> {
        match self {
            Self::NetworkStatus {
                quality_level,
                bandwidth_kbps,
                rtt_ms,
            } => Some((
                "OnNetworkStatusChanged",
                vec![
                    VariantValue::Unsigned(*quality_level),
                    VariantValue::Integer(*bandwidth_kbps),
                    VariantValue::Integer(*rtt_ms),
                ],
            )),
            // The modern facade drops these classic events rather than re-raising them.
            Self::RequestContainerMinimize
            | Self::LogonError(_)
            | Self::UserNameAcquired(_)
            | Self::RemoteProgramDisplayed { .. }
            | Self::RemoteWindowDisplayed { .. } => None,
        }
    }
}

/// The latest network characteristics reported by the server for one connection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct NetworkStatusTracker {
    base_rtt_ms: Option<u32>,
    bandwidth_kbps: Option<u32>,
}

impl NetworkStatusTracker {
    /// Records a Network Characteristics Result and returns the notification for the updated state.
    ///
    /// Like mstscax, the reported round-trip time is the base RTT, the average RTT is ignored, and a metric the
    /// result omits keeps its previous value.
    pub(super) fn update(&mut self, base_rtt_ms: Option<u32>, bandwidth_kbps: Option<u32>) -> SessionNotification {
        if let Some(base_rtt_ms) = base_rtt_ms {
            self.base_rtt_ms = Some(base_rtt_ms);
        }
        if let Some(bandwidth_kbps) = bandwidth_kbps {
            self.bandwidth_kbps = Some(bandwidth_kbps);
        }

        let metric =
            |value: Option<u32>| value.map_or(NETWORK_METRIC_UNKNOWN, |value| i32::try_from(value).unwrap_or(i32::MAX));
        SessionNotification::NetworkStatus {
            quality_level: self.base_rtt_ms.map_or(NETWORK_QUALITY_UNKNOWN, network_quality_level),
            bandwidth_kbps: metric(self.bandwidth_kbps),
            rtt_ms: metric(self.base_rtt_ms),
        }
    }
}

/// Maps a round-trip time to mstscax's connection quality level, from 1 (poor) to 4 (excellent).
fn network_quality_level(rtt_ms: u32) -> u32 {
    if rtt_ms < 100 {
        4
    } else if rtt_ms < 200 {
        3
    } else if rtt_ms < 300 {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::rdp::session_info::{
        LogonErrorNotificationData, LogonErrorNotificationDataErrorCode, LogonErrorNotificationType,
    };

    use super::super::*;
    use super::*;

    type RecordedEvents = Arc<Mutex<Vec<(i32, Vec<Recorded>)>>>;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Recorded {
        I4(i32),
        Ui4(u32),
        Bool(bool),
        Bstr(String),
    }

    /// Records every invocation with its arguments in physical `rgvarg` order.
    #[implement(IDispatch)]
    struct RecordingSink {
        seen: RecordedEvents,
    }

    impl IDispatch_Impl for RecordingSink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
            Err(Error::from_hresult(E_NOTIMPL))
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _dispids: *mut i32,
        ) -> Result<()> {
            Err(Error::from_hresult(DISP_E_UNKNOWNNAME))
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            flags: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> Result<()> {
            assert!(flags.contains(DISPATCH_METHOD));
            let params = unsafe { params.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            assert_eq!(params.cNamedArgs, 0);
            let arguments = if params.cArgs == 0 {
                &[][..]
            } else {
                unsafe { slice::from_raw_parts(params.rgvarg, params.cArgs as usize) }
            };
            let recorded = arguments
                .iter()
                .map(|argument| {
                    let header = variant_header(argument);
                    match header.vt {
                        VT_I4 => Recorded::I4(unsafe { header.Anonymous.lVal }),
                        VT_UI4 => Recorded::Ui4(unsafe { header.Anonymous.ulVal }),
                        VT_BOOL => Recorded::Bool(unsafe { header.Anonymous.boolVal } != VARIANT_FALSE),
                        VT_BSTR => Recorded::Bstr(variant_string(argument, ptr::null_mut()).expect("valid BSTR")),
                        other => panic!("unexpected event argument type {other:?}"),
                    }
                })
                .collect();
            self.seen.lock().expect("recorded events").push((dispid, recorded));
            Ok(())
        }
    }

    fn recording_sink() -> (IDispatch, RecordedEvents) {
        let seen = RecordedEvents::default();
        let dispatch = RecordingSink {
            seen: Arc::clone(&seen),
        }
        .into();
        (dispatch, seen)
    }

    fn take(seen: &RecordedEvents) -> Vec<(i32, Vec<Recorded>)> {
        core::mem::take(&mut *seen.lock().expect("recorded events"))
    }

    #[test]
    fn classic_events_use_mstscax_argument_slots() {
        // oracle: reference mstscax.dll 10.0.26100.9711 IMsTscAxEvents DISPIDs and CProxy_IMsTscAxEvents::Fire_* rgvarg layout
        let control = Control::new();
        let (dispatch, seen) = recording_sink();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        control.raise_session_notification(&SessionNotification::RequestContainerMinimize);
        control.raise_session_notification(&SessionNotification::LogonError(-2));
        control.raise_session_notification(&SessionNotification::UserNameAcquired("CONTOSO\\alice".to_owned()));
        control.raise_session_notification(&SessionNotification::NetworkStatus {
            quality_level: 3,
            bandwidth_kbps: 50_000,
            rtt_ms: 120,
        });
        control.raise_session_notification(&SessionNotification::RemoteProgramDisplayed {
            displayed: true,
            display_information: 2,
        });
        control.raise_session_notification(&SessionNotification::RemoteWindowDisplayed {
            displayed: true,
            window: 0x0001_0203,
            attribute: REMOTE_APP_WINDOW_DISPLAYED,
        });

        assert_eq!(
            take(&seen),
            [
                (14, vec![]),
                (22, vec![Recorded::I4(-2)]),
                (24, vec![Recorded::Bstr("CONTOSO\\alice".to_owned())]),
                // The quality level is the first parameter, so it is in the last slot.
                (32, vec![Recorded::I4(120), Recorded::I4(50_000), Recorded::Ui4(3)]),
                // mstscax fills both RemoteApp display events in parameter order.
                (21, vec![Recorded::Bool(true), Recorded::I4(2)]),
                (
                    29,
                    vec![Recorded::Bool(true), Recorded::Ui4(0x0001_0203), Recorded::Ui4(1)]
                ),
            ]
        );
    }

    #[test]
    fn frozen_events_raise_no_session_notification() {
        let control = Control::new();
        let (dispatch, seen) = recording_sink();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        control.set_events_frozen(true);
        control.raise_session_notification(&SessionNotification::LogonError(0));
        assert!(take(&seen).is_empty());
    }

    #[test]
    fn modern_events_use_the_documented_argument_order() {
        let control = Control::new();
        let (callback, seen) = recording_sink();
        control
            .attach_modern_event("OnNetworkStatusChanged", callback.as_raw())
            .expect("network status has an event source");
        control
            .attach_modern_event("OnKeyCombinationPressed", callback.as_raw())
            .expect("key combinations have an event source");

        control.raise_session_notification(&SessionNotification::NetworkStatus {
            quality_level: 4,
            bandwidth_kbps: -1,
            rtt_ms: 20,
        });
        // The modern facade drops the classic logon-error event.
        control.raise_session_notification(&SessionNotification::LogonError(0));
        control.fire_event(DISPID_ON_CONNECTION_BAR_PULL_DOWN, &[]);

        assert_eq!(
            take(&seen),
            [
                (0, vec![Recorded::I4(20), Recorded::I4(-1), Recorded::Ui4(4)]),
                (0, vec![Recorded::I4(0)]),
            ]
        );
    }

    #[test]
    fn logon_error_reports_message_types_and_otherwise_the_data() {
        let message = LogonErrorsInfo {
            error_type: LogonErrorNotificationType::SessionContinue,
            error_data: LogonErrorNotificationData::SessionId(7),
        };
        assert_eq!(
            SessionNotification::logon_error(&message),
            SessionNotification::LogonError(-2)
        );

        let failure = LogonErrorsInfo {
            error_type: LogonErrorNotificationType::AccessDenied,
            error_data: LogonErrorNotificationData::ErrorCode(LogonErrorNotificationDataErrorCode::FailedBadPassword),
        };
        assert_eq!(
            SessionNotification::logon_error(&failure),
            SessionNotification::LogonError(-1)
        );

        // An NTSTATUS failure reports its LOGON_FAILED_* data, as mstscax does.
        let status = LogonErrorsInfo {
            error_type: LogonErrorNotificationType::NtStatus(0xC000_006D),
            error_data: LogonErrorNotificationData::ErrorCode(LogonErrorNotificationDataErrorCode::FailedOther),
        };
        assert_eq!(
            SessionNotification::logon_error(&status),
            SessionNotification::LogonError(2)
        );
    }

    #[test]
    fn network_status_keeps_omitted_metrics_and_grades_base_rtt() {
        let mut tracker = NetworkStatusTracker::default();
        assert_eq!(
            tracker.update(None, None),
            SessionNotification::NetworkStatus {
                quality_level: 0,
                bandwidth_kbps: -1,
                rtt_ms: -1,
            }
        );
        assert_eq!(
            tracker.update(Some(150), Some(8_000)),
            SessionNotification::NetworkStatus {
                quality_level: 3,
                bandwidth_kbps: 8_000,
                rtt_ms: 150,
            }
        );
        assert_eq!(
            tracker.update(None, Some(9_000)),
            SessionNotification::NetworkStatus {
                quality_level: 3,
                bandwidth_kbps: 9_000,
                rtt_ms: 150,
            }
        );
        for (rtt_ms, quality_level) in [(99, 4), (100, 3), (199, 3), (200, 2), (299, 2), (300, 1)] {
            assert_eq!(network_quality_level(rtt_ms), quality_level);
        }
    }

    #[test]
    fn projected_rail_windows_report_display_transitions() {
        let (input_sender, _input_receiver) = RdpInputSender::channel(16);
        let mut manager = RailWindowManager::new(
            Rc::new(RefCell::new(InputDatabase::new())),
            Rc::new(RefCell::new(CompatibilitySettings::default())),
            Rc::new(RefCell::new(None)),
            Rc::new(RefCell::new(None)),
        );
        manager.start(Some(input_sender));
        let order = |window_id, is_new, show_state| ProjectedRailWindowOrder {
            is_new,
            window_id,
            owner_window_id: None,
            style: None,
            show_state,
            title: None,
            client_area_offset: None,
            client_area_size: None,
            window_offset: None,
            client_delta: None,
            window_size: None,
        };
        let hidden = SessionNotification::RemoteWindowDisplayed {
            displayed: false,
            window: 0,
            attribute: 0,
        };

        manager.apply_window_order(order(1, true, Some(0)));
        assert!(manager.take_notifications().is_empty());

        manager.apply_window_order(order(1, false, Some(5)));
        let window = manager.windows.get(&1).expect("projected window").hwnd.0 as usize as u32;
        assert_eq!(
            manager.take_notifications(),
            [
                SessionNotification::RemoteWindowDisplayed {
                    displayed: true,
                    window,
                    attribute: REMOTE_APP_WINDOW_DISPLAYED,
                },
                SessionNotification::RemoteProgramDisplayed {
                    displayed: true,
                    display_information: 0,
                },
            ]
        );

        manager.apply_window_order(order(2, true, Some(5)));
        assert_eq!(manager.take_notifications().len(), 1);

        manager.apply_window_order(order(1, false, Some(0)));
        assert_eq!(manager.take_notifications(), slice::from_ref(&hidden));

        manager.destroy_window(2);
        assert_eq!(
            manager.take_notifications(),
            [
                hidden,
                SessionNotification::RemoteProgramDisplayed {
                    displayed: false,
                    display_information: 0,
                },
            ]
        );

        // Session teardown reports nothing, like mstscax.
        manager.apply_window_order(order(1, false, Some(5)));
        manager.stop();
        assert!(manager.take_notifications().is_empty());
    }
}

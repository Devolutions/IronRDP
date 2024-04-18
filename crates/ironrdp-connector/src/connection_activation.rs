use std::mem;

use ironrdp_pdu::{
    gcc::KeyboardType,
    rdp::{self, capability_sets::CapabilitySet},
};

use crate::{legacy, Config, ConnectionFinalizationSequence, ConnectorResult, DesktopSize, Sequence, State, Written};

/// Represents the Capability Exchange and Connection Finalization phases
/// of the connection sequence (section [1.3.1.1]).
///
/// This is abstracted into its own struct to allow it to be used for the ordinary
/// RDP connection sequence [`ClientConnector`] that occurs for every RDP connection,
/// as well as the Deactivation-Reactivation Sequence ([1.3.1.3]) that occurs when
/// a [Server Deactivate All PDU] is received.
///
/// [1.3.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/023f1e69-cfe8-4ee6-9ee0-7e759fb4e4ee
/// [1.3.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
/// [`ClientConnector`]: crate::ClientConnector
/// [Server Deactivate All PDU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/8a29971a-df3c-48da-add2-8ed9a05edc89
#[derive(Debug, Clone)]
pub struct ConnectionActivationSequence {
    pub state: ConnectionActivationState,
    config: Config,
}

impl ConnectionActivationSequence {
    pub fn new(config: Config, io_channel_id: u16, user_channel_id: u16) -> Self {
        Self {
            state: ConnectionActivationState::CapabilitiesExchange {
                io_channel_id,
                user_channel_id,
            },
            config,
        }
    }

    #[must_use]
    pub fn reset_clone(&self) -> Self {
        self.clone().reset()
    }

    fn reset(mut self) -> Self {
        match &self.state {
            ConnectionActivationState::CapabilitiesExchange {
                io_channel_id,
                user_channel_id,
            }
            | ConnectionActivationState::ConnectionFinalization {
                io_channel_id,
                user_channel_id,
                ..
            }
            | ConnectionActivationState::Finalized {
                io_channel_id,
                user_channel_id,
                ..
            } => {
                self.state = ConnectionActivationState::CapabilitiesExchange {
                    io_channel_id: *io_channel_id,
                    user_channel_id: *user_channel_id,
                };

                self
            }
            ConnectionActivationState::Consumed => self,
        }
    }
}

impl Sequence for ConnectionActivationSequence {
    fn next_pdu_hint(&self) -> Option<&dyn ironrdp_pdu::PduHint> {
        match &self.state {
            ConnectionActivationState::Consumed => None,
            ConnectionActivationState::Finalized { .. } => None,
            ConnectionActivationState::CapabilitiesExchange { .. } => Some(&ironrdp_pdu::X224_HINT),
            ConnectionActivationState::ConnectionFinalization {
                connection_finalization,
                ..
            } => connection_finalization.next_pdu_hint(),
        }
    }

    fn state(&self) -> &dyn State {
        &self.state
    }

    fn step(&mut self, input: &[u8], output: &mut ironrdp_pdu::write_buf::WriteBuf) -> ConnectorResult<Written> {
        let (written, next_state) = match mem::take(&mut self.state) {
            ConnectionActivationState::Consumed | ConnectionActivationState::Finalized { .. } => {
                return Err(general_err!(
                    "connector sequence state is finalized or consumed (this is a bug)"
                ));
            }
            ConnectionActivationState::CapabilitiesExchange {
                io_channel_id,
                user_channel_id,
            } => {
                debug!("Capabilities Exchange");

                let send_data_indication_ctx = legacy::decode_send_data_indication(input)?;
                let share_control_ctx = legacy::decode_share_control(send_data_indication_ctx)?;

                debug!(message = ?share_control_ctx.pdu, "Received");

                if share_control_ctx.channel_id != io_channel_id {
                    warn!(
                        io_channel_id,
                        share_control_ctx.channel_id, "Unexpected channel ID for received Share Control Pdu"
                    );
                }

                let capability_sets = if let rdp::headers::ShareControlPdu::ServerDemandActive(server_demand_active) =
                    share_control_ctx.pdu
                {
                    server_demand_active.pdu.capability_sets
                } else {
                    return Err(general_err!(
                        "unexpected Share Control Pdu (expected ServerDemandActive)",
                    ));
                };

                for c in &capability_sets {
                    if let rdp::capability_sets::CapabilitySet::General(g) = c {
                        if g.protocol_version != rdp::capability_sets::PROTOCOL_VER {
                            warn!(version = g.protocol_version, "Unexpected protocol version");
                        }
                        break;
                    }
                }

                // At this point we have already sent a requested desktop size to the server -- either as a part of the
                // [`TS_UD_CS_CORE`] (on initial connection) or the [`DISPLAYCONTROL_MONITOR_LAYOUT`] (on resize event).
                //
                // The server is therefore responding with a desktop size here, which will be close to the requested size but
                // may be slightly different due to server-side constraints. We should use this negotiated size for the rest of
                // the session.
                //
                // [TS_UD_CS_CORE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/00f1da4a-ee9c-421a-852f-c19f92343d73
                // [DISPLAYCONTROL_MONITOR_LAYOUT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
                let desktop_size = capability_sets
                    .iter()
                    .find_map(|c| match c {
                        rdp::capability_sets::CapabilitySet::Bitmap(b) => Some(DesktopSize {
                            width: b.desktop_width,
                            height: b.desktop_height,
                        }),
                        _ => None,
                    })
                    .unwrap_or(DesktopSize {
                        width: self.config.desktop_size.width,
                        height: self.config.desktop_size.height,
                    });

                let client_confirm_active = rdp::headers::ShareControlPdu::ClientConfirmActive(
                    create_client_confirm_active(&self.config, capability_sets, desktop_size),
                );

                debug!(message = ?client_confirm_active, "Send");

                let written = legacy::encode_share_control(
                    user_channel_id,
                    io_channel_id,
                    share_control_ctx.share_id,
                    client_confirm_active,
                    output,
                )?;

                (
                    Written::from_size(written)?,
                    ConnectionActivationState::ConnectionFinalization {
                        io_channel_id,
                        user_channel_id,
                        desktop_size,
                        connection_finalization: ConnectionFinalizationSequence::new(io_channel_id, user_channel_id),
                    },
                )
            }
            ConnectionActivationState::ConnectionFinalization {
                io_channel_id,
                user_channel_id,
                desktop_size,
                mut connection_finalization,
            } => {
                debug!("Connection Finalization");

                let written = connection_finalization.step(input, output)?;

                let next_state = if !connection_finalization.state.is_terminal() {
                    ConnectionActivationState::ConnectionFinalization {
                        io_channel_id,
                        user_channel_id,
                        desktop_size,
                        connection_finalization,
                    }
                } else {
                    ConnectionActivationState::Finalized {
                        io_channel_id,
                        user_channel_id,
                        desktop_size,
                        no_server_pointer: self.config.no_server_pointer,
                        pointer_software_rendering: self.config.pointer_software_rendering,
                    }
                };

                (written, next_state)
            }
        };

        self.state = next_state;

        Ok(written)
    }
}

#[derive(Default, Debug, Clone)]
pub enum ConnectionActivationState {
    #[default]
    Consumed,
    CapabilitiesExchange {
        io_channel_id: u16,
        user_channel_id: u16,
    },
    ConnectionFinalization {
        io_channel_id: u16,
        user_channel_id: u16,
        desktop_size: DesktopSize,
        connection_finalization: ConnectionFinalizationSequence,
    },
    Finalized {
        io_channel_id: u16,
        user_channel_id: u16,
        desktop_size: DesktopSize,
        no_server_pointer: bool,
        pointer_software_rendering: bool,
    },
}

impl State for ConnectionActivationState {
    fn name(&self) -> &'static str {
        match self {
            ConnectionActivationState::Consumed => "Consumed",
            ConnectionActivationState::CapabilitiesExchange { .. } => "CapabilitiesExchange",
            ConnectionActivationState::ConnectionFinalization { .. } => "ConnectionFinalization",
            ConnectionActivationState::Finalized { .. } => "Finalized",
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(self, ConnectionActivationState::Finalized { .. })
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

const DEFAULT_POINTER_CACHE_SIZE: u16 = 0x0019;

fn create_client_confirm_active(
    config: &Config,
    mut server_capability_sets: Vec<CapabilitySet>,
    desktop_size: DesktopSize,
) -> rdp::capability_sets::ClientConfirmActive {
    use ironrdp_pdu::rdp::capability_sets::*;

    server_capability_sets.retain(|capability_set| matches!(capability_set, CapabilitySet::MultiFragmentUpdate(_)));

    let lossy_bitmap_compression = config
        .bitmap
        .as_ref()
        .map(|bitmap| bitmap.lossy_compression)
        .unwrap_or(false);

    let drawing_flags = if lossy_bitmap_compression {
        BitmapDrawingFlags::ALLOW_SKIP_ALPHA
            | BitmapDrawingFlags::ALLOW_DYNAMIC_COLOR_FIDELITY
            | BitmapDrawingFlags::ALLOW_COLOR_SUBSAMPLING
    } else {
        BitmapDrawingFlags::ALLOW_SKIP_ALPHA
    };

    let mut order = Order::new(
        OrderFlags::NEGOTIATE_ORDER_SUPPORT
            | OrderFlags::ZERO_BOUNDS_DELTAS_SUPPORT
            | OrderFlags::COLOR_INDEX_SUPPORT
            | OrderFlags::ORDER_FLAGS_EXTRA_FLAGS,
        OrderSupportExFlags::ALTSEC_FRAME_MARKER_SUPPORT,
        0x00038400,
        0xFDE9,
    );
    order.set_support_flag(OrderSupportIndex::DstBlt, true);
    order.set_support_flag(OrderSupportIndex::PatBlt, true);
    order.set_support_flag(OrderSupportIndex::ScrBlt, true);
    order.set_support_flag(OrderSupportIndex::LineTo, true);
    order.set_support_flag(OrderSupportIndex::MultiOpaqueRect, true);
    order.set_support_flag(OrderSupportIndex::Polyline, true);

    server_capability_sets.extend_from_slice(&[
        CapabilitySet::General(General {
            major_platform_type: MajorPlatformType::UNIX,
            minor_platform_type: MinorPlatformType(0xFFFD),
            protocol_version: 0x0000,
            extra_flags: GeneralExtraFlags::FASTPATH_OUTPUT_SUPPORTED
                | GeneralExtraFlags::NO_BITMAP_COMPRESSION_HDR
                | GeneralExtraFlags::LONG_CREDENTIALS_SUPPORTED
                | GeneralExtraFlags::AUTORECONNECT_SUPPORTED
                | GeneralExtraFlags::ENC_SALTED_CHECKSUM,
            refresh_rect_support: true,
            suppress_output_support: true,
        }),
        CapabilitySet::Bitmap(Bitmap {
            pref_bits_per_pix: 32,
            desktop_width: desktop_size.width,
            desktop_height: desktop_size.height,
            // This is required to be true in order for the Microsoft::Windows::RDS::DisplayControl DVC to work.
            desktop_resize_flag: true,
            drawing_flags: BitmapDrawingFlags::ALLOW_DYNAMIC_COLOR_FIDELITY | BitmapDrawingFlags::ALLOW_SKIP_ALPHA,
        }),
        CapabilitySet::Order(order),
        CapabilitySet::BitmapCacheRev2(BitmapCacheRev2 {
            cache_flags: CacheFlags::ALLOW_CACHE_WAITING_LIST_FLAG,
            num_cell_caches: 0x05,
            cache_cell_info: [
                CellInfo {
                    num_entries: 600,
                    is_cache_persistent: false,
                },
                CellInfo {
                    num_entries: 600,
                    is_cache_persistent: false,
                },
                CellInfo {
                    num_entries: 2048,
                    is_cache_persistent: false,
                },
                CellInfo {
                    num_entries: 4096,
                    is_cache_persistent: false,
                },
                CellInfo {
                    num_entries: 2048,
                    is_cache_persistent: false,
                },
            ],
        }),
        CapabilitySet::Pointer(Pointer {
            // Pointer cache should be set to non-zero value to enable client-side pointer rendering.
            color_pointer_cache_size: DEFAULT_POINTER_CACHE_SIZE,
            pointer_cache_size: DEFAULT_POINTER_CACHE_SIZE,
        }),
        CapabilitySet::Input(Input {
            // InputFlags(SCANCODES | MOUSEX | FASTPATH_INPUT | FASTPATH_INPUT_2 | TS_MOUSE_HWHEEL | TS_QOE_TIMESTAMPS)
            input_flags: InputFlags::SCANCODES
                | InputFlags::MOUSEX
                | InputFlags::FASTPATH_INPUT
                | InputFlags::FASTPATH_INPUT_2
                | InputFlags::TS_MOUSE_HWHEEL
                | InputFlags::TS_QOE_TIMESTAMPS,
            keyboard_layout: 0,
            keyboard_type: Some(KeyboardType::IbmEnhanced),
            keyboard_subtype: 0,
            keyboard_function_key: 0x0000000C,
            keyboard_ime_filename: config.ime_file_name.clone(),
        }),
        CapabilitySet::Brush(Brush {
            support_level: SupportLevel::ColorFull,
        }),
        CapabilitySet::GlyphCache(GlyphCache {
            glyph_cache: [
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 4,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 4,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 8,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 8,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 16,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 32,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 64,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 128,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 256,
                },
                CacheDefinition {
                    entries: 254,
                    max_cell_size: 256,
                },
            ],
            frag_cache: CacheDefinition {
                entries: 256,
                max_cell_size: 256,
            },
            glyph_support_level: GlyphSupportLevel::None,
        }),
        CapabilitySet::VirtualChannel(VirtualChannel {
            flags: VirtualChannelFlags::NO_COMPRESSION,
            chunk_size: Some(0x00000640), // ignored
        }),
        CapabilitySet::Sound(Sound {
            flags: SoundFlags::BEEPS,
        }),
        CapabilitySet::Share(vec![
            0x00, 0x00, // nodeID
            0x00, 0x00, // pad2octets
        ]),
        CapabilitySet::Font(Font {
            font_support_flags: FontSupportFlags::FONTSUPPORT_FONTLIST,
        }),
        CapabilitySet::Control(Control::default()),
        CapabilitySet::ColorCache(ColorCache::default()),
        CapabilitySet::WindowActivation(vec![
            0x00, 0x00, // helpKeyFlag
            0x00, 0x00, // helpKeyIndexFlag
            0x00, 0x00, // helpExtendedKeyFlag
            0x00, 0x00, // windowManagerKeyFlag
        ]),
        CapabilitySet::OffscreenBitmapCache(OffscreenBitmapCache {
            is_supported: false,
            cache_size: 0,
            cache_entries: 0,
        }),
        CapabilitySet::LargePointer(LargePointer {
            // Setting `LargePointerSupportFlags::UP_TO_384X384_PIXELS` allows server to send
            // `TS_FP_LARGEPOINTERATTRIBUTE` update messages, which are required for client-side
            // rendering of pointers bigger than 96x96 pixels.
            flags: LargePointerSupportFlags::UP_TO_384X384_PIXELS,
        }),
        CapabilitySet::SurfaceCommands(SurfaceCommands {
            flags: CmdFlags::SET_SURFACE_BITS | CmdFlags::STREAM_SURFACE_BITS | CmdFlags::FRAME_MARKER,
        }),
        CapabilitySet::BitmapCodecs(BitmapCodecs(vec![Codec {
            id: 0x03, // RemoteFX
            property: CodecProperty::RemoteFx(RemoteFxContainer::ClientContainer(RfxClientCapsContainer {
                capture_flags: CaptureFlags::empty(),
                caps_data: RfxCaps(RfxCapset(vec![RfxICap {
                    flags: RfxICapFlags::empty(),
                    entropy_bits: EntropyBits::Rlgr3,
                }])),
            })),
        }])),
        CapabilitySet::FrameAcknowledge(FrameAcknowledge {
            max_unacknowledged_frame_count: 2,
        }),
    ]);

    if !server_capability_sets
        .iter()
        .any(|c| matches!(&c, CapabilitySet::MultiFragmentUpdate(_)))
    {
        server_capability_sets.push(CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate {
            max_request_size: 8 * 1024 * 1024, // 8 MB
        }));
    }

    ClientConfirmActive {
        originator_id: SERVER_CHANNEL_ID,
        pdu: DemandActive {
            source_descriptor: "IRONRDP".to_owned(),
            capability_sets: server_capability_sets,
        },
    }
}

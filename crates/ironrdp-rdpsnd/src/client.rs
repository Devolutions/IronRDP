use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use core::time::Duration;

use ironrdp_core::{Decode as _, Encode as _, EncodeResult, ReadCursor, cast_length, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};
use tracing::{debug, error, trace, warn};

use crate::pdu::{
    self, AudioFormat, ClientAudioOutputPdu, PitchPdu, ServerAudioDatagram, ServerAudioFormatPdu, TrainingPdu,
    UdpWaveLastPdu, VolumePdu,
};
use crate::server::RdpsndSvcMessages;

/// Name of the audio output dynamic virtual channel on a reliable transport.
///
/// [MS-RDPEA] 2.1 "Transport".
///
/// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
pub const DVC_CHANNEL_NAME: &str = "AUDIO_PLAYBACK_DVC";

/// Name of the audio output dynamic virtual channel on an unreliable UDP transport.
///
/// [MS-RDPEA] 2.1 "Transport".
///
/// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
pub const LOSSY_DVC_CHANNEL_NAME: &str = "AUDIO_PLAYBACK_LOSSY_DVC";

/// Size of the signature that starts every `AUDIO_FRAGDATA` structure.
const FRAGDATA_SIGNATURE_SIZE: usize = 8;

/// Largest `AUDIO_FRAGDATA` structure, as `wTotalSize` is a 16-bit field.
const MAX_FRAGDATA_SIZE: usize = 0xFFFF;

/// Most fragments retained for one UDP sample.
///
/// Senders split samples at the path MTU, so a full 64 KiB sample spans about fifty fragments.
const MAX_FRAGMENTS: usize = 1024;

/// Monotonic clock used to measure how long the client held a wave block before confirming it.
pub type Clock = Box<dyn Fn() -> Duration + Send>;

pub trait RdpsndClientHandler: Send + core::fmt::Debug {
    fn get_flags(&self) -> pdu::AudioFormatFlags {
        pdu::AudioFormatFlags::empty()
    }

    fn get_formats(&self) -> &[AudioFormat];

    /// Chooses the formats to advertise in reply to the server's format list.
    ///
    /// The order states the client's preference. The client keeps only the returned formats the
    /// server offered ([MS-RDPEA] 2.2.2.2), and Wave PDUs index that list. The default returns
    /// [`get_formats`](Self::get_formats).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        let _ = server_formats;
        self.get_formats().to_vec()
    }

    /// Play a wave block for the given negotiated format.
    ///
    /// `format` is the entry from the client format list advertised during
    /// negotiation (MS-RDPEA `wFormatNo` indexes that list).
    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>);

    /// Time until the audio queued so far, including the last wave block, finishes playing.
    ///
    /// Queried after every [`wave`](Self::wave) call. The Wave Confirm PDU reports the server's
    /// timestamp plus the time the client held the block plus this latency, so the server can
    /// measure the full render delay ([MS-RDPEA] 3.2.5.2.1.6).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    fn playback_latency(&self) -> Duration {
        Duration::ZERO
    }

    fn set_volume(&mut self, volume: VolumePdu);

    fn set_pitch(&mut self, pitch: PitchPdu);

    fn close(&mut self);
}

#[derive(Debug)]
pub struct NoopRdpsndBackend;

impl RdpsndClientHandler for NoopRdpsndBackend {
    fn get_formats(&self) -> &[AudioFormat] {
        &[]
    }

    fn wave(&mut self, _format: &AudioFormat, _ts: u32, _data: Cow<'_, [u8]>) {}

    fn set_volume(&mut self, _volume: VolumePdu) {}

    fn set_pitch(&mut self, _pitch: PitchPdu) {}

    fn close(&mut self) {}
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum RdpsndState {
    Start,
    WaitingForTraining,
    Ready,
    /// Waiting for the bare Wave payload after a pre-v8 WaveInfo PDU.
    ExpectingWave,
    Stop,
}

/// Pending pre-v8 WaveInfo fields until the following Wave payload arrives.
#[derive(Debug, Clone)]
struct PendingWave {
    timestamp: u16,
    format_no: u16,
    block_no: u8,
    data_prefix: [u8; 4],
    /// Total audio length including the four-byte prefix (MS-RDPEA `n`).
    audio_length: u16,
}

/// Identity of a received audio sample, as carried by the PDU that completed it.
#[derive(Debug, Clone, Copy)]
struct ReceivedWave {
    format_no: u16,
    timestamp: u16,
    block_no: u8,
    audio_timestamp: u32,
}

/// Fragments of the audio sample being received over UDP.
#[derive(Debug, Default)]
struct PendingUdpWave {
    block_no: u8,
    fragments: BTreeMap<u16, Vec<u8>>,
    last: Option<UdpWaveLastPdu<'static>>,
    /// Bytes held in `fragments` and `last`.
    ///
    /// INVARIANT: `retained <= MAX_FRAGDATA_SIZE`.
    retained: usize,
}

impl PendingUdpWave {
    fn new(block_no: u8) -> Self {
        Self {
            block_no,
            ..Self::default()
        }
    }

    /// Accounts for replacing `replaced` retained bytes with `added` ones, or returns `false` when
    /// the sample would exceed `MAX_FRAGDATA_SIZE`.
    fn try_retain(&mut self, replaced: usize, added: usize) -> bool {
        let retained = self.retained - replaced + added;
        if MAX_FRAGDATA_SIZE < retained {
            return false;
        }
        self.retained = retained;
        true
    }

    /// Stores a fragment, or returns `false` when the sample would exceed the protocol's bounds.
    fn insert(&mut self, frag_no: u16, data: &[u8]) -> bool {
        let replaced = self.fragments.get(&frag_no).map(Vec::len);
        if replaced.is_none() && MAX_FRAGMENTS <= self.fragments.len() {
            return false;
        }
        if !self.try_retain(replaced.unwrap_or(0), data.len()) {
            return false;
        }
        self.fragments.insert(frag_no, data.to_vec());
        true
    }

    /// Stores the final fragment, or returns `false` when the sample would exceed the protocol's bounds.
    fn set_last(&mut self, last: UdpWaveLastPdu<'_>) -> bool {
        let replaced = self.last.as_ref().map_or(0, |last| last.data.len());
        if !self.try_retain(replaced, last.data.len()) {
            return false;
        }
        self.last = Some(UdpWaveLastPdu {
            data: Cow::Owned(last.data.into_owned()),
            ..last
        });
        true
    }
}

/// Returns whether block number `block_no` comes after `reference`, with 8-bit wraparound.
fn is_later_block(block_no: u8, reference: u8) -> bool {
    let distance = i8::from_ne_bytes([block_no.wrapping_sub(reference)]);
    0 < distance
}

/// Computes the `AUDIO_FRAGDATA` signature of `audio` ([MS-RDPEA] 3.3.5.2.1.3): the first eight
/// bytes of the SHA-1 hash of the Crypt Key seed, the block number, three zero bytes and the data.
///
/// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
fn fragdata_signature(seed: &[u8; 32], block_no: u8, audio: &[u8]) -> [u8; FRAGDATA_SIGNATURE_SIZE] {
    use sha1::Digest as _;

    let hash = sha1::Sha1::new()
        .chain_update(seed)
        .chain_update([block_no, 0, 0, 0])
        .chain_update(audio)
        .finalize();
    let mut signature = [0; FRAGDATA_SIGNATURE_SIZE];
    signature.copy_from_slice(&hash[..FRAGDATA_SIGNATURE_SIZE]);
    signature
}

/// Audio output client protocol state, independent of the channel that carries it.
struct AudioOutputClient {
    handler: Box<dyn RdpsndClientHandler>,
    quality_mode: pdu::QualityMode,
    state: RdpsndState,
    server_format: Option<ServerAudioFormatPdu>,
    /// Formats advertised to the server, in wire order.
    ///
    /// Wave/Wave2 `wFormatNo` indexes this list (MS-RDPEA).
    client_formats: Vec<AudioFormat>,
    pending_wave: Option<PendingWave>,
    pending_udp_wave: Option<PendingUdpWave>,
    /// Block number of the last UDP sample completed, so stale fragments are ignored.
    last_udp_block: Option<u8>,
    /// Seed from the Crypt Key PDU, all zeros when the server sends none ([MS-RDPEA] 3.3.5.2.1.3).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    crypt_key_seed: [u8; 32],
    clock: Option<Clock>,
    /// Port advertised in `wDGramPort` for audio data over UDP, or zero.
    datagram_port: u16,
    /// Name of the channel that carries the negotiation, for diagnostics.
    channel_name: &'static str,
}

impl core::fmt::Debug for AudioOutputClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AudioOutputClient")
            .field("handler", &self.handler)
            .field("quality_mode", &self.quality_mode)
            .field("state", &self.state)
            .field("client_formats", &self.client_formats)
            .field("datagram_port", &self.datagram_port)
            .field("channel_name", &self.channel_name)
            .finish_non_exhaustive()
    }
}

impl AudioOutputClient {
    fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        Self {
            handler,
            quality_mode: pdu::QualityMode::High,
            state: RdpsndState::Start,
            server_format: None,
            client_formats: Vec::new(),
            pending_wave: None,
            pending_udp_wave: None,
            last_udp_block: None,
            crypt_key_seed: [0; 32],
            clock: None,
            datagram_port: 0,
            channel_name: "RDPSND",
        }
    }

    fn now(&self) -> Option<Duration> {
        self.clock.as_ref().map(|clock| clock())
    }

    fn reset(&mut self) {
        self.handler.close();
        self.state = RdpsndState::Start;
        self.server_format = None;
        self.client_formats.clear();
        self.pending_wave = None;
        self.pending_udp_wave = None;
        self.last_udp_block = None;
        self.crypt_key_seed = [0; 32];
    }

    fn version(&self) -> PduResult<pdu::Version> {
        let server_format = self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no version"))?;

        Ok(server_format.version)
    }

    fn client_formats_pdu(&mut self) -> PduResult<ClientAudioOutputPdu> {
        let server_formats = &self
            .server_format
            .as_ref()
            .ok_or_else(|| pdu_other_err!("invalid state - no server format"))?
            .formats;

        // Windows is confused by formats it did not offer (e.g.: Opus), so the handler only
        // picks among the server's formats; wFormatNo indexes the list kept here.
        let formats = pdu::retain_offered(self.handler.choose_formats(server_formats), server_formats);
        debug!(
            channel = self.channel_name,
            ?formats,
            "Advertising audio output formats"
        );

        self.client_formats = formats.clone();

        Ok(ClientAudioOutputPdu::AudioFormat(pdu::ClientAudioFormatPdu {
            version: self.version()?,
            flags: self.handler.get_flags() | pdu::AudioFormatFlags::ALIVE,
            formats,
            volume_left: 0xFFFF,
            volume_right: 0xFFFF,
            pitch: 0x00010000,
            dgram_port: self.datagram_port,
        }))
    }

    fn training_confirm(pdu: &TrainingPdu) -> PduResult<ClientAudioOutputPdu> {
        let pack_size: EncodeResult<_> = cast_length!("wPackSize", pdu.data.len());
        let pack_size = pack_size.map_err(|e| encode_err!(e))?;
        Ok(ClientAudioOutputPdu::TrainingConfirm(pdu::TrainingConfirmPdu {
            timestamp: pdu.timestamp,
            pack_size,
        }))
    }

    /// Hands one complete audio sample to the handler.
    fn play(&mut self, format_no: u16, audio_timestamp: u32, data: Cow<'_, [u8]>) {
        match self.client_formats.get(usize::from(format_no)) {
            Some(format) => self.handler.wave(format, audio_timestamp, data),
            None => warn!(
                format_no,
                n_formats = self.client_formats.len(),
                "Ignoring wave with out-of-range format_no"
            ),
        }
    }

    /// Plays one complete audio sample, when there is one, and builds its Wave Confirm PDU.
    ///
    /// `arrival` is when the last byte of the sample was received.
    fn play_and_confirm(
        &mut self,
        arrival: Option<Duration>,
        wave: ReceivedWave,
        data: Option<Cow<'_, [u8]>>,
    ) -> Vec<ClientAudioOutputPdu> {
        let ReceivedWave {
            format_no,
            timestamp,
            block_no,
            audio_timestamp,
        } = wave;
        if let Some(data) = data {
            self.play(format_no, audio_timestamp, data);
        }

        // MS-RDPEA 3.2.5.2.1.6: wTimeStamp is the server's timestamp plus the time between receiving
        // the sample and sending this confirmation. The handler latency extends that to when the
        // sample finishes playing, as the confirmation is due once the sample is emitted.
        let held = match (arrival, self.now()) {
            (Some(arrival), Some(now)) => now.saturating_sub(arrival),
            _ => Duration::ZERO,
        };
        let delay = held.saturating_add(self.handler.playback_latency());
        let delay_ms = u16::try_from(delay.as_millis() % 0x1_0000).unwrap_or(u16::MAX);
        trace!(block_no, timestamp, delay_ms, "Confirming wave");

        vec![ClientAudioOutputPdu::WaveConfirm(pdu::WaveConfirmPdu {
            timestamp: timestamp.wrapping_add(delay_ms),
            block_no,
        })]
    }

    fn begin_format_negotiation(&mut self, af: ServerAudioFormatPdu) -> PduResult<Vec<ClientAudioOutputPdu>> {
        self.handler.close();
        self.server_format = Some(af);
        self.client_formats.clear();
        self.pending_wave = None;
        self.pending_udp_wave = None;
        self.state = RdpsndState::WaitingForTraining;
        let mut msgs = vec![self.client_formats_pdu()?];
        if self.version()? >= pdu::Version::V6 {
            msgs.push(ClientAudioOutputPdu::QualityMode(pdu::QualityModePdu {
                quality_mode: self.quality_mode,
            }));
        }
        Ok(msgs)
    }

    /// Complete a pre-v8 Wave transfer from WaveInfo + bare Wave payload bytes.
    fn finish_pending_wave(
        &mut self,
        arrival: Option<Duration>,
        wave_payload: &[u8],
    ) -> PduResult<Vec<ClientAudioOutputPdu>> {
        let Some(pending) = self.pending_wave.take() else {
            warn!("Received Wave payload without a pending WaveInfo");
            self.state = RdpsndState::Ready;
            return Ok(vec![]);
        };

        self.state = RdpsndState::Ready;

        // Wave payload is bPad[4] + remaining audio after the WaveInfo Data prefix.
        // Total wire length equals WaveInfo audio_length (`n`).
        let expected_len = usize::from(pending.audio_length);
        let data = if expected_len <= wave_payload.len() {
            let remaining = &wave_payload[4..expected_len];
            let mut data = Vec::with_capacity(expected_len);
            data.extend_from_slice(&pending.data_prefix);
            data.extend_from_slice(remaining);
            Some(data.into())
        } else {
            // MS-RDPEA §3.2.5.2.1.6: still confirm so servers can advance latency accounting.
            warn!(
                got = wave_payload.len(),
                expected = expected_len,
                "Wave payload shorter than WaveInfo audio length; confirming without playback"
            );
            None
        };

        Ok(self.play_and_confirm(
            arrival,
            ReceivedWave {
                format_no: pending.format_no,
                timestamp: pending.timestamp,
                block_no: pending.block_no,
                audio_timestamp: u32::from(pending.timestamp),
            },
            data,
        ))
    }

    /// Processes one PDU received on the static channel or on `AUDIO_PLAYBACK_DVC`.
    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<ClientAudioOutputPdu>> {
        let arrival = self.now();

        // Pre-v8 Wave data has no RDPSND header (MS-RDPEA §2.2.3.4).
        if self.state == RdpsndState::ExpectingWave {
            debug!(len = payload.len(), "Completing pending WaveInfo with Wave payload");
            return self.finish_pending_wave(arrival, payload);
        }

        let pdu = match pdu::ServerAudioOutputPdu::decode(&mut ReadCursor::new(payload)) {
            Ok(pdu) => pdu,
            Err(error) => {
                error!(?error, "Ignoring malformed RDPSND PDU");
                return Ok(vec![]);
            }
        };

        debug!(?pdu, ?self.state);
        let msg = match self.state {
            RdpsndState::Start => {
                let pdu::ServerAudioOutputPdu::AudioFormat(af) = pdu else {
                    error!("Invalid pdu");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.begin_format_negotiation(af)?
            }
            RdpsndState::WaitingForTraining => {
                let pdu::ServerAudioOutputPdu::Training(pdu) = pdu else {
                    error!("Invalid PDU");
                    self.state = RdpsndState::Stop;
                    return Ok(vec![]);
                };
                self.state = RdpsndState::Ready;
                vec![Self::training_confirm(&pdu)?]
            }
            RdpsndState::Ready => {
                match pdu {
                    pdu::ServerAudioOutputPdu::Wave(pdu) => {
                        // MS-RDPEA §2.2.3.3: WaveInfo only; next SVC message is bare Wave (§2.2.3.4).
                        // Some test stacks may concatenate Wave after WaveInfo in one buffer —
                        // finish immediately when trailing bytes are present.
                        self.pending_wave = Some(PendingWave {
                            timestamp: pdu.timestamp,
                            format_no: pdu.format_no,
                            block_no: pdu.block_no,
                            data_prefix: pdu.data_prefix,
                            audio_length: pdu.audio_length,
                        });
                        self.state = RdpsndState::ExpectingWave;

                        // SNDPROLOG (4) + WaveInfo body; trailing bytes mean a concatenated Wave payload.
                        let header_and_info = 4usize.checked_add(pdu.size()).expect("header_and_info never overflows");
                        if payload.len() > header_and_info {
                            return self.finish_pending_wave(arrival, &payload[header_and_info..]);
                        }
                        return Ok(vec![]);
                    }
                    pdu::ServerAudioOutputPdu::Wave2(pdu) => {
                        return Ok(self.play_and_confirm(
                            arrival,
                            ReceivedWave {
                                format_no: pdu.format_no,
                                timestamp: pdu.timestamp,
                                block_no: pdu.block_no,
                                audio_timestamp: pdu.audio_timestamp,
                            },
                            Some(pdu.data),
                        ));
                    }
                    pdu::ServerAudioOutputPdu::Volume(pdu) => {
                        self.handler.set_volume(pdu);
                    }
                    pdu::ServerAudioOutputPdu::Pitch(pdu) => {
                        self.handler.set_pitch(pdu);
                    }
                    pdu::ServerAudioOutputPdu::Close => {
                        self.handler.close();
                    }
                    pdu::ServerAudioOutputPdu::Training(pdu) => return Ok(vec![Self::training_confirm(&pdu)?]),
                    pdu::ServerAudioOutputPdu::AudioFormat(af) => {
                        return self.begin_format_negotiation(af);
                    }
                    pdu::ServerAudioOutputPdu::CryptKey(pdu) => {
                        // The seed authenticates audio received over UDP.
                        self.crypt_key_seed = pdu.seed;
                    }
                    // Unsupported optional PDU: keep the channel alive.
                    pdu::ServerAudioOutputPdu::WaveEncrypt(_) => {
                        warn!(?pdu, "Ignoring unsupported RDPSND PDU");
                    }
                }
                vec![]
            }
            state => {
                error!(?state, "Invalid state");
                vec![]
            }
        };

        Ok(msg)
    }

    /// Processes one PDU received on `AUDIO_PLAYBACK_LOSSY_DVC`.
    ///
    /// Windows negotiates on `AUDIO_PLAYBACK_DVC` and sends only Wave2 PDUs on this channel, so
    /// anything else, or wave data arriving before the negotiation completed, is ignored rather than
    /// allowed to disturb the shared negotiation. The waves are not confirmed ([MS-RDPEA]
    /// 3.2.5.2.1.6).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    fn process_lossy(&mut self, payload: &[u8]) -> PduResult<Vec<ClientAudioOutputPdu>> {
        if self.state != RdpsndState::Ready {
            debug!(state = ?self.state, "Ignoring lossy-channel data outside the Ready state");
            return Ok(vec![]);
        }
        match pdu::ServerAudioOutputPdu::decode(&mut ReadCursor::new(payload)) {
            Ok(pdu::ServerAudioOutputPdu::Wave2(pdu)) => {
                self.play(pdu.format_no, pdu.audio_timestamp, pdu.data);
                Ok(vec![])
            }
            Ok(pdu) => {
                debug!(?pdu, "Ignoring a non-wave PDU on the lossy channel");
                Ok(vec![])
            }
            Err(error) => {
                error!(?error, "Ignoring malformed RDPSND PDU");
                Ok(vec![])
            }
        }
    }

    fn process_datagram(&mut self, datagram: &[u8]) -> PduResult<Vec<ClientAudioOutputPdu>> {
        let arrival = self.now();

        let pdu = match ServerAudioDatagram::decode(&mut ReadCursor::new(datagram)) {
            Ok(pdu) => pdu,
            Err(error) => {
                error!(?error, "Ignoring malformed RDPSND datagram");
                return Ok(vec![]);
            }
        };

        match pdu {
            ServerAudioDatagram::Prolog(pdu::ServerAudioOutputPdu::Training(pdu)) => {
                Ok(vec![Self::training_confirm(&pdu)?])
            }
            ServerAudioDatagram::Prolog(pdu) => {
                // Wave Encrypt data needs the Crypt Key based decryption, which is not implemented.
                warn!(?pdu, "Ignoring unsupported RDPSND datagram");
                Ok(vec![])
            }
            ServerAudioDatagram::UdpWave(_) | ServerAudioDatagram::UdpWaveLast(_)
                if self.version().map_or(true, |version| version < pdu::Version::V5) =>
            {
                // MS-RDPEA 2.2.3.6: UDP Wave PDUs exist only when both sides are at version 5 or later.
                debug!("Ignoring a UDP wave datagram before a version 5 negotiation");
                Ok(vec![])
            }
            ServerAudioDatagram::UdpWave(fragment) => {
                let Some(pending) = self.udp_wave_for(fragment.block_no) else {
                    return Ok(vec![]);
                };
                if !pending.insert(fragment.frag_no, &fragment.data) {
                    warn!(
                        block_no = fragment.block_no,
                        "UDP wave exceeds its size bounds; dropping sample"
                    );
                    self.pending_udp_wave = None;
                    return Ok(vec![]);
                }
                Ok(self.try_finish_udp_wave(arrival))
            }
            ServerAudioDatagram::UdpWaveLast(last) => {
                let block_no = last.block_no;
                let Some(pending) = self.udp_wave_for(block_no) else {
                    return Ok(vec![]);
                };
                if !pending.set_last(last) {
                    warn!(block_no, "UDP wave exceeds its size bounds; dropping sample");
                    self.pending_udp_wave = None;
                    return Ok(vec![]);
                }
                Ok(self.try_finish_udp_wave(arrival))
            }
        }
    }

    /// Returns the reassembly state for `block_no`, or `None` for a fragment of an older block.
    ///
    /// MS-RDPEA 3.2.5.2.1.1: a sample for a later block abandons the incomplete one, and fragments of
    /// earlier blocks are discarded.
    fn udp_wave_for(&mut self, block_no: u8) -> Option<&mut PendingUdpWave> {
        let stale = match (&self.pending_udp_wave, self.last_udp_block) {
            (Some(pending), _) if pending.block_no == block_no => false,
            (Some(pending), _) => !is_later_block(block_no, pending.block_no),
            (None, Some(completed)) => !is_later_block(block_no, completed),
            (None, None) => false,
        };
        if stale {
            trace!(block_no, "Ignoring a fragment of an earlier UDP wave");
            return None;
        }
        let pending = self
            .pending_udp_wave
            .get_or_insert_with(|| PendingUdpWave::new(block_no));
        if pending.block_no != block_no {
            trace!(abandoned = pending.block_no, block_no, "Abandoning incomplete UDP wave");
            *pending = PendingUdpWave::new(block_no);
        }
        Some(pending)
    }

    fn try_finish_udp_wave(&mut self, arrival: Option<Duration>) -> Vec<ClientAudioOutputPdu> {
        let Some(pending) = self.pending_udp_wave.as_ref() else {
            return Vec::new();
        };
        let Some(last) = pending.last.as_ref() else {
            return Vec::new();
        };

        // Fragments are numbered from zero without gaps (MS-RDPEA 3.3.5.2.1.4).
        let contiguous = pending
            .fragments
            .keys()
            .enumerate()
            .all(|(expected, frag_no)| usize::from(*frag_no) == expected);
        if !contiguous || pending.retained < usize::from(last.total_size) {
            return Vec::new();
        }

        let pending = self.pending_udp_wave.take().expect("checked above");
        self.last_udp_block = Some(pending.block_no);
        let last = pending.last.expect("checked above");
        if pending.retained != usize::from(last.total_size) {
            warn!(
                size = pending.retained,
                total_size = last.total_size,
                "UDP wave size mismatch; dropping sample"
            );
            return Vec::new();
        }

        let mut fragdata = Vec::with_capacity(pending.retained);
        for fragment in pending.fragments.into_values() {
            fragdata.extend_from_slice(&fragment);
        }
        fragdata.extend_from_slice(&last.data);
        let Some((signature, audio)) = fragdata.split_at_checked(FRAGDATA_SIGNATURE_SIZE) else {
            warn!(
                block_no = last.block_no,
                "UDP wave is shorter than its signature; dropping sample"
            );
            return Vec::new();
        };
        // The UDP socket is not authenticated like the virtual channel, so every sample must carry
        // the signature derived from the Crypt Key seed.
        if *signature != fragdata_signature(&self.crypt_key_seed, last.block_no, audio) {
            warn!(block_no = last.block_no, "UDP wave signature mismatch; dropping sample");
            return Vec::new();
        }

        self.play_and_confirm(
            arrival,
            ReceivedWave {
                format_no: last.format_no,
                timestamp: last.timestamp,
                block_no: last.block_no,
                audio_timestamp: u32::from(last.timestamp),
            },
            Some(Cow::Owned(audio.to_vec())),
        )
    }
}

impl Drop for AudioOutputClient {
    fn drop(&mut self) {
        self.handler.close();
    }
}

/// Required for rdpdr to work: [\[MS-RDPEFS\] Appendix A<1>]
///
/// [\[MS-RDPEFS\] Appendix A<1>]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/fd28bfd9-dae2-4a78-abe1-b4efa208b7aa#Appendix_A_1
#[derive(Debug)]
pub struct Rdpsnd {
    inner: AudioOutputClient,
}

impl Rdpsnd {
    pub const NAME: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        Self {
            inner: AudioOutputClient::new(handler),
        }
    }

    #[must_use]
    pub fn with_quality_mode(mut self, quality_mode: pdu::QualityMode) -> Self {
        self.inner.quality_mode = quality_mode;
        self
    }

    /// Measures how long wave blocks are held before being confirmed.
    ///
    /// Without a clock, Wave Confirm PDUs report only the handler's playback latency.
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.inner.clock = Some(clock);
        self
    }

    /// Advertises `port` in `wDGramPort` so the server may send audio data over UDP.
    ///
    /// The caller owns the UDP socket and passes every received datagram to
    /// [`process_datagram`](Self::process_datagram). Windows servers from Windows Vista on
    /// ignore this port and use the dynamic virtual channels instead.
    ///
    /// UDP Wave samples are played once their signature checks out. Wave Encrypt PDUs are ignored:
    /// MS-RDPEA 3.3.5.2.1.3 derives their RC4 key from a hash that includes the data being
    /// encrypted, which a receiver cannot compute before decrypting.
    #[must_use]
    pub fn with_datagram_port(mut self, port: u16) -> Self {
        self.inner.datagram_port = port;
        self
    }

    pub fn get_format(&self, format_no: u16) -> PduResult<&AudioFormat> {
        self.inner
            .client_formats
            .get(usize::from(format_no))
            .ok_or_else(|| pdu_other_err!("invalid format"))
    }

    pub fn version(&self) -> PduResult<pdu::Version> {
        self.inner.version()
    }

    pub fn client_formats(&mut self) -> PduResult<RdpsndSvcMessages> {
        Ok(RdpsndSvcMessages::new(vec![self.inner.client_formats_pdu()?.into()]))
    }

    pub fn quality_mode(&mut self) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::QualityModePdu {
            quality_mode: self.inner.quality_mode,
        };
        Ok(RdpsndSvcMessages::new(vec![
            ClientAudioOutputPdu::QualityMode(pdu).into(),
        ]))
    }

    pub fn training_confirm(&mut self, pdu: &TrainingPdu) -> PduResult<RdpsndSvcMessages> {
        Ok(RdpsndSvcMessages::new(vec![
            AudioOutputClient::training_confirm(pdu)?.into(),
        ]))
    }

    pub fn wave_confirm(&mut self, timestamp: u16, block_no: u8) -> PduResult<RdpsndSvcMessages> {
        let pdu = pdu::WaveConfirmPdu { timestamp, block_no };
        Ok(RdpsndSvcMessages::new(vec![
            ClientAudioOutputPdu::WaveConfirm(pdu).into(),
        ]))
    }

    /// Processes one datagram received on the port advertised with
    /// [`with_datagram_port`](Self::with_datagram_port).
    ///
    /// Returns the replies, which the caller sends back over UDP ([MS-RDPEA] 3.2.5.2.1.6).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    pub fn process_datagram(&mut self, datagram: &[u8]) -> PduResult<Vec<ClientAudioOutputPdu>> {
        self.inner.process_datagram(datagram)
    }
}

impl_as_any!(Rdpsnd);

impl SvcProcessor for Rdpsnd {
    fn channel_name(&self) -> ChannelName {
        Self::NAME
    }

    fn compression_condition(&self) -> CompressionCondition {
        CompressionCondition::Never
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        Ok(self.inner.process(payload)?.into_iter().map(SvcMessage::from).collect())
    }
}

impl SvcClientProcessor for Rdpsnd {}

/// Audio output state shared by the dynamic audio channels of one connection.
///
/// Windows 8 and later try the `AUDIO_PLAYBACK_DVC` and `AUDIO_PLAYBACK_LOSSY_DVC` channels before
/// the static `RDPSND` channel, and open them on the multitransport UDP tunnel when one is
/// available ([MS-RDPEA] 3.1.3). The formats are negotiated on `AUDIO_PLAYBACK_DVC`; Windows then
/// sends AAC wave data on `AUDIO_PLAYBACK_LOSSY_DVC`, indexing the formats negotiated on the other
/// channel. Both channels therefore drive this one state, and a new negotiation starts whenever
/// `AUDIO_PLAYBACK_DVC` is opened.
///
/// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
#[derive(Debug, Clone)]
pub struct RdpsndDvcState {
    inner: Arc<Mutex<AudioOutputClient>>,
}

impl RdpsndDvcState {
    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        let mut inner = AudioOutputClient::new(handler);
        inner.channel_name = DVC_CHANNEL_NAME;
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    #[must_use]
    pub fn with_quality_mode(self, quality_mode: pdu::QualityMode) -> Self {
        self.lock().quality_mode = quality_mode;
        self
    }

    /// Measures how long wave blocks are held before being confirmed.
    #[must_use]
    pub fn with_clock(self, clock: Clock) -> Self {
        self.lock().clock = Some(clock);
        self
    }

    /// Returns a processor for `AUDIO_PLAYBACK_DVC`.
    pub fn channel(&self) -> RdpsndDvc {
        RdpsndDvc {
            state: self.clone(),
            lossy: false,
        }
    }

    /// Returns a processor for `AUDIO_PLAYBACK_LOSSY_DVC`.
    ///
    /// Wave blocks received on this channel are not confirmed ([MS-RDPEA] 3.2.5.2.1.6).
    ///
    /// [MS-RDPEA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/bea2d5cf-e3b9-4419-92e5-0e074ff9bc5b
    pub fn lossy_channel(&self) -> RdpsndDvc {
        RdpsndDvc {
            state: self.clone(),
            lossy: true,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AudioOutputClient> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Audio output client on the `AUDIO_PLAYBACK_DVC` or `AUDIO_PLAYBACK_LOSSY_DVC` dynamic channel.
///
/// Created from an [`RdpsndDvcState`], which the two channels share.
#[derive(Debug)]
pub struct RdpsndDvc {
    state: RdpsndDvcState,
    lossy: bool,
}

impl RdpsndDvc {
    /// Client for `AUDIO_PLAYBACK_DVC` with a state of its own.
    pub fn new(handler: Box<dyn RdpsndClientHandler>) -> Self {
        RdpsndDvcState::new(handler).channel()
    }

    fn name(&self) -> &'static str {
        if self.lossy {
            LOSSY_DVC_CHANNEL_NAME
        } else {
            DVC_CHANNEL_NAME
        }
    }
}

impl_as_any!(RdpsndDvc);

impl DvcProcessor for RdpsndDvc {
    fn channel_name(&self) -> &str {
        self.name()
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(channel_id, channel_name = self.name(), "Audio output channel started");
        if !self.lossy {
            // The server opens the exchange with its format list.
            self.state.lock().reset();
        }
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let mut state = self.state.lock();
        let replies = if self.lossy {
            state.process_lossy(payload)?
        } else {
            state.process(payload)?
        };
        Ok(replies.into_iter().map(|pdu| -> DvcMessage { Box::new(pdu) }).collect())
    }

    fn close(&mut self, channel_id: u32) {
        debug!(channel_id, channel_name = self.name(), "Audio output channel closed");
        if !self.lossy {
            self.state.lock().reset();
        }
    }
}

impl DvcClientProcessor for RdpsndDvc {}

use core::cell::RefCell;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::rc::Rc;

use ironrdp::rdpsnd::client::RdpsndClientHandler;
use ironrdp::rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu, WaveFormat};
use wasm_bindgen::{closure::Closure, JsCast as _};
use web_sys::{AudioBuffer, AudioContext, Event, GainNode};

use crate::error::IronError;

/// Web Audio API backend for RDP audio playback
///
/// Features:
/// - PCM 16-bit audio format support
/// - Sample rate conversion for compatibility with any RDP server
/// - Channel count conversion (mono/stereo)
/// - User gesture activation handling for Web Audio policy
#[derive(Debug)]
pub(crate) struct WebAudioBackend {
    audio_context: AudioContext,
    gain_node: GainNode,
    context_sample_rate: f32,
    supported_formats: Vec<AudioFormat>,
    #[expect(dead_code)] // Will be used for future audio optimization
    browser_capabilities: BrowserAudioCapabilities,
    volume: f32,
    pitch: f32,
    is_active: bool,
    context_ready: Rc<RefCell<bool>>,
    pending_audio_data: VecDeque<(Vec<u8>, AudioFormat)>,
    audio_queue: AudioQueue,
}

#[derive(Debug)]
struct AudioQueue {
    context: AudioContext,
    current_time: f64,
}

#[derive(Debug, Clone)]
struct BrowserAudioCapabilities {
    supported_sample_rates: Vec<u32>,
    #[expect(dead_code)] // Reserved for future bandwidth optimization
    max_sample_rate: f32,
    #[expect(dead_code)] // Reserved for future quality fallback
    min_sample_rate: f32,
}

impl WebAudioBackend {
    /// Create a new WebAudioBackend with the specified sample rate
    pub(crate) fn new(sample_rate: Option<f32>) -> Result<Self, IronError> {
        let audio_context = AudioContext::new().map_err(|e| {
            anyhow::Error::msg(format!(
                "failed to create Web Audio API context (check browser support and user gesture requirement): {e:?}"
            ))
        })?;

        let gain_node = audio_context
            .create_gain()
            .map_err(|e| anyhow::Error::msg(format!("failed to create Web Audio gain node: {e:?}")))?;

        // Connect gain node to destination
        gain_node
            .connect_with_audio_node(&audio_context.destination())
            .map_err(|e| anyhow::Error::msg(format!("failed to connect gain node to audio destination: {e:?}")))?;

        let context_sample_rate = audio_context.sample_rate();
        let requested_rate = sample_rate.unwrap_or(context_sample_rate);

        let browser_capabilities = Self::detect_browser_capabilities(&audio_context);
        let supported_formats = Self::create_supported_formats(requested_rate, &browser_capabilities);

        let audio_queue = AudioQueue {
            context: audio_context.clone(),
            current_time: audio_context.current_time(),
        };

        let context_ready = Rc::new(RefCell::new(false));

        // Set up user gesture activation
        let backend = Self {
            audio_context: audio_context.clone(),
            gain_node,
            context_sample_rate,
            supported_formats,
            browser_capabilities,
            volume: 1.0,
            pitch: 1.0,
            is_active: true, // Mark as active by default
            context_ready: Rc::clone(&context_ready),
            pending_audio_data: VecDeque::new(),
            audio_queue,
        };

        info!(
            "WebAudioBackend initialized: {} supported formats, context sample rate: {}Hz",
            backend.supported_formats.len(),
            backend.context_sample_rate
        );
        for (i, format) in backend.supported_formats.iter().enumerate() {
            info!(
                "Audio format {}: {:?} {}Hz {}ch ({}bps)",
                i, format.format, format.n_samples_per_sec, format.n_channels, format.bits_per_sample
            );
        }

        // Set up one-time user gesture listener on document
        Self::setup_user_gesture_listener(audio_context, context_ready);

        Ok(backend)
    }

    /// Detect browser audio capabilities
    /// Simplified to focus on sample rate support since we only use PCM
    fn detect_browser_capabilities(audio_context: &AudioContext) -> BrowserAudioCapabilities {
        let context_sample_rate = audio_context.sample_rate();

        // Common sample rates that browsers and RDP servers typically support
        let mut supported_sample_rates = vec![22050, 44100, 48000];

        // Add the context's native sample rate if not already included
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let native_rate = context_sample_rate.round() as u32;
        if !supported_sample_rates.contains(&native_rate) {
            supported_sample_rates.push(native_rate);
            supported_sample_rates.sort_unstable();
        }

        BrowserAudioCapabilities {
            supported_sample_rates,
            max_sample_rate: context_sample_rate,
            min_sample_rate: 8000.0, // Minimum for voice quality
        }
    }

    /// Create the list of audio formats supported by the web backend
    /// Only advertises PCM formats that we can actually decode and that RDP servers commonly support
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn create_supported_formats(preferred_rate: f32, capabilities: &BrowserAudioCapabilities) -> Vec<AudioFormat> {
        let mut formats = Vec::new();

        // Priority order: preferred rate first, then other supported rates
        let mut rates_to_add = vec![preferred_rate.round() as u32];
        for &rate in &capabilities.supported_sample_rates {
            if rate != preferred_rate.round() as u32 {
                rates_to_add.push(rate);
            }
        }

        // Only add PCM formats - these are what we can actually decode
        // and what 95%+ of RDP servers actually support
        for &rate in &rates_to_add {
            // PCM 16-bit stereo (primary format)
            formats.push(AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: rate,
                n_avg_bytes_per_sec: rate * 2 * 2,
                n_block_align: 4,
                bits_per_sample: 16,
                data: Some(Vec::new()),
            });

            // PCM 16-bit mono (fallback)
            formats.push(AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 1,
                n_samples_per_sec: rate,
                n_avg_bytes_per_sec: rate * 2,
                n_block_align: 2,
                bits_per_sample: 16,
                data: Some(Vec::new()),
            });
        }

        info!(
            "Created {} PCM audio formats for sample rates: {:?}",
            formats.len(),
            rates_to_add
        );

        formats
    }

    /// Convert sample rate using linear interpolation
    ///
    /// Linear interpolation is sufficient for RDP audio quality requirements.
    /// Future enhancement: Consider higher-quality resampling for specialized use cases.
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn convert_sample_rate(input_samples: &[f32], from_rate: u32, to_rate: u32, channels: u16) -> Vec<f32> {
        if from_rate == to_rate {
            return input_samples.to_vec();
        }

        let ratio = f64::from(from_rate) / f64::from(to_rate);
        let input_frames = input_samples.len() / channels as usize;
        let output_frames = (input_frames as f64 / ratio).round() as usize;
        let mut output = Vec::with_capacity(output_frames * channels as usize);

        for output_frame in 0..output_frames {
            let input_pos = output_frame as f64 * ratio;
            let input_frame = input_pos.floor() as usize;
            let frac = input_pos.fract() as f32;

            for channel in 0..channels as usize {
                let sample1_idx = input_frame * channels as usize + channel;
                let sample2_idx = ((input_frame + 1).min(input_frames - 1)) * channels as usize + channel;

                let sample1 = input_samples.get(sample1_idx).copied().unwrap_or(0.0);
                let sample2 = input_samples.get(sample2_idx).copied().unwrap_or(sample1);

                // Linear interpolation
                let interpolated = sample1 + (sample2 - sample1) * frac;
                output.push(interpolated);
            }
        }

        output
    }

    /// Convert mono to stereo by duplicating the channel
    fn convert_mono_to_stereo(samples: &[f32]) -> Vec<f32> {
        let mut stereo = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            stereo.push(sample);
            stereo.push(sample);
        }
        stereo
    }

    /// Convert stereo to mono by averaging channels
    fn convert_stereo_to_mono(samples: &[f32]) -> Vec<f32> {
        let mut mono = Vec::with_capacity(samples.len() / 2);
        for chunk in samples.chunks_exact(2) {
            let avg = (chunk[0] + chunk[1]) / 2.0;
            mono.push(avg);
        }
        mono
    }

    /// Convert PCM 16-bit signed integer data to 32-bit float samples
    fn convert_pcm_to_float(pcm_data: &[u8], format: &AudioFormat) -> Result<Vec<f32>, IronError> {
        // Reasonable upper bound for audio buffer size (10 seconds of 48kHz stereo audio)
        const MAX_REASONABLE_AUDIO_BUFFER_SIZE: usize = 48000 * 2 * 2 * 10; // ~1.9MB

        if pcm_data.len() > MAX_REASONABLE_AUDIO_BUFFER_SIZE {
            return Err(anyhow::Error::msg(format!(
                "audio buffer too large ({} bytes), possible malformed data (max: {} bytes)",
                pcm_data.len(),
                MAX_REASONABLE_AUDIO_BUFFER_SIZE
            ))
            .into());
        }

        if format.bits_per_sample != 16 {
            return Err(anyhow::Error::msg(format!(
                "unsupported bits per sample: {} (only 16-bit supported)",
                format.bits_per_sample
            ))
            .into());
        }

        if pcm_data.len() % 2 != 0 {
            return Err(anyhow::Error::msg("PCM data length must be even for 16-bit samples").into());
        }

        let mut float_samples = Vec::with_capacity(pcm_data.len() / 2);

        // Convert 16-bit signed PCM to float32 samples
        for chunk in pcm_data.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            let float_sample = f32::from(sample) / 32768.0; // Convert to range [-1.0, 1.0]
            float_samples.push(float_sample);
        }

        Ok(float_samples)
    }

    /// Create an AudioBuffer from audio data
    /// Only handles PCM since that's what we advertise and what RDP servers typically send
    fn create_audio_buffer(&self, audio_data: &[u8], format: &AudioFormat) -> Result<AudioBuffer, IronError> {
        match format.format {
            WaveFormat::PCM => self.create_pcm_buffer(audio_data, format),
            _ => {
                // This should not happen since we only advertise PCM formats
                error!(
                    "Received unsupported audio format: {:?} - only PCM is supported",
                    format.format
                );
                Err(anyhow::Error::msg(format!("Unsupported audio format: {:?}", format.format)).into())
            }
        }
    }

    /// Create an AudioBuffer from PCM data with sample rate conversion
    fn create_pcm_buffer(&self, pcm_data: &[u8], format: &AudioFormat) -> Result<AudioBuffer, IronError> {
        let mut float_samples = Self::convert_pcm_to_float(pcm_data, format)?;

        // Convert sample rate if needed
        #[expect(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let context_rate = self.context_sample_rate.round() as u32;
        if format.n_samples_per_sec != context_rate {
            info!(
                "Converting sample rate from {}Hz to {}Hz",
                format.n_samples_per_sec, context_rate
            );
            float_samples = Self::convert_sample_rate(
                &float_samples,
                format.n_samples_per_sec,
                context_rate,
                format.n_channels,
            );
        }

        // Convert channel count if needed
        let target_channels = 2; // Web Audio typically works best with stereo
        if format.n_channels == 1 && target_channels == 2 {
            debug!("Converting mono to stereo");
            float_samples = Self::convert_mono_to_stereo(&float_samples);
        } else if format.n_channels == 2 && target_channels == 1 {
            debug!("Converting stereo to mono");
            float_samples = Self::convert_stereo_to_mono(&float_samples);
        }

        let final_channels = if format.n_channels == 1 && target_channels == 2 {
            2
        } else {
            format.n_channels
        };
        let sample_count = float_samples.len() / final_channels as usize;

        let buffer = self
            .audio_context
            .create_buffer(
                final_channels.into(),
                u32::try_from(sample_count).map_err(|_| anyhow::Error::msg("Sample count too large"))?,
                context_rate as f32,
            )
            .map_err(|e| anyhow::Error::msg(format!("failed to create Web Audio buffer: {e:?}")))?;

        // Fill buffer channels with converted samples
        for channel in 0..final_channels {
            let mut channel_data = buffer
                .get_channel_data(channel.into())
                .map_err(|e| anyhow::Error::msg(format!("Failed to get channel data: {e:?}")))?;

            // Interleaved to channel data conversion
            for (sample_idx, float_sample) in float_samples
                .iter()
                .skip(channel as usize)
                .step_by(final_channels as usize)
                .enumerate()
            {
                if sample_idx < channel_data.len() {
                    channel_data[sample_idx] = *float_sample;
                }
            }
        }

        Ok(buffer)
    }

    /// Apply current volume to the gain node
    fn apply_volume(&self) -> Result<(), IronError> {
        self.gain_node.gain().set_value(self.volume);
        Ok(())
    }

    /// Try to resume audio context and process pending audio data
    fn try_resume_and_process_pending(&mut self) -> Result<(), IronError> {
        if *self.context_ready.borrow() {
            return Ok(());
        }

        // Try to resume the audio context
        match self.audio_context.resume() {
            Ok(_) => {
                debug!("Audio context resumed successfully");
                *self.context_ready.borrow_mut() = true;

                // Process any pending audio data
                while let Some((data, format)) = self.pending_audio_data.pop_front() {
                    if let Ok(buffer) = self.create_audio_buffer(&data, &format) {
                        if let Err(_e) = self.audio_queue.enqueue_audio(buffer, &self.gain_node) {
                            error!("Failed to enqueue pending audio");
                        }
                    }
                }

                Ok(())
            }
            Err(_) => {
                // Context still suspended, likely needs user gesture
                Err(anyhow::Error::msg("Audio context suspended - user gesture required").into())
            }
        }
    }

    /// Set up a one-time listener for user gestures to activate audio context
    fn setup_user_gesture_listener(audio_context: AudioContext, context_ready: Rc<RefCell<bool>>) {
        let window = match web_sys::window() {
            Some(w) => w,
            None => {
                warn!("No window object available for audio gesture activation");
                return;
            }
        };

        let document = match window.document() {
            Some(d) => d,
            None => {
                warn!("No document object available for audio gesture activation");
                return;
            }
        };

        // Simplified closure pattern with proper cleanup
        let activation_handler = Rc::new(RefCell::new(None::<Closure<dyn FnMut(Event)>>));
        let handler_clone = Rc::clone(&activation_handler);
        let document_clone = document.clone();

        let activate_closure = Closure::wrap(Box::new(move |_event: Event| {
            // Use compare-and-swap pattern to prevent race conditions
            if let Ok(mut ready) = context_ready.try_borrow_mut() {
                if !*ready {
                    match audio_context.resume() {
                        Ok(_) => {
                            info!("Audio context activated by user gesture");
                            *ready = true;

                            // Remove event listeners to prevent memory leak
                            if let Some(handler) = handler_clone.borrow_mut().take() {
                                let callback = handler.as_ref().unchecked_ref();
                                let _ = document_clone.remove_event_listener_with_callback("click", callback);
                                let _ = document_clone.remove_event_listener_with_callback("keydown", callback);
                                let _ = document_clone.remove_event_listener_with_callback("touchstart", callback);
                                debug!("Audio gesture event listeners removed after activation");
                            }
                        }
                        Err(e) => {
                            debug!("Failed to resume audio context on user gesture: {e:?}");
                        }
                    }
                }
            }
        }) as Box<dyn FnMut(Event)>);

        // Add listeners for common user interaction events with error handling
        let callback_ref = activate_closure.as_ref().unchecked_ref();
        if let Err(e) = document.add_event_listener_with_callback("click", callback_ref) {
            warn!("Failed to add click listener for audio activation: {e:?}");
        }
        if let Err(e) = document.add_event_listener_with_callback("keydown", callback_ref) {
            warn!("Failed to add keydown listener for audio activation: {e:?}");
        }
        if let Err(e) = document.add_event_listener_with_callback("touchstart", callback_ref) {
            warn!("Failed to add touchstart listener for audio activation: {e:?}");
        }

        info!("Audio gesture activation listeners registered (click, keydown, touchstart)");

        // Store the closure to prevent it from being dropped
        *activation_handler.borrow_mut() = Some(activate_closure);
    }
}

impl AudioQueue {
    /// Enqueue an audio buffer for playback through the specified gain node
    fn enqueue_audio(&mut self, buffer: AudioBuffer, gain_node: &GainNode) -> Result<(), IronError> {
        // Validate audio context state
        match self.context.state() {
            web_sys::AudioContextState::Running => {}
            web_sys::AudioContextState::Suspended => {
                return Err(anyhow::Error::msg("Audio context suspended - user gesture required").into());
            }
            web_sys::AudioContextState::Closed => {
                return Err(anyhow::Error::msg("Audio context is closed").into());
            }
            _ => {
                return Err(anyhow::Error::msg("Audio context in unknown state").into());
            }
        }
        let source = self
            .context
            .create_buffer_source()
            .map_err(|e| anyhow::Error::msg(format!("Failed to create buffer source: {e:?}")))?;

        source.set_buffer(Some(&buffer));

        // Connect through the gain node for proper volume control
        source
            .connect_with_audio_node(gain_node)
            .map_err(|e| anyhow::Error::msg(format!("Failed to connect buffer source to gain node: {e:?}")))?;

        // Use the audio context's actual current time with proper scheduling
        let context_time = self.context.current_time();
        let start_time = context_time.max(self.current_time);

        source
            .start_with_when(start_time)
            .map_err(|e| anyhow::Error::msg(format!("Failed to start audio playback: {e:?}")))?;

        // Update tracking time for next buffer
        self.current_time = start_time + buffer.duration();

        info!(
            "Audio buffer scheduled: duration={:.3}s, start_time={:.3}s, next_time={:.3}s, context_time={:.3}s",
            buffer.duration(),
            start_time,
            self.current_time,
            context_time
        );

        Ok(())
    }
}

// SAFETY: In WebAssembly single-threaded environment, Send is safe for WebAudioBackend
unsafe impl Send for WebAudioBackend {}

impl RdpsndClientHandler for WebAudioBackend {
    fn get_flags(&self) -> AudioFormatFlags {
        // Return basic flags for web audio compatibility
        AudioFormatFlags::empty()
    }

    fn get_formats(&self) -> &[AudioFormat] {
        &self.supported_formats
    }

    fn wave(&mut self, format_no: usize, ts: u32, data: Cow<'_, [u8]>) {
        info!(
            "Received audio wave: format_no={}, timestamp={}, data_len={} bytes",
            format_no,
            ts,
            data.len()
        );

        if !self.is_active {
            debug!("Audio backend not active, ignoring wave data");
            return;
        }

        let format = match self.supported_formats.get(format_no) {
            Some(format) => format.clone(),
            None => {
                warn!("Invalid format number: {}", format_no);
                return;
            }
        };

        // Try to resume context and process audio
        if let Ok(()) = self.try_resume_and_process_pending() {
            // Context is ready, play audio immediately
            match self.create_audio_buffer(&data, &format) {
                Ok(buffer) => {
                    if let Err(e) = self.audio_queue.enqueue_audio(buffer, &self.gain_node) {
                        error!("Failed to enqueue audio: {:?}", e);
                    } else {
                        info!("Successfully processed audio format: {:?}", format.format);
                    }
                }
                Err(e) => {
                    warn!("Failed to create audio buffer for format {:?}: {:?}", format.format, e);
                }
            }
        } else {
            // Context not ready, buffer the audio data
            if self.pending_audio_data.len() < 10 {
                // Limit buffer size
                self.pending_audio_data.push_back((data.to_vec(), format));
                info!(
                    "Audio data buffered (context not ready), queue size: {}",
                    self.pending_audio_data.len()
                );
            } else {
                warn!("Audio buffer full, dropping oldest data (consider user gesture to activate audio)");
                self.pending_audio_data.pop_front();
                self.pending_audio_data.push_back((data.to_vec(), format));
            }
        }
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        info!(
            "Setting volume: left={}, right={} (gain: {:.2})",
            volume.volume_left,
            volume.volume_right,
            (f32::from(volume.volume_left) + f32::from(volume.volume_right)) / (2.0 * 65535.0)
        );

        // Convert RDP volume (0-65535) to Web Audio gain (0.0-1.0)
        // For simplicity, use average of left and right channels
        let left_gain = f32::from(volume.volume_left) / 65535.0;
        let right_gain = f32::from(volume.volume_right) / 65535.0;
        self.volume = (left_gain + right_gain) / 2.0;

        if let Err(_e) = self.apply_volume() {
            error!("Failed to apply volume");
        }
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        info!(
            "Setting pitch: {} (note: pitch control not implemented for web)",
            pitch.pitch
        );

        // Store pitch value but don't implement pitch shifting for now
        // Web Audio API pitch shifting would require more complex processing
        self.pitch = pitch.pitch as f32 / 65535.0;
    }

    fn close(&mut self) {
        info!("Closing audio backend");
        self.is_active = false;

        // Suspend audio context to free resources
        if let Err(e) = self.audio_context.suspend() {
            warn!("Failed to suspend audio context: {e:?}");
        }

        // Audio queue cleanup (buffers are automatically cleaned up when context suspends)
        debug!("Audio backend closed, context suspended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_pcm_to_float_valid_16bit() {
        let format = AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 44100,
            n_block_align: 2,
            bits_per_sample: 16,
            data: Some(Vec::new()),
        };

        // Test data: max positive, zero, max negative values
        let pcm_data = vec![0xFF, 0x7F, 0x00, 0x00, 0x00, 0x80]; // 32767, 0, -32768
        let result = WebAudioBackend::convert_pcm_to_float(&pcm_data, &format).unwrap();

        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.999_969_5).abs() < 0.0001); // 32767/32768
        assert!((result[1] - 0.0).abs() < f32::EPSILON);
        assert!((result[2] - (-1.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_convert_pcm_to_float_invalid_bits_per_sample() {
        let format = AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 22050,
            n_block_align: 1,
            bits_per_sample: 8, // Unsupported
            data: Some(Vec::new()),
        };

        let pcm_data = vec![0x80];
        let result = WebAudioBackend::convert_pcm_to_float(&pcm_data, &format);
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_pcm_to_float_too_large_buffer() {
        let format = AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 44100,
            n_block_align: 2,
            bits_per_sample: 16,
            data: Some(Vec::new()),
        };

        // Create a buffer larger than the maximum allowed size
        let pcm_data = vec![0u8; 48000 * 2 * 2 * 10 + 1]; // Just over the limit
        let result = WebAudioBackend::convert_pcm_to_float(&pcm_data, &format);
        assert!(result.is_err());
        assert!(format!("{:?}", result.unwrap_err()).contains("audio buffer too large"));
    }

    #[test]
    fn test_convert_pcm_to_float_odd_length() {
        let format = AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 1,
            n_samples_per_sec: 22050,
            n_avg_bytes_per_sec: 44100,
            n_block_align: 2,
            bits_per_sample: 16,
            data: Some(Vec::new()),
        };

        let pcm_data = vec![0xFF, 0x7F, 0x00]; // Odd length
        let result = WebAudioBackend::convert_pcm_to_float(&pcm_data, &format);
        assert!(result.is_err());
    }

    #[test]
    fn test_convert_mono_to_stereo() {
        let mono_samples = vec![0.5f32, -0.25f32, 0.75f32];
        let stereo_samples = WebAudioBackend::convert_mono_to_stereo(&mono_samples);

        assert_eq!(stereo_samples.len(), 6);
        assert_eq!(stereo_samples, vec![0.5, 0.5, -0.25, -0.25, 0.75, 0.75]);
    }

    #[test]
    fn test_convert_stereo_to_mono() {
        let stereo_samples = vec![0.8f32, 0.4f32, -0.6f32, -0.2f32];
        let mono_samples = WebAudioBackend::convert_stereo_to_mono(&stereo_samples);

        assert_eq!(mono_samples.len(), 2);
        assert!((mono_samples[0] - 0.6).abs() < f32::EPSILON); // (0.8 + 0.4) / 2
        assert!((mono_samples[1] - (-0.4)).abs() < f32::EPSILON); // (-0.6 + -0.2) / 2
    }

    #[test]
    fn test_convert_sample_rate_no_conversion_needed() {
        let input_samples = vec![1.0f32, -1.0f32, 0.5f32, -0.5f32];
        let result = WebAudioBackend::convert_sample_rate(&input_samples, 44100, 44100, 2);
        assert_eq!(result, input_samples);
    }

    #[test]
    fn test_convert_sample_rate_upsampling() {
        let input_samples = vec![1.0f32, -1.0f32]; // 1 stereo frame at 22050 Hz
        let result = WebAudioBackend::convert_sample_rate(&input_samples, 22050, 44100, 2);

        // Should approximately double the frames (with interpolation)
        assert!(result.len() >= 2); // At least original length
        assert_eq!(result.len() % 2, 0); // Even number for stereo
    }

    #[test]
    fn test_convert_sample_rate_downsampling() {
        let input_samples = vec![1.0f32, -1.0f32, 0.5f32, -0.5f32]; // 2 stereo frames
        let result = WebAudioBackend::convert_sample_rate(&input_samples, 44100, 22050, 2);

        // Should approximately halve the frames
        assert!(result.len() <= input_samples.len());
        assert_eq!(result.len() % 2, 0); // Even number for stereo
    }

    #[test]
    fn test_create_supported_formats_includes_preferred_rate() {
        let capabilities = BrowserAudioCapabilities {
            supported_sample_rates: vec![22050, 44100, 48000],
            max_sample_rate: 48000.0,
            min_sample_rate: 8000.0,
        };

        let formats = WebAudioBackend::create_supported_formats(44100.0, &capabilities);

        // Should have formats for both stereo and mono for each rate
        assert!(formats.len() >= 6); // At least 3 rates × 2 channel configs

        // First format should be preferred rate (44100) stereo
        assert_eq!(formats[0].n_samples_per_sec, 44100);
        assert_eq!(formats[0].n_channels, 2);
        assert_eq!(formats[0].format, WaveFormat::PCM);
    }
}

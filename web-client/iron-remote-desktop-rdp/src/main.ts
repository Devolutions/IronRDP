import wasm_init, {
    setup,
    DesktopSize,
    DeviceEvent,
    InputTransaction,
    SessionBuilder,
    ClipboardData,
    Extension,
    RdpFile,
} from '../../../crates/ironrdp-web/pkg/ironrdp_web';

export async function init(log_level: string) {
    await wasm_init();
    setup(log_level);
}

export { RdpFile };

export const Backend = {
    DesktopSize: DesktopSize,
    InputTransaction: InputTransaction,
    SessionBuilder: SessionBuilder,
    ClipboardData: ClipboardData,
    DeviceEvent: DeviceEvent,
};

export function preConnectionBlob(pcb: string): Extension {
    return new Extension('pcb', pcb);
}

export function displayControl(enable: boolean): Extension {
    return new Extension('display_control', enable);
}

export function kdcProxyUrl(url: string): Extension {
    return new Extension('kdc_proxy_url', url);
}

export function outboundMessageSizeLimit(limit: number): Extension {
    return new Extension('outbound_message_size_limit', limit);
}

export function enableCredssp(enable: boolean): Extension {
    return new Extension('enable_credssp', enable);
}

/**
 * Enable or disable audio playback for the RDP session.
 * 
 * When enabled, the client will negotiate audio capabilities with the server
 * and attempt to play PCM audio through the browser's Web Audio API.
 * 
 * Requirements:
 * - Modern browsers with Web Audio API support (Chrome 14+, Firefox 25+, Safari 6+)
 * - User gesture activation (click, touch, or keypress) required by browser security policy
 * 
 * @param enable - Whether to enable audio playback
 * @returns Extension for audio enablement
 */
export function enableAudio(enable: boolean): Extension {
    return new Extension('enable_audio', enable);
}

/**
 * Set the preferred sample rate for audio format negotiation.
 * 
 * This influences which PCM format the server is likely to choose by placing
 * the specified sample rate first in the client's advertised format list.
 * The implementation automatically handles sample rate conversion if the server
 * chooses a different rate, so this is primarily an optimization.
 * 
 * Common sample rates:
 * - 22050 Hz - Lower bandwidth, suitable for voice
 * - 44100 Hz - CD quality
 * - 48000 Hz - Professional audio, often browser native
 * 
 * If not specified, the browser's native sample rate is used as the preference.
 * 
 * @param rate - Preferred sample rate in Hz (e.g., 48000 for 48kHz)
 * @returns Extension for sample rate preference
 */
export function audioSampleRate(rate: number): Extension {
    return new Extension('audio_sample_rate', rate);
}

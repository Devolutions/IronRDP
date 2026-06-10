// Protocol/backend selection for the demo client.
//
// The client is RDP by default; appending `?protocol=vnc` to the URL (or setting
// `VITE_IRON_PROTOCOL=vnc`) switches the whole app to the IronVNC backend
// (`static/iron-remote-desktop-vnc`, built from the IronVNC repo).
import * as rdp from '../../static/iron-remote-desktop-rdp';
import * as vnc from '../../static/iron-remote-desktop-vnc';

export type Protocol = 'rdp' | 'vnc';

function detectProtocol(): Protocol {
    if (typeof window !== 'undefined') {
        const fromUrl = new URLSearchParams(window.location.search).get('protocol');
        if (fromUrl === 'vnc' || fromUrl === 'rdp') {
            return fromUrl;
        }
    }
    return (import.meta.env.VITE_IRON_PROTOCOL as string | undefined) === 'vnc' ? 'vnc' : 'rdp';
}

export const protocol: Protocol = detectProtocol();

export const Backend = protocol === 'vnc' ? vnc.Backend : rdp.Backend;
export const init = protocol === 'vnc' ? vnc.init : rdp.init;

/// RDP-only config extensions; `undefined` when the VNC backend is active.
export const rdpExtensions =
    protocol === 'rdp'
        ? {
              preConnectionBlob: rdp.preConnectionBlob,
              displayControl: rdp.displayControl,
              kdcProxyUrl: rdp.kdcProxyUrl,
          }
        : undefined;

/// The Gateway generic-TCP forward endpoint is `/jet/fwd/tcp/{jet_aid}` and the path id must
/// equal the token's `jet_aid` claim; `ironvnc-web` uses `proxyAddress` verbatim (only appending
/// `?token=`), so the full per-session URL has to be built client-side from the JWT.
export function vncForwardUrl(gatewayAddress: string, token: string): string {
    const payload = token.split('.')[1];
    if (payload === undefined) {
        throw new Error('forwarding token is not a JWT');
    }
    const claims = JSON.parse(atob(payload.replace(/-/g, '+').replace(/_/g, '/'))) as { jet_aid?: string };
    if (claims.jet_aid === undefined) {
        throw new Error('forwarding token carries no jet_aid claim');
    }
    const base = new URL(gatewayAddress);
    return `${base.protocol}//${base.host}/jet/fwd/tcp/${claims.jet_aid}`;
}

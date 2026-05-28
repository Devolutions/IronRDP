//! Human-readable descriptions for canonical `.rdp` properties surfaced by the
//! `dump-properties` IPC call. The list is intentionally curated rather than
//! exhaustive; unknown keys fall back to the generic description.

pub fn property_description(key: &str) -> &'static str {
    match key {
        "full address" => "Server address (host:port)",
        "alternate full address" => "Alternate server address used by some Microsoft clients",
        "username" => "Username used for the connection",
        "ClearTextPassword" => "Plain-text password (live-only; never persisted)",
        "domain" => "Active Directory or local domain",
        "desktopwidth" => "Initial desktop width in pixels",
        "desktopheight" => "Initial desktop height in pixels",
        "desktopscalefactor" => "DPI scale factor (100, 125, 150, 200)",
        "session bpp" => "Desktop colour depth in bits per pixel",
        "compression" => "Whether bulk compression is enabled (0/1)",
        "audiomode" => "Audio redirection mode",
        "redirectclipboard" => "Whether the clipboard is redirected",
        "redirectprinters" => "Whether printers are redirected",
        "redirectsmartcards" => "Whether smart cards are redirected",
        "enablecredsspsupport" => "Whether CredSSP/NLA is enabled",
        "negotiate security layer" => "Whether security negotiation is enabled",
        "gatewayhostname" => "RDP gateway host name",
        "gatewayusername" => "RDP gateway username",
        "GatewayPassword" => "RDP gateway password (live-only)",
        "gatewayusagemethod" => "Gateway usage method (0=never, 1=direct, 2=detect, 4=default)",
        "kdcproxyname" => "KDC proxy URL for Kerberos over HTTPS",
        "drivestoredirect" => "Comma-separated drive paths to redirect",
        "autoreconnection enabled" => "Whether auto-reconnect on transient failures is enabled",
        "prompt for credentials" => "Whether the client should prompt for credentials",
        "use multimon" => "Whether multimon span is enabled",
        "agent:state" => "Current session state (connecting, connected, failed, disconnected)",
        "agent:last_error" => "Last fatal error reported by the session, if any",
        "agent:current_width" => "Current desktop width as last reported by the server",
        "agent:current_height" => "Current desktop height as last reported by the server",
        "agent:label" => "Human-readable label for the session",
        "agent:frame_sequence" => "Monotonically increasing framebuffer sequence number",
        _ => "(custom or undocumented property)",
    }
}

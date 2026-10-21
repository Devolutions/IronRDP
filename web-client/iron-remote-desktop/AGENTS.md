# iron-remote-desktop

`iron-remote-desktop` is protocol-agnostic.
A method belongs in the base API (`UserInteraction`, `Session`, `SessionBuilder`) only if the web component itself needs to call it for transparent behaviour or the feature is universal across remote-desktop backends.
Protocol-specific concepts go through the extension mechanism.
See `ARCHITECTURE.md` § `web-client/iron-remote-desktop`.

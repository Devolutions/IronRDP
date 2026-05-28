//! Static long-form help text emitted by `--help-agent`.

pub const HELP_AGENT: &str = r#"
ironrdp-agent - daemon and CLI for driving RDP sessions programmatically

OVERVIEW

  ironrdp-agent is split into a long-running daemon that hosts one or more
  RDP sessions and a thin client mode that issues IPC requests against it.
  The transport is a binary length-prefixed framing format using ironrdp-core's
  Encode/Decode traits; no HTTP, JSON, or serde are involved.

ENDPOINTS

  By default the daemon listens on a platform-appropriate endpoint:

      Windows:  pipe:ironrdp-agent
      Unix:     unix:/tmp/ironrdp-agent.sock

  Use `--endpoint pipe:NAME` or `--endpoint unix:/path` to override.

SUBCOMMANDS

  daemon                  Run the IPC daemon in the foreground.
  connect                 Open a new RDP session through the daemon.
  sessions                List all active sessions.
  status [--session ID]   Print daemon health, or the status of one session.
  disconnect --session ID Close a session and disconnect cleanly.
  mouse                   Send mouse input (move/click/down/up/wheel/position).
  keyboard                Send keyboard input (key/text/shortcut/release-all).
  resize                  Re-issue display-control with new dimensions.
  wait-frame              Block until a new framebuffer has arrived.
  screenshot              Save the latest framebuffer to a PNG file.
  dump-properties         Print the live PropertySet for a session.

LIVE PROPERTY SET

  Every session keeps a live `PropertySet` that combines the original `.rdp`
  content with synthetic `agent:*` properties:

      agent:state             connecting | connected | failed | disconnected
      agent:last_error        last fatal error, if any
      agent:current_width     current desktop width
      agent:current_height    current desktop height
      agent:label             user-provided label
      agent:frame_sequence    monotonic framebuffer counter

  `dump-properties` returns all entries with human-readable descriptions.
  `set-property` can adjust a small set of mutable runtime properties.

CONNECT PAYLOAD

  Unlike PR #1289, the agent does not forward argv strings. Instead the
  client serialises its CLI choices into a `.rdp` text file (the same format
  the viewer's --dump-rdp emits) and sends that to the daemon over IPC. The
  daemon parses it back into a `PropertySet` and feeds the result through
  `ironrdp_client::config::ConfigBuilder::from_property_set(...)`.

EXAMPLES

  ironrdp-agent daemon
  ironrdp-agent connect host.example -u user -p secret --label primary
  ironrdp-agent sessions
  ironrdp-agent mouse --session $ID move --x 100 --y 200
  ironrdp-agent keyboard --session $ID text --text "hello"
  ironrdp-agent screenshot --session $ID --output ./shot.png
  ironrdp-agent dump-properties --session $ID

EXIT CODES

  0   Success.
  1   Generic failure (IPC error, daemon refusal).
  2   CLI parsing error (clap default).
"#;

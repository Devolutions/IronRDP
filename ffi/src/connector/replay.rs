//! FFI builder for replay sessions.
//!
//! A replay (see `ironrdp-replay-bench` / the .NET `ReplayBench`) has no live server, so it cannot
//! obtain a [`ConnectionResult`] from the connector. This builder reconstructs one from the recorded
//! `IRDPREC1` manifest values via the shared `ironrdp-replay-core` crate, so the .NET replay builds a
//! session identical to the native one and reproduces the same framebuffer checksum.

#[diplomat::bridge]
pub mod ffi {
    use ironrdp_replay_core::{ChannelEntry, ReplayParams, build_connection_result, parse_compression};

    use crate::connector::result::ffi::ConnectionResult;

    /// Accumulates the manifest values needed to rebuild a replay [`ConnectionResult`]. Set the
    /// compression and add the recorded static channels, then call [`ReplayConnectionBuilder::build`].
    #[diplomat::opaque]
    pub struct ReplayConnectionBuilder(pub ReplayParams);

    impl ReplayConnectionBuilder {
        #[expect(clippy::too_many_arguments, reason = "flat FFI constructor mirroring the manifest")]
        pub fn new(
            io_channel_id: u16,
            user_channel_id: u16,
            share_id: u32,
            desktop_width: u16,
            desktop_height: u16,
            enable_server_pointer: bool,
            pointer_software_rendering: bool,
        ) -> Box<ReplayConnectionBuilder> {
            Box::new(ReplayConnectionBuilder(ReplayParams {
                io_channel_id,
                user_channel_id,
                share_id,
                desktop_width,
                desktop_height,
                enable_server_pointer,
                pointer_software_rendering,
                compression_type: None,
                channels: Vec::new(),
            }))
        }

        /// Sets the negotiated bulk compression by its manifest name (`"K8"`, `"K64"`, `"Rdp6"`,
        /// `"Rdp61"`); any other value (including empty) means no compression.
        pub fn set_compression(&mut self, name: &str) {
            self.0.compression_type = parse_compression(Some(name));
        }

        /// Adds a recorded static virtual channel (`drdynvc` / `rdpsnd` / `rdpdr`) with the server
        /// channel ID it was assigned, so the replay routes that channel's captured traffic.
        pub fn add_channel(&mut self, name: &str, channel_id: u16) {
            self.0.channels.push(ChannelEntry {
                name: name.to_owned(),
                id: channel_id,
            });
        }

        /// Builds the [`ConnectionResult`] to pass to `ActiveStage::new`.
        pub fn build(&self) -> Box<ConnectionResult> {
            Box::new(ConnectionResult(Some(build_connection_result(&self.0))))
        }
    }
}

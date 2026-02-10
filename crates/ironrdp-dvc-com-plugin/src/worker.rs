//! COM worker thread that drives the plugin lifecycle.
//!
//! All COM objects live on this single thread. Communication with the
//! [`DvcComChannel`](crate::channel::DvcComChannel) instances (which live on
//! IronRDP's async runtime threads) happens via `std::sync::mpsc` channels.

use std::collections::HashMap;
use std::sync::mpsc as std_mpsc;

use tracing::{debug, error, info, trace, warn};
use windows::Win32::System::RemoteDesktop::{
    IWTSListenerCallback, IWTSPlugin, IWTSVirtualChannel, IWTSVirtualChannelCallback, IWTSVirtualChannelManager,
};
use windows_core::{BOOL, BSTR};

use crate::com::{ActiveChannel, OnWriteDvc, VirtualChannel};

/// Commands sent from [`DvcComChannel`] to the COM worker thread.
pub(crate) enum ComCommand {
    /// The server created a DVC matching one of our listeners.
    /// The COM worker should call `OnNewChannelConnection` on the plugin's listener callback.
    ChannelOpened {
        channel_name: String,
        channel_id: u32,
        /// Reply channel: send `true` if the plugin accepted the channel.
        accept_tx: std_mpsc::SyncSender<bool>,
    },

    /// Data arrived from the RDP server for this channel.
    DataReceived { channel_id: u32, data: Vec<u8> },

    /// The server (or IronRDP) closed this channel.
    ChannelClosed { channel_id: u32 },

    /// The RDP connection is established; notify the plugin.
    Connected,

    /// The session is ending; tell the plugin to clean up.
    Shutdown,
}

/// Run the COM worker loop on the current thread.
///
/// This function blocks until a `Shutdown` command is received. It must be called
/// on a dedicated thread since COM objects are `!Send`.
pub(crate) fn run_com_worker(
    plugin: IWTSPlugin,
    _manager: IWTSVirtualChannelManager,
    listeners: HashMap<String, IWTSListenerCallback>,
    command_rx: std_mpsc::Receiver<ComCommand>,
    on_write_dvc_rx: std_mpsc::Receiver<OnWriteDvc>,
) {
    info!("COM worker thread started");

    let mut active_channels: HashMap<u32, ActiveChannel> = HashMap::new();

    loop {
        let cmd = match command_rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => {
                debug!("Command channel closed, shutting down COM worker");
                break;
            }
        };

        match cmd {
            ComCommand::Connected => {
                debug!("Notifying plugin: Connected");
                // SAFETY: calling COM method on the thread that owns the objects
                let result = unsafe { plugin.Connected() };
                if let Err(e) = result {
                    // Per the spec, Connected() failure is non-fatal
                    warn!("IWTSPlugin::Connected returned error (non-fatal): {e}");
                }
            }

            ComCommand::ChannelOpened {
                channel_name,
                channel_id,
                accept_tx,
            } => {
                debug!(channel_name = %channel_name, channel_id, "Opening DVC channel via COM plugin");

                let listener_callback = match listeners.get(&channel_name) {
                    Some(cb) => cb,
                    None => {
                        warn!(channel_name = %channel_name, "No listener registered for channel");
                        let _ = accept_tx.send(false);
                        continue;
                    }
                };

                // Create the IWTSVirtualChannel that the plugin will use to Write() data
                //
                // We need to clone the on_write_dvc callback. Since it's boxed, we need
                // to receive a new one for each channel. But for simplicity, we'll wrap
                // the callback in an Arc-based approach.
                //
                // Actually, the write callback just sends to an mpsc channel, so we
                // receive a fresh one for each channel that needs it.
                let on_write: OnWriteDvc = match on_write_dvc_rx.try_recv() {
                    Ok(cb) => cb,
                    Err(_) => {
                        // Reuse the primary one by having the caller send a fresh one
                        // For the first channel, we already got it above. For subsequent channels
                        // we need a fresh callback. The caller sends one per ChannelOpened.
                        // If we can't get one, something went wrong.
                        error!("No write callback for channel {channel_name}");
                        let _ = accept_tx.send(false);
                        continue;
                    }
                };

                let virtual_channel: IWTSVirtualChannel = VirtualChannel::new(channel_id, on_write).into();

                let mut accept = BOOL::default();
                let mut channel_callback: Option<IWTSVirtualChannelCallback> = None;

                // SAFETY: calling COM method on the owner thread; pointers are valid for the call duration
                let result = unsafe {
                    listener_callback.OnNewChannelConnection(
                        &virtual_channel,
                        &BSTR::default(),
                        &mut accept,
                        &mut channel_callback,
                    )
                };

                match result {
                    Ok(()) if accept.as_bool() => {
                        if let Some(callback) = channel_callback {
                            info!(channel_name = %channel_name, channel_id, "Plugin accepted DVC channel");
                            active_channels.insert(
                                channel_id,
                                ActiveChannel {
                                    callback,
                                    _channel: virtual_channel,
                                },
                            );
                            let _ = accept_tx.send(true);
                        } else {
                            warn!(
                                channel_name = %channel_name, channel_id,
                                "Plugin accepted channel but returned no callback"
                            );
                            let _ = accept_tx.send(false);
                        }
                    }
                    Ok(()) => {
                        debug!(channel_name = %channel_name, channel_id, "Plugin rejected DVC channel");
                        let _ = accept_tx.send(false);
                    }
                    Err(e) => {
                        warn!(
                            channel_name = %channel_name, channel_id,
                            "OnNewChannelConnection failed: {e}"
                        );
                        let _ = accept_tx.send(false);
                    }
                }
            }

            ComCommand::DataReceived { channel_id, data } => {
                trace!(channel_id, size = data.len(), "Forwarding data to COM plugin");

                if let Some(active) = active_channels.get(&channel_id) {
                    // SAFETY: calling COM method on the owner thread; buffer is valid for the call
                    let result = unsafe { active.callback.OnDataReceived(&data) };
                    if let Err(e) = result {
                        warn!(channel_id, "OnDataReceived failed: {e}");
                    }
                } else {
                    warn!(channel_id, "Data received for unknown channel");
                }
            }

            ComCommand::ChannelClosed { channel_id } => {
                debug!(channel_id, "Closing DVC channel in COM plugin");

                if let Some(active) = active_channels.remove(&channel_id) {
                    // SAFETY: calling COM method on the owner thread
                    let result = unsafe { active.callback.OnClose() };
                    if let Err(e) = result {
                        warn!(channel_id, "OnClose failed: {e}");
                    }
                }
            }

            ComCommand::Shutdown => {
                info!("Shutting down COM plugin");

                // Close all active channels
                for (channel_id, active) in active_channels.drain() {
                    // SAFETY: calling COM method on the owner thread
                    let result = unsafe { active.callback.OnClose() };
                    if let Err(e) = result {
                        warn!(channel_id, "OnClose during shutdown failed: {e}");
                    }
                }

                // SAFETY: calling COM methods on the owner thread
                unsafe {
                    let result = plugin.Disconnected(0);
                    if let Err(e) = result {
                        warn!("IWTSPlugin::Disconnected failed: {e}");
                    }

                    let result = plugin.Terminated();
                    if let Err(e) = result {
                        warn!("IWTSPlugin::Terminated failed: {e}");
                    }
                }

                break;
            }
        }
    }

    info!("COM worker thread exiting");
}

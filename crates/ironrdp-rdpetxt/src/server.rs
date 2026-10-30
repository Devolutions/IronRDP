//! Server side of the text input channels.
//!
//! The server sends on [`SERVER_TO_CLIENT_CHANNEL_NAME`] and receives on
//! [`CLIENT_TO_SERVER_CHANNEL_NAME`], so both processors share one [`TextInputServer`]:
//! [`ClientToServerChannel`] hands it client PDUs, and the PDUs it queues leave through
//! [`ServerToClientChannel`], when that channel opens and then whenever the server drains
//! [`ServerToClientChannel::pending_messages`].
//!
//! The application behind the text input objects, which acknowledges operations and reports
//! focus and text changes, is the [`TextInputServerHandler`].

use std::sync::{Arc, Mutex};

use ironrdp_core::impl_as_any;
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::channel::{lock, process_frames, versioned_messages};
use crate::pdu::{ClientPdu, Guid, NotifyServerVersionPdu, PduLayout, ServerPdu};
use crate::{
    CLIENT_TO_SERVER_CHANNEL_NAME, MAX_MESSAGE_SIZE, MAX_PENDING_PDUS, QueueFull, SERVER_TO_CLIENT_CHANNEL_NAME,
};

/// Application side of the text input objects.
pub trait TextInputServerHandler: Send {
    /// Handles a PDU from the client and returns the PDUs to send back.
    fn client_pdu(&mut self, pdu: ClientPdu) -> Vec<ServerPdu>;
}

/// Server state shared by both text input channel processors.
pub type SharedTextInputServer = Arc<Mutex<TextInputServer>>;

/// Server protocol state for MS-RDPETXT.
pub struct TextInputServer {
    version_major: u32,
    version_minor: u32,
    client_version: Option<(u32, u32)>,
    handler: Box<dyn TextInputServerHandler>,
    /// INVARIANT: `pending.len() <= MAX_PENDING_PDUS`.
    pending: Vec<ServerPdu>,
}

impl TextInputServer {
    /// Creates a server announcing version 1 with the **bcpTag** of `CoreInputProfile`
    /// (minor bit 0x00000002), the only update the specification documents.
    pub fn new(handler: Box<dyn TextInputServerHandler>) -> Self {
        Self {
            version_major: 1,
            version_minor: 0x0000_0002,
            client_version: None,
            handler,
            pending: Vec::new(),
        }
    }

    /// Announces `major` and `minor` instead of the default version.
    #[must_use]
    pub fn with_version(mut self, major: u32, minor: u32) -> Self {
        self.version_major = major;
        self.version_minor = minor;
        self
    }

    /// Wraps the server for sharing between [`ServerToClientChannel`] and [`ClientToServerChannel`].
    pub fn into_shared(self) -> SharedTextInputServer {
        Arc::new(Mutex::new(self))
    }

    /// Major and minor version announced by the client, once received.
    pub fn client_version(&self) -> Option<(u32, u32)> {
        self.client_version
    }

    /// The fields both peers support, or the base layout until the client announced its version.
    pub fn layout(&self) -> PduLayout {
        self.client_version.map_or(PduLayout::BASE, |client_version| {
            PduLayout::negotiated((self.version_major, self.version_minor), client_version)
        })
    }

    /// Queues `pdu` for the client, unless [`MAX_PENDING_PDUS`] PDUs are queued already.
    pub fn send(&mut self, pdu: impl Into<ServerPdu>) -> Result<(), QueueFull> {
        if self.pending.len() == MAX_PENDING_PDUS {
            return Err(QueueFull);
        }
        self.pending.push(pdu.into());
        Ok(())
    }

    /// Removes and returns the PDUs queued for the client.
    pub fn take_pending(&mut self) -> Vec<ServerPdu> {
        core::mem::take(&mut self.pending)
    }

    /// Applies a client PDU and queues the handler's replies.
    ///
    /// Fails when a reply cannot be queued; the replies queued before it stay queued.
    pub fn process(&mut self, pdu: ClientPdu) -> Result<(), QueueFull> {
        if let ClientPdu::NotifyClientVersion(version) = &pdu {
            debug!(
                major = version.version_major,
                minor = version.version_minor,
                "Text input client version received"
            );
            self.client_version = Some((version.version_major, version.version_minor));
        }
        self.handler
            .client_pdu(pdu)
            .into_iter()
            .try_for_each(|reply| self.send(reply))
    }

    fn version_pdu(&self) -> ServerPdu {
        ServerPdu::NotifyServerVersion(NotifyServerVersionPdu {
            container_id: Guid::default(),
            version_major: self.version_major,
            version_minor: self.version_minor,
        })
    }
}

impl core::fmt::Debug for TextInputServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextInputServer")
            .field("version_major", &self.version_major)
            .field("version_minor", &self.version_minor)
            .field("client_version", &self.client_version)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

/// Server processor for [`SERVER_TO_CLIENT_CHANNEL_NAME`].
#[derive(Debug)]
pub struct ServerToClientChannel {
    server: SharedTextInputServer,
    channel_id: Option<u32>,
}

impl ServerToClientChannel {
    pub fn new(server: SharedTextInputServer) -> Self {
        Self {
            server,
            channel_id: None,
        }
    }

    pub fn server(&self) -> &SharedTextInputServer {
        &self.server
    }

    /// Encodes the PDUs queued for the client, or nothing while the channel is closed.
    pub fn pending_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let Some(channel_id) = self.channel_id else {
            return Ok(Vec::new());
        };
        let messages = take_dvc_messages(&mut lock(&self.server));
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        encode_dvc_messages(channel_id, messages, ChannelFlags::empty()).map_err(|error| encode_err!(error))
    }
}

impl_as_any!(ServerToClientChannel);

impl DvcProcessor for ServerToClientChannel {
    fn channel_name(&self) -> &str {
        SERVER_TO_CLIENT_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        debug!(channel_id, "Text input server-to-client channel started");
        let mut server = lock(&self.server);
        server.client_version = None;
        // The server announces its version first ([1.7] Versioning and Capability Negotiation).
        let mut messages: Vec<DvcMessage> = vec![Box::new(server.version_pdu())];
        messages.extend(take_dvc_messages(&mut server));
        Ok(messages)
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        warn!(
            channel_id,
            size = payload.len(),
            "Ignoring data received on the text input server-to-client channel"
        );
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        lock(&self.server).take_pending();
        debug!("Text input server-to-client channel closed");
    }
}

impl DvcServerProcessor for ServerToClientChannel {}

/// Server processor for [`CLIENT_TO_SERVER_CHANNEL_NAME`].
#[derive(Debug)]
pub struct ClientToServerChannel {
    server: SharedTextInputServer,
}

impl ClientToServerChannel {
    pub fn new(server: SharedTextInputServer) -> Self {
        Self { server }
    }

    pub fn server(&self) -> &SharedTextInputServer {
        &self.server
    }
}

impl_as_any!(ClientToServerChannel);

impl DvcProcessor for ClientToServerChannel {
    fn channel_name(&self) -> &str {
        CLIENT_TO_SERVER_CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_MESSAGE_SIZE)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(channel_id, "Text input client-to-server channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        process_frames(
            payload,
            &mut *lock(&self.server),
            TextInputServer::layout,
            ClientPdu::decode_body,
            TextInputServer::process,
        );
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for ClientToServerChannel {}

fn take_dvc_messages(server: &mut TextInputServer) -> Vec<DvcMessage> {
    let layout = server.layout();
    versioned_messages(server.take_pending(), layout)
}

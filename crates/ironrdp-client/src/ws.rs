use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite;

pub(crate) fn websocket_compat<S>(stream: S) -> impl AsyncRead + AsyncWrite + Unpin + Send + 'static
where
    S: Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
        + Sink<tungstenite::Message, Error = tungstenite::Error>
        + Unpin
        + Send
        + 'static,
{
    let compat = stream
        .filter_map(|item| {
            let mapped = item
                .map(|msg| match msg {
                    tungstenite::Message::Text(s) => Some(transport::WsReadMsg::Payload(tungstenite::Bytes::from(s))),
                    tungstenite::Message::Binary(data) => Some(transport::WsReadMsg::Payload(data)),
                    tungstenite::Message::Ping(_) | tungstenite::Message::Pong(_) => None,
                    tungstenite::Message::Close(_) => Some(transport::WsReadMsg::Close),
                    tungstenite::Message::Frame(_) => unreachable!("raw frames are never returned when reading"),
                })
                .transpose();

            core::future::ready(mapped)
        })
        .with(|item| {
            core::future::ready(Ok::<_, tungstenite::Error>(tungstenite::Message::Binary(
                tungstenite::Bytes::from(item),
            )))
        });

    transport::WsStream::new(compat)
}

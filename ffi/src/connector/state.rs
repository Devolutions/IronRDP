#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct ClientConnectorState(pub ironrdp::connector::ClientConnectorState);
}

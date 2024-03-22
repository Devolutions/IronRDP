
#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct StaticChannelSet(pub ironrdp::svc::StaticChannelSet);
}
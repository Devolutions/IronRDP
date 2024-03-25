
#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct StaticChannelSet<'a>(pub &'a ironrdp::svc::StaticChannelSet);
}
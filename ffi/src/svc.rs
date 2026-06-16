#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque_mut]
    pub struct StaticChannelSet<'a>(pub &'a ironrdp::svc::StaticChannelSet);
}

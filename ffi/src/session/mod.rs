
#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct ActiveStage(pub ironrdp::session::ActiveStage);
}
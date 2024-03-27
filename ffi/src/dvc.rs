#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct DrdynvcChannel(pub ironrdp::dvc::DrdynvcClient);
}

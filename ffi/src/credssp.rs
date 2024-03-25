#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct KerberosConfig(pub ironrdp::connector::credssp::KerberosConfig);
}

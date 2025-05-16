use windows::core::PCWSTR;

#[derive(Default, Debug)]
pub(crate) struct WideString(pub Vec<u16>);

impl WideString {
    pub(crate) fn new(s: &str) -> Self {
        let mut buf = s.encode_utf16().collect::<Vec<_>>();
        buf.push(0);
        Self(buf)
    }

    pub(crate) fn as_pcwstr(&self) -> PCWSTR {
        PCWSTR::from_raw(self.0.as_ptr())
    }
}

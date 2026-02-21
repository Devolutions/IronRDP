/// usage | 0xAA
pub fn usage_ke(usage: i32) -> [u8; 5] {
    key_usage(usage, 0xAA)
}

/// usage | 0x99
pub fn usage_kc(usage: i32) -> [u8; 5] {
    key_usage(usage, 0x99)
}

/// usage | 0x55
pub fn usage_ki(usage: i32) -> [u8; 5] {
    key_usage(usage, 0x55)
}

/// https://www.rfc-editor.org/rfc/rfc3961#section-5.3
/// the key usage number, expressed as four octets in big-endian order, followed by one octet
fn key_usage(usage: i32, well_known_constant: u8) -> [u8; 5] {
    // 5 = 4 /* usage */ + 1 /* known constant */
    let mut result = [0; 5];

    result[0..4].copy_from_slice(&usage.to_be_bytes());
    result[4] = well_known_constant;

    result
}

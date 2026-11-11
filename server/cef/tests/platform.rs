use ironrdp_cef_server::platform::Platform;

#[test]
fn windows_native_keys_use_cefs_dom_code_scan_codes() {
    // oracle: reference chromiumembedded/cef@8ea0c73b5dbed1389c8dafb911a98c41246bb8cb:libcef/browser/native/browser_platform_delegate_native_win.cc.
    // oracle: reference chromium/chromium@8954d18543712dc11b97c3aca0a6e714aa2d7dcd:ui/events/keycodes/dom/dom_code_data.inc.
    assert_eq!(Platform::Windows.native_key_code(0x1e, false, 0), Some(0x001e));
    assert_eq!(Platform::Windows.native_key_code(0x4b, true, 123), Some(0xe04b));
    assert_eq!(Platform::Windows.native_key_code(0x1d, true, 59), Some(0xe01d));
}

#[test]
fn windows_character_events_use_the_typed_character() {
    // oracle: reference chromiumembedded/cef@8ea0c73b5dbed1389c8dafb911a98c41246bb8cb:libcef/browser/native/browser_platform_delegate_native_aura.cc.
    assert_eq!(
        Platform::Windows.character_key_code(i32::from(b'A'), u16::from(b'a')),
        i32::from(b'a')
    );
    assert_eq!(
        Platform::Windows.character_key_code(i32::from(b'1'), u16::from(b'!')),
        i32::from(b'!')
    );
}

#[test]
fn linux_character_events_keep_the_virtual_key() {
    // oracle: reference chromiumembedded/cef@8ea0c73b5dbed1389c8dafb911a98c41246bb8cb:libcef/browser/native/browser_platform_delegate_native_linux.cc.
    assert_eq!(
        Platform::Linux.character_key_code(i32::from(b'A'), u16::from(b'a')),
        i32::from(b'A')
    );
}

#[test]
fn alt_system_key_flag_is_confined_to_windows() {
    // oracle: reference chromiumembedded/cef@8ea0c73b5dbed1389c8dafb911a98c41246bb8cb:include/internal/cef_types.h.
    // oracle: spec Microsoft WM_SYSKEYDOWN §Parameters.
    assert!(!Platform::Linux.is_alt_system_key(true));
    assert!(!Platform::MacOs.is_alt_system_key(true));
    assert!(Platform::Windows.is_alt_system_key(true));
    assert!(!Platform::Windows.is_alt_system_key(false));
}

#[test]
fn linux_native_keys_use_xkb_codes_instead_of_mac_codes() {
    // oracle: reference chromium/chromium@8954d18543712dc11b97c3aca0a6e714aa2d7dcd:ui/events/keycodes/dom/dom_code_data.inc.
    assert_eq!(Platform::Linux.native_key_code(0x1e, false, 0), Some(38));
    assert_eq!(Platform::Linux.native_key_code(0x1d, true, 59), Some(105));
    assert_eq!(Platform::Linux.native_key_code(0x4b, true, 123), Some(113));
    assert_eq!(Platform::Linux.native_key_code(0x5c, true, 55), Some(134));
}

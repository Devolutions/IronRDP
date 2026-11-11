use std::path::PathBuf;

use anyhow::Context as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Linux,
    Windows,
}

impl Platform {
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        return Self::MacOs;
        #[cfg(target_os = "linux")]
        return Self::Linux;
        #[cfg(windows)]
        return Self::Windows;
    }

    pub fn native_key_code(self, scan: u8, extended: bool, mac_code: i32) -> Option<i32> {
        match self {
            Self::Windows => Some(i32::from(scan) | if extended { 0xe000 } else { 0 }),
            Self::Linux if extended => Some(match scan {
                0x1c => 104,
                0x1d => 105,
                0x35 => 106,
                0x38 => 108,
                0x47 => 110,
                0x48 => 111,
                0x49 => 112,
                0x4b => 113,
                0x4d => 114,
                0x4f => 115,
                0x50 => 116,
                0x51 => 117,
                0x52 => 118,
                0x53 => 119,
                0x5b => 133,
                0x5c => 134,
                _ => return None,
            }),
            Self::Linux => Some(i32::from(scan) + 8),
            Self::MacOs => Some(mac_code),
        }
    }

    pub fn character_key_code(self, virtual_key: i32, character: u16) -> i32 {
        match self {
            Self::Windows => i32::from(character),
            Self::MacOs | Self::Linux => virtual_key,
        }
    }

    pub const fn is_alt_system_key(self, alt_active: bool) -> bool {
        matches!(self, Self::Windows) && alt_active
    }
}

pub fn default_font() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let candidates = [PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf")];
    #[cfg(windows)]
    let candidates = [
        PathBuf::from(std::env::var_os("WINDIR").context("WINDIR is unset; specify --font")?)
            .join("Fonts")
            .join("arial.ttf"),
    ];
    #[cfg(target_os = "linux")]
    let candidates = [
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"),
        PathBuf::from("/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .context("no toolbar font found; install a TrueType font or specify --font")
}

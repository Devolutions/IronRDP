use std::io::{self, Read as _};
use std::path::Path;

pub const DISPLAY_REQUEST_DIRECTORY: &str = r"C:\ProgramData\IronRDPDisplay";

/// A single-monitor size requested for an assigned Windows session.
///
/// The file representation is `IDS1`, followed by little-endian width and height.
/// Both dimensions are 200..=8192 pixels, the width is even, and the area is at most 3840 * 2400 pixels.
/// Publishing replaces the entire file atomically so the driver cannot read a partial request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplaySize {
    width: u16,
    height: u16,
}

impl DisplaySize {
    pub fn new(width: u16, height: u16) -> io::Result<Self> {
        if (200..=8192).contains(&width)
            && width.is_multiple_of(2)
            && (200..=8192).contains(&height)
            && u32::from(width) * u32::from(height) <= 3840 * 2400
        {
            Ok(Self { width, height })
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid display size"))
        }
    }

    pub fn width(self) -> u16 {
        self.width
    }

    pub fn height(self) -> u16 {
        self.height
    }

    pub fn publish(self, directory: &Path, session_id: u32, connection_id: u32) -> io::Result<()> {
        if session_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "display session is not assigned",
            ));
        }
        let temporary = directory.join(format!("display-size-{session_id}-{connection_id}.tmp"));
        let mut bytes = *b"IDS1\0\0\0\0";
        bytes[4..6].copy_from_slice(&self.width.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.height.to_le_bytes());
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, directory.join(format!("display-size-{session_id}.bin")))
    }

    pub fn read(directory: &Path, session_id: u32) -> io::Result<Self> {
        if session_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "display session is not assigned",
            ));
        }
        let mut file = std::fs::File::open(directory.join(format!("display-size-{session_id}.bin")))?;
        let mut bytes = [0; 4 /* Magic */ + 2 /* Width */ + 2 /* Height */];
        file.read_exact(&mut bytes)?;
        if &bytes[..4] != b"IDS1" || file.read(&mut [0])? != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid display size record",
            ));
        }
        Self::new(
            u16::from_le_bytes([bytes[4], bytes[5]]),
            u16::from_le_bytes([bytes[6], bytes[7]]),
        )
    }
}

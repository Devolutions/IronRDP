use tracing::{debug, warn};

/// 8bpp color palette (256 RGB entries).
///
/// Initialized with the default Windows system palette (VGA colors)
/// per MS-RDPBCGR 2.2.9.1.1.3.1.1. Updated by TS_UPDATE_PALETTE_DATA
/// updates during the session.
#[derive(Debug, Clone)]
pub(crate) struct Palette {
    colors: [[u8; 3]; 256],
}

impl Palette {
    /// Create a palette initialized with the 20 static colors from the
    /// Windows default system palette. Indices 0-9 and 246-255 are the
    /// reserved static colors; the middle 236 entries (10-245) are black.
    ///
    /// Reference: <https://learn.microsoft.com/en-us/windows/win32/gdi/default-palette>
    pub(crate) fn system_default() -> Self {
        let mut colors = [[0u8; 3]; 256];
        // Lower 10 static colors (indices 0-9)
        colors[0] = [0, 0, 0]; // Black
        colors[1] = [128, 0, 0]; // Dark Red
        colors[2] = [0, 128, 0]; // Dark Green
        colors[3] = [128, 128, 0]; // Dark Yellow
        colors[4] = [0, 0, 128]; // Dark Blue
        colors[5] = [128, 0, 128]; // Dark Magenta
        colors[6] = [0, 128, 128]; // Dark Cyan
        colors[7] = [192, 192, 192]; // Light Gray
        colors[8] = [192, 220, 192]; // Money Green
        colors[9] = [166, 202, 240]; // Sky Blue
        // Upper 10 static colors (indices 246-255)
        colors[246] = [255, 251, 240]; // Cream
        colors[247] = [160, 160, 164]; // Medium Gray
        colors[248] = [128, 128, 128]; // Dark Gray
        colors[249] = [255, 0, 0]; // Red
        colors[250] = [0, 255, 0]; // Green
        colors[251] = [255, 255, 0]; // Yellow
        colors[252] = [0, 0, 255]; // Blue
        colors[253] = [255, 0, 255]; // Magenta
        colors[254] = [0, 255, 255]; // Cyan
        colors[255] = [255, 255, 255]; // White
        Self { colors }
    }

    /// Parse TS_UPDATE_PALETTE_DATA and update palette entries.
    /// Wire format: updateType(u16) + pad(u16) + numberColors(u32) + N x RGB triplet.
    pub(crate) fn process_update(&mut self, data: &[u8]) {
        const UPDATE_TYPE_PALETTE: u16 = 0x0002;
        const PALETTE_ENTRY_COUNT: usize = 256;
        const PALETTE_ENTRY_SIZE: usize = 3;

        if data.len() < 8 {
            warn!("Palette update too short: {} bytes", data.len());
            return;
        }

        let update_type = u16::from_le_bytes([data[0], data[1]]);
        if update_type != UPDATE_TYPE_PALETTE {
            warn!(update_type, "Ignoring palette update with unexpected type");
            return;
        }

        let number_colors = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if number_colors != 256 {
            warn!(number_colors, "Ignoring palette update with unexpected entry count");
            return;
        }

        let entry_data = &data[8..];
        let required_len = PALETTE_ENTRY_COUNT * PALETTE_ENTRY_SIZE;
        if entry_data.len() < required_len {
            warn!(
                "Palette data truncated: expected {} bytes for {} colors, got {}",
                required_len,
                PALETTE_ENTRY_COUNT,
                entry_data.len()
            );
            return;
        }

        for (color, rgb) in self.colors.iter_mut().zip(entry_data.chunks_exact(PALETTE_ENTRY_SIZE)) {
            *color = [rgb[0], rgb[1], rgb[2]];
        }

        debug!("Updated palette with {} colors", PALETTE_ENTRY_COUNT);
    }

    /// Borrow the underlying color table for bitmap application.
    pub(crate) fn colors(&self) -> &[[u8; 3]; 256] {
        &self.colors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_spec_palette_data() {
        let mut data = vec![0; 8 + 256 * 3];
        data[0..2].copy_from_slice(&0x0002u16.to_le_bytes());
        data[4..8].copy_from_slice(&256u32.to_le_bytes());
        data[8..11].copy_from_slice(&[0x10, 0x20, 0x30]);
        data[11..14].copy_from_slice(&[0x40, 0x50, 0x60]);

        let mut palette = Palette::system_default();
        palette.process_update(&data);

        assert_eq!(palette.colors()[0], [0x10, 0x20, 0x30]);
        assert_eq!(palette.colors()[1], [0x40, 0x50, 0x60]);
    }
}

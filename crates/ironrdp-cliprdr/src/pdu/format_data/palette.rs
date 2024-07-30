use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{impl_pdu_pod, PduDecode, PduEncode, PduResult};

/// Represents `PALETTEENTRY`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub extra: u8,
}

impl PaletteEntry {
    const SIZE: usize = 1 /* R */ + 1 /* G */ + 1 /* B */ + 1 /* extra */;
}

/// Represents `CLIPRDR_PALETTE`
///
/// NOTE: `PduDecode` implementation will read all remaining data in cursor as the palette entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPalette {
    pub entries: Vec<PaletteEntry>,
}

impl_pdu_pod!(ClipboardPalette);

impl ClipboardPalette {
    const NAME: &'static str = "CLIPRDR_PALETTE";
}

impl PduEncode for ClipboardPalette {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        for entry in &self.entries {
            dst.write_u8(entry.red);
            dst.write_u8(entry.green);
            dst.write_u8(entry.blue);
            dst.write_u8(entry.extra);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.entries.len() * PaletteEntry::SIZE
    }
}

impl<'de> PduDecode<'de> for ClipboardPalette {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let entries_count = src.len() / PaletteEntry::SIZE;

        let mut entries = Vec::with_capacity(entries_count);
        for _ in 0..entries_count {
            let red = src.read_u8();
            let green = src.read_u8();
            let blue = src.read_u8();
            let extra = src.read_u8();

            entries.push(PaletteEntry {
                red,
                green,
                blue,
                extra,
            });
        }

        Ok(Self { entries })
    }
}

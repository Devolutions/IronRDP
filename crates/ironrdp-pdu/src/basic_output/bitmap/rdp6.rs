use crate::{Error as PduError, PduDecode, PduEncode, ReadCursor, Result as PduResult, WriteCursor};

const NON_RLE_PADDING_SIZE: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPlanes<'a> {
    Argb {
        data: &'a [u8],
    },
    AYCoCg {
        color_loss_level: u8,
        use_chroma_subsampling: bool,
        data: &'a [u8],
    },
}

/// Represents `RDP6_BITMAP_STREAM` structure described in [MS-RDPEGDI] 2.2.2.5.1
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapStream<'a> {
    pub enable_rle_compression: bool,
    pub use_alpha: bool,
    pub color_planes: ColorPlanes<'a>,
}

impl<'a> BitmapStream<'a> {
    pub const NAME: &'static str = "Rdp6BitmapStream";
    const FIXED_PART_SIZE: usize = 1;

    pub fn color_panes_data(&self) -> &'a [u8] {
        match self.color_planes {
            ColorPlanes::Argb { data } => data,
            ColorPlanes::AYCoCg { data, .. } => data,
        }
    }

    pub fn has_subsampled_chroma(&self) -> bool {
        match self.color_planes {
            ColorPlanes::Argb { .. } => false,
            ColorPlanes::AYCoCg {
                use_chroma_subsampling, ..
            } => use_chroma_subsampling,
        }
    }
}

impl<'a> PduDecode<'a> for BitmapStream<'a> {
    fn decode(src: &mut ReadCursor<'a>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);
        let header = src.read_u8();

        let color_loss_level = header & 0x07;
        let use_chroma_subsampling = (header & 0x08) != 0;
        let enable_rle_compression = (header & 0x10) != 0;
        let use_alpha = (header & 0x20) == 0;

        let color_planes_size = if !enable_rle_compression {
            // Cut padding field if RLE flags is set to 0
            if src.is_empty() {
                return Err(PduError::Other {
                    context: Self::NAME,
                    reason: "Missing padding byte from zero-size Non-RLE bitmap data",
                });
            }
            src.len() - NON_RLE_PADDING_SIZE
        } else {
            src.len()
        };

        let color_planes_data = src.peek_slice(color_planes_size);

        let color_planes = match color_loss_level {
            0 => {
                // ARGB color planes
                ColorPlanes::Argb {
                    data: color_planes_data,
                }
            }
            color_loss_level => ColorPlanes::AYCoCg {
                color_loss_level,
                use_chroma_subsampling,
                data: color_planes_data,
            },
        };

        Ok(Self {
            enable_rle_compression,
            use_alpha,
            color_planes,
        })
    }
}

impl<'a> PduEncode for BitmapStream<'a> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let mut header = ((self.enable_rle_compression as u8) << 4) | ((!self.use_alpha as u8) << 5);

        match self.color_planes {
            ColorPlanes::Argb { .. } => {
                // ARGB color planes keep cll and cs flags set to 0
            }
            ColorPlanes::AYCoCg {
                color_loss_level,
                use_chroma_subsampling,
                ..
            } => {
                // Add cll and cs flags to header
                header |= (color_loss_level & 0x07) | ((use_chroma_subsampling as u8) << 3);
            }
        }

        ensure_size!(in: dst, size: self.size());

        dst.write_u8(header);
        dst.write_slice(self.color_panes_data());

        // Write padding
        if !self.enable_rle_compression {
            dst.write_u8(0);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        if self.enable_rle_compression {
            Self::FIXED_PART_SIZE + self.color_panes_data().len()
        } else {
            Self::FIXED_PART_SIZE + NON_RLE_PADDING_SIZE + self.color_panes_data().len()
        }
    }
}

#[cfg(test)]
mod tests {
    use expect_test::{expect, Expect};

    use super::*;
    use crate::{decode, encode_buf, PduEncode};

    fn assert_roundtrip(buffer: &[u8], expected: Expect) {
        let pdu = decode::<BitmapStream>(buffer).unwrap();
        expected.assert_debug_eq(&pdu);
        assert_eq!(pdu.size(), buffer.len());
        let mut reencoded = vec![];
        encode_buf(&pdu, &mut reencoded).unwrap();
        assert_eq!(reencoded.as_slice(), buffer);
    }

    fn assert_parsing_failure(buffer: &[u8], expected: Expect) {
        let error = decode::<BitmapStream>(buffer).err().unwrap();
        expected.assert_debug_eq(&error);
    }

    #[test]
    fn parsing_valid_data_succeeds() {
        // AYCoCg color planes, with RLE
        assert_roundtrip(
            &[0x3F, 0x01, 0x02, 0x03, 0x04],
            expect![[r#"
                BitmapStream {
                    enable_rle_compression: true,
                    use_alpha: false,
                    color_planes: AYCoCg {
                        color_loss_level: 7,
                        use_chroma_subsampling: true,
                        data: [
                            1,
                            2,
                            3,
                            4,
                        ],
                    },
                }
            "#]],
        );

        // RGB color planes, with RLE, with alpha
        assert_roundtrip(
            &[0x10, 0x01, 0x02, 0x03, 0x04],
            expect![[r#"
                BitmapStream {
                    enable_rle_compression: true,
                    use_alpha: true,
                    color_planes: Argb {
                        data: [
                            1,
                            2,
                            3,
                            4,
                        ],
                    },
                }
            "#]],
        );

        // Without RLE, validate that padding is handled correctly
        assert_roundtrip(
            &[0x20, 0x01, 0x02, 0x03, 0x00],
            expect![[r#"
                BitmapStream {
                    enable_rle_compression: false,
                    use_alpha: false,
                    color_planes: Argb {
                        data: [
                            1,
                            2,
                            3,
                        ],
                    },
                }
            "#]],
        );

        // Empty color planes, with RLE
        assert_roundtrip(
            &[0x10],
            expect![[r#"
                BitmapStream {
                    enable_rle_compression: true,
                    use_alpha: true,
                    color_planes: Argb {
                        data: [],
                    },
                }
            "#]],
        );

        // Empty color planes, without RLE
        assert_roundtrip(
            &[0x00, 0x00],
            expect![[r#"
                BitmapStream {
                    enable_rle_compression: false,
                    use_alpha: true,
                    color_planes: Argb {
                        data: [],
                    },
                }
            "#]],
        );
    }

    #[test]
    pub fn failures_handled_gracefully() {
        // Empty buffer
        assert_parsing_failure(
            &[],
            expect![[r#"
                NotEnoughBytes {
                    name: "Rdp6BitmapStream",
                    received: 0,
                    expected: 1,
                }
            "#]],
        );

        // Without RLE, Check that missing padding byte is handled correctly
        assert_parsing_failure(
            &[0x20],
            expect![[r#"
                Other {
                    context: "Rdp6BitmapStream",
                    reason: "Missing padding byte from zero-size Non-RLE bitmap data",
                }
            "#]],
        );
    }
}

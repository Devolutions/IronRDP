use std::cmp::{max, min};

use crate::cursor::ReadCursor;
use crate::{PduDecode, PduEncode, PduResult};

pub(crate) mod private {
    pub struct BaseRectangle {
        pub left: u16,
        pub top: u16,
        pub right: u16,
        pub bottom: u16,
    }

    impl BaseRectangle {
        pub fn empty() -> Self {
            Self {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            }
        }
    }

    pub trait RectangleImpl: Sized {
        fn from_base(rect: BaseRectangle) -> Self;
        fn to_base(&self) -> BaseRectangle;

        fn left(&self) -> u16;
        fn top(&self) -> u16;
        fn right(&self) -> u16;
        fn bottom(&self) -> u16;
    }
}

use private::*;

pub trait Rectangle: RectangleImpl {
    fn width(&self) -> u16;
    fn height(&self) -> u16;

    fn empty() -> Self {
        Self::from_base(BaseRectangle::empty())
    }

    fn union_all(rectangles: &[Self]) -> Self {
        Self::from_base(BaseRectangle {
            left: rectangles.iter().map(|r| r.left()).min().unwrap_or(0),
            top: rectangles.iter().map(|r| r.top()).min().unwrap_or(0),
            right: rectangles.iter().map(|r| r.right()).max().unwrap_or(0),
            bottom: rectangles.iter().map(|r| r.bottom()).max().unwrap_or(0),
        })
    }

    fn intersect(&self, other: &Self) -> Option<Self> {
        let a = self.to_base();
        let b = other.to_base();

        let result = BaseRectangle {
            left: max(a.left, b.left),
            top: max(a.top, b.top),
            right: min(a.right, b.right),
            bottom: min(a.bottom, b.bottom),
        };

        if result.left < result.right && result.top < result.bottom {
            Some(Self::from_base(result))
        } else {
            None
        }
    }

    #[must_use]
    fn union(&self, other: &Self) -> Self {
        let a = self.to_base();
        let b = other.to_base();

        let result = BaseRectangle {
            left: min(a.left, b.left),
            top: min(a.top, b.top),
            right: max(a.right, b.right),
            bottom: max(a.bottom, b.bottom),
        };

        Self::from_base(result)
    }
}

/// An **inclusive** rectangle.
///
/// This struct is defined as an **inclusive** rectangle.
/// That is, the pixel at coordinate (right, bottom) is included in the rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InclusiveRectangle {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

/// An **exclusive** rectangle.
/// This struct is defined as an **exclusive** rectangle.
/// That is, the pixel at coordinate (right, bottom) is not included in the rectangle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExclusiveRectangle {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

macro_rules! impl_rectangle {
    ($type:ty) => {
        impl RectangleImpl for $type {
            fn from_base(rect: BaseRectangle) -> Self {
                Self {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                }
            }

            fn to_base(&self) -> BaseRectangle {
                BaseRectangle {
                    left: self.left,
                    top: self.top,
                    right: self.right,
                    bottom: self.bottom,
                }
            }

            fn left(&self) -> u16 {
                self.left
            }
            fn top(&self) -> u16 {
                self.top
            }
            fn right(&self) -> u16 {
                self.right
            }
            fn bottom(&self) -> u16 {
                self.bottom
            }
        }
    };
}

impl_rectangle!(InclusiveRectangle);
impl_rectangle!(ExclusiveRectangle);

impl Rectangle for InclusiveRectangle {
    fn width(&self) -> u16 {
        self.right - self.left + 1
    }

    fn height(&self) -> u16 {
        self.bottom - self.top + 1
    }
}

impl Rectangle for ExclusiveRectangle {
    fn width(&self) -> u16 {
        self.right - self.left
    }

    fn height(&self) -> u16 {
        self.bottom - self.top
    }
}

impl InclusiveRectangle {
    const NAME: &str = "InclusiveRectangle";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 4;

    pub const ENCODED_SIZE: usize = Self::FIXED_PART_SIZE;
}

impl PduEncode for InclusiveRectangle {
    fn encode(&self, dst: &mut crate::cursor::WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.left);
        dst.write_u16(self.top);
        dst.write_u16(self.right);
        dst.write_u16(self.bottom);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for InclusiveRectangle {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let left = src.read_u16();
        let top = src.read_u16();
        let right = src.read_u16();
        let bottom = src.read_u16();

        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }
}

impl ExclusiveRectangle {
    const NAME: &str = "ExclusiveRectangle";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 4;

    pub const ENCODED_SIZE: usize = Self::FIXED_PART_SIZE;
}

impl PduEncode for ExclusiveRectangle {
    fn encode(&self, dst: &mut crate::cursor::WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.left);
        dst.write_u16(self.top);
        dst.write_u16(self.right);
        dst.write_u16(self.bottom);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for ExclusiveRectangle {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let left = src.read_u16();
        let top = src.read_u16();
        let right = src.read_u16();
        let bottom = src.read_u16();

        Ok(Self {
            left,
            top,
            right,
            bottom,
        })
    }
}

// Legacy code for serializing/deserializing exclusive rectangles as inclusive rectangles structure
// TODO(@pacmancoder) this should be removed later
mod legacy {
    use std::io;

    use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

    use super::*;

    impl InclusiveRectangle {
        // TODO: clarify code related to rectangles (inclusive vs exclusive bounds, …)
        // See for instance:
        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/84a3d4d2-5523-4e49-9a48-33952c559485
        // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/776dbdaf-7619-45fd-9a90-ebfd07802b24
        // Rename / new structs "InclusiveRect" / "ExclusiveRect"…
        // Also, avoid / audit mixed usage of "Rectangle" with "RfxRectangle"
        // We should be careful when manipulating structs with slight nuances like this.

        pub fn from_buffer_exclusive(mut stream: impl io::Read) -> Result<Self, io::Error> {
            let left = stream.read_u16::<LittleEndian>()?;
            let top = stream.read_u16::<LittleEndian>()?;
            let right = stream
                .read_u16::<LittleEndian>()?
                .checked_sub(1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "invalid exclusive right bound"))?;
            let bottom = stream
                .read_u16::<LittleEndian>()?
                .checked_sub(1)
                .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "invalid exclusive bottom bound"))?;

            Ok(Self {
                left,
                top,
                right,
                bottom,
            })
        }

        pub fn to_buffer_exclusive(&self, mut stream: impl io::Write) -> Result<(), io::Error> {
            stream.write_u16::<LittleEndian>(self.left)?;
            stream.write_u16::<LittleEndian>(self.top)?;
            stream.write_u16::<LittleEndian>(self.right + 1)?;
            stream.write_u16::<LittleEndian>(self.bottom + 1)?;

            Ok(())
        }
    }

    impl crate::PduParsing for InclusiveRectangle {
        type Error = io::Error;

        fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
            let left = stream.read_u16::<LittleEndian>()?;
            let top = stream.read_u16::<LittleEndian>()?;
            let right = stream.read_u16::<LittleEndian>()?;
            let bottom = stream.read_u16::<LittleEndian>()?;

            Ok(Self {
                left,
                top,
                right,
                bottom,
            })
        }

        fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
            stream.write_u16::<LittleEndian>(self.left)?;
            stream.write_u16::<LittleEndian>(self.top)?;
            stream.write_u16::<LittleEndian>(self.right)?;
            stream.write_u16::<LittleEndian>(self.bottom)?;

            Ok(())
        }

        fn buffer_length(&self) -> usize {
            8
        }
    }
}

use std::cmp::{max, min};
use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rectangle {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl Rectangle {
    pub fn empty() -> Self {
        Self {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }
    }

    pub fn width(&self) -> u16 {
        self.right - self.left + 1
    }

    pub fn height(&self) -> u16 {
        self.bottom - self.top + 1
    }

    pub fn union_all(rectangles: &[Self]) -> Self {
        Self {
            left: rectangles.iter().map(|r| r.left).min().unwrap_or(0),
            top: rectangles.iter().map(|r| r.top).min().unwrap_or(0),
            right: rectangles.iter().map(|r| r.right).max().unwrap_or(0),
            bottom: rectangles.iter().map(|r| r.bottom).max().unwrap_or(0),
        }
    }

    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let result = Self {
            left: max(self.left, other.left),
            top: max(self.top, other.top),
            right: min(self.right, other.right),
            bottom: min(self.bottom, other.bottom),
        };

        if result.left < result.right && result.top < result.bottom {
            Some(result)
        } else {
            None
        }
    }

    pub fn union(&self, other: &Self) -> Self {
        Self {
            left: min(self.left, other.left),
            top: min(self.top, other.top),
            right: max(self.right, other.right),
            bottom: max(self.bottom, other.bottom),
        }
    }
}

impl crate::PduParsing for Rectangle {
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

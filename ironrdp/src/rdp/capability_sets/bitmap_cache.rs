#[cfg(test)]
mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::{rdp::CapabilitySetsError, PduParsing};

pub const BITMAP_CACHE_ENTRIES_NUM: usize = 3;

const BITMAP_CACHE_LENGTH: usize = 36;
const BITMAP_CACHE_REV2_LENGTH: usize = 36;
const CELL_INFO_LENGTH: usize = 4;
const BITMAP_CACHE_REV2_CELL_INFO_NUM: usize = 5;
const CACHE_ENTRY_LENGTH: usize = 4;

#[derive(Debug, PartialEq, Clone)]
pub struct BitmapCache {
    pub caches: [CacheEntry; BITMAP_CACHE_ENTRIES_NUM],
}

impl PduParsing for BitmapCache {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;

        let mut caches = [CacheEntry::default(); BITMAP_CACHE_ENTRIES_NUM];

        for cache in caches.iter_mut() {
            *cache = CacheEntry::from_buffer(&mut buffer)?;
        }

        Ok(BitmapCache { caches })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding

        for cache in self.caches.iter() {
            cache.to_buffer(&mut buffer)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BITMAP_CACHE_LENGTH
    }
}

#[derive(Debug, PartialEq, Copy, Clone, Default)]
pub struct CacheEntry {
    pub entries: u16,
    pub max_cell_size: u16,
}

impl PduParsing for CacheEntry {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let entries = buffer.read_u16::<LittleEndian>()?;
        let max_cell_size = buffer.read_u16::<LittleEndian>()?;

        Ok(CacheEntry {
            entries,
            max_cell_size,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.entries)?;
        buffer.write_u16::<LittleEndian>(self.max_cell_size)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CACHE_ENTRY_LENGTH
    }
}

bitflags! {
    pub struct CacheFlags: u16 {
        const PERSISTENT_KEYS_EXPECTED_FLAG = 1;
        const ALLOW_CACHE_WAITING_LIST_FLAG = 2;
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct BitmapCacheRev2 {
    pub cache_flags: CacheFlags,
    pub num_cell_caches: u8,
    pub cache_cell_info: [CellInfo; BITMAP_CACHE_REV2_CELL_INFO_NUM],
}

impl PduParsing for BitmapCacheRev2 {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let cache_flags = CacheFlags::from_bits_truncate(buffer.read_u16::<LittleEndian>()?);
        let _padding = buffer.read_u8()?;
        let num_cell_caches = buffer.read_u8()?;

        let mut cache_cell_info = [CellInfo::default(); BITMAP_CACHE_REV2_CELL_INFO_NUM];

        for cell in cache_cell_info.iter_mut() {
            *cell = CellInfo::from_buffer(&mut buffer)?;
        }

        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;
        let _padding = buffer.read_u32::<LittleEndian>()?;

        Ok(BitmapCacheRev2 {
            cache_flags,
            num_cell_caches,
            cache_cell_info,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u16::<LittleEndian>(self.cache_flags.bits())?;
        buffer.write_u8(0)?; // padding
        buffer.write_u8(self.num_cell_caches)?;

        for cell_info in self.cache_cell_info.iter() {
            cell_info.to_buffer(&mut buffer)?;
        }

        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding
        buffer.write_u32::<LittleEndian>(0)?; // padding

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BITMAP_CACHE_REV2_LENGTH
    }
}

#[derive(Debug, PartialEq, Copy, Clone, Default)]
pub struct CellInfo {
    pub num_entries: u32,
    pub is_cache_persistent: bool,
}

impl PduParsing for CellInfo {
    type Error = CapabilitySetsError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let cell_info = buffer.read_u32::<LittleEndian>()?;

        let num_entries = cell_info & !(1 << 31);
        let is_cache_persistent = cell_info >> 31 != 0;

        Ok(CellInfo {
            num_entries,
            is_cache_persistent,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        let mut data = self.num_entries;

        if self.is_cache_persistent {
            data |= 1 << 31;
        }

        buffer.write_u32::<LittleEndian>(data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CELL_INFO_LENGTH
    }
}

//! Persistent bitmap cache ([MS-RDPEGFX] [3.3.1.5]).
//!
//! The bitmap cache ([3.3.1.4]) holds tiles the server stored with `SurfaceToCache` and pastes
//! with `CacheToSurface`. A client that keeps those tiles across sessions offers their keys
//! in `RDPGFX_CACHE_IMPORT_OFFER_PDU` ([2.2.2.16]) right after capability negotiation; the
//! server answers with `RDPGFX_CACHE_IMPORT_REPLY_PDU` ([2.2.2.17]), naming the slot each
//! accepted entry goes into, and from then on pastes them without uploading them first.
//!
//! [`GraphicsPipelineClient`](crate::client::GraphicsPipelineClient) drives that exchange; where
//! the entries live is up to the embedder, which supplies a [`PersistentCacheStore`].
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
//! [2.2.2.16]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/890f0077-dedb-4b22-8b20-ea69b9cfcacd
//! [2.2.2.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/0c4d88f8-50dc-465a-ab00-88a3fe0ec3c5
//! [3.3.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/d1c1540f-9bc7-4344-81e0-3cce6b845f86
//! [3.3.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/1a2abd79-889c-4207-a1c3-c8c1e1d6f117

use crate::pdu::CacheEntryMetadata;

/// The most entries one `RDPGFX_CACHE_IMPORT_OFFER_PDU` may carry: 2.2.2.16 requires
/// `cacheEntriesCount` to be less than 5462.
pub const MAX_CACHE_IMPORT_ENTRIES: usize = 5461;

/// The largest entry offered or stored, in bytes: a 128 x 128 tile.
///
/// The protocol sets no limit, but mstsc never offers a larger entry, so it is the largest size
/// Windows servers are known to import.
pub const MAX_CACHE_IMPORT_ENTRY_BYTES: usize = 64 * 1024;

/// Cache slots and bytes available with the full bitmap cache (3.3.1.4).
pub const CACHE_SLOTS: u16 = 25600;
/// Bitmap cache capacity, in bytes, without `SMALL_CACHE` or `THINCLIENT` (3.3.1.4).
pub const CACHE_BYTES: usize = 100 * 1024 * 1024;
/// Cache slots available when the server confirms `SMALL_CACHE`, `THINCLIENT` or version 10.3.
pub const SMALL_CACHE_SLOTS: u16 = 4096;
/// Bitmap cache capacity, in bytes, when the server confirms `SMALL_CACHE`, `THINCLIENT` or
/// version 10.3.
pub const SMALL_CACHE_BYTES: usize = 16 * 1024 * 1024;

/// Error a [`PersistentCacheStore`] reports. The client logs it and carries on without the
/// persistent cache; it never ends the session.
pub type PersistentCacheError = Box<dyn core::error::Error + Send + Sync>;

/// A bitmap cache entry read back from a [`PersistentCacheStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentCacheEntry {
    /// Server-assigned key (`cacheKey` of `RDPGFX_SURFACE_TO_CACHE_PDU`, 2.2.2.6).
    pub cache_key: u64,
    /// Width, in pixels.
    pub width: u16,
    /// Height, in pixels.
    pub height: u16,
    /// RGBA8888 pixels, row-major, exactly `width * height * 4` bytes.
    pub data: Vec<u8>,
}

/// A bitmap cache entry handed to [`PersistentCacheStore::save`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedBitmap<'a> {
    /// Server-assigned key.
    pub cache_key: u64,
    /// Width, in pixels.
    pub width: u16,
    /// Height, in pixels.
    pub height: u16,
    /// RGBA8888 pixels, row-major, exactly `width * height * 4` bytes.
    pub data: &'a [u8],
}

/// Storage for the persistent bitmap cache, supplied by the embedder.
///
/// Every method runs on the thread processing the graphics channel, so an implementation that
/// does I/O delays rendering for as long as it takes. [`Self::entries`] runs once per
/// connection, after capability negotiation; [`Self::load`] runs once per entry the server
/// accepts, before the first frame that may use it; [`Self::save`] runs when the channel closes.
pub trait PersistentCacheStore: Send {
    /// List the stored entries, the most valuable first.
    ///
    /// The client offers the entries that fit, in this order: it skips an entry larger than
    /// [`MAX_CACHE_IMPORT_ENTRY_BYTES`] or that would take the offer past the confirmed cache's
    /// byte capacity and keeps going with the smaller ones, and stops at
    /// [`MAX_CACHE_IMPORT_ENTRIES`] or at the cache's slot count.
    fn entries(&mut self) -> Result<Vec<CacheEntryMetadata>, PersistentCacheError>;

    /// Read the entry stored under `cache_key`, or `None` if it is gone.
    fn load(&mut self, cache_key: u64) -> Result<Option<PersistentCacheEntry>, PersistentCacheError>;

    /// Called once the server's reply has been processed and no further [`Self::load`] follows,
    /// so the store can release what it held open for the import.
    fn import_finished(&mut self) {}

    /// Replace the stored entries with `entries`, the bitmap cache's content when the channel
    /// closes, most recently used first, without the entries that could never be offered.
    fn save(&mut self, entries: &[CachedBitmap<'_>]) -> Result<(), PersistentCacheError>;
}

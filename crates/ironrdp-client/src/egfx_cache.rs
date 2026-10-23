//! File-backed persistent bitmap cache for the graphics pipeline ([MS-RDPEGFX] 3.3.1.5).
//!
//! The file has the format mstsc gives its `CacheNNNN.bin` files, which FreeRDP also reads and
//! writes, so a cache file moves between the three clients. It holds the bitmap cache's tiles,
//! most recently used first, until the end of the file:
//!
//! ```text
//! signature  8 bytes   "RDP8bmp\0"
//! version    u32 LE    6
//! then, per entry:
//!   cache_key  u64 LE
//!   width      u16 LE
//!   height     u16 LE
//!   pixels     width * height * 4 bytes, rows top to bottom, each pixel B, G, R, A
//! ```
//!
//! mstsc refuses a whole file holding an entry with key 0 or with pixels outside 4 to 65,536
//! bytes, so such entries are never written.
//!
//! Saving writes a sibling temporary file and renames it over the cache, so a crash mid-save
//! leaves the previous cache intact and concurrent sessions never read a half-written file.
//!
//! One cache serves every server the user connects to and the last session to save wins, with no
//! merging. This is what mstsc does: each session writes a new `CacheNNNN.bin` under
//! `%LOCALAPPDATA%\Microsoft\Terminal Server Client\Cache`, and the next connection imports the
//! newest one. The keys are hashes of the bitmap content, so a tile cached for one server is a hit
//! on another that paints the same pixels.
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0

use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use ironrdp_egfx::cache::{
    CACHE_BYTES, CACHE_SLOTS, CachedBitmap, MAX_CACHE_IMPORT_ENTRY_BYTES, PersistentCacheEntry, PersistentCacheError,
    PersistentCacheStore,
};
use ironrdp_egfx::pdu::CacheEntryMetadata;

/// The signature and version mstsc writes and requires, byte for byte.
const FILE_HEADER: &[u8; 12] = b"RDP8bmp\0\x06\0\0\0";
const FILE_HEADER_SIZE: u64 = 8 /* signature */ + 4 /* version */;
const ENTRY_HEADER_SIZE: u64 = 8 /* cache_key */ + 2 /* width */ + 2 /* height */;

/// Distinguishes the temporary files of saves running at once in one process.
static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where an entry's pixels sit in the cache file.
#[derive(Debug, Clone, Copy)]
struct EntryLocation {
    offset: u64,
    width: u16,
    height: u16,
}

/// A [`PersistentCacheStore`] kept in one file.
#[derive(Debug)]
pub struct FilePersistentCache {
    path: PathBuf,
    /// Entries found by the last [`PersistentCacheStore::entries`] call, until the import finishes.
    index: HashMap<u64, EntryLocation>,
    /// The file `index` describes, kept open with it so loads read what was listed.
    reader: Option<File>,
}

impl FilePersistentCache {
    /// A cache stored at `path`. Nothing is read or written until the graphics channel asks.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            index: HashMap::new(),
            reader: None,
        }
    }

    /// Forget the listed entries and close the file they were measured in. Both go together, so
    /// an indexed entry always has its file.
    fn discard_import(&mut self) {
        self.index.clear();
        self.reader = None;
    }

    fn read_index(&mut self) -> io::Result<Vec<CacheEntryMetadata>> {
        self.discard_import();

        let file = match open_shared(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        let mut header = [0; 12];
        reader.read_exact(&mut header)?;
        if &header != FILE_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a version 6 RDP8bmp graphics cache file",
            ));
        }

        let mut offset = FILE_HEADER_SIZE;
        let mut entries = Vec::new();
        // No cache holds more tiles than it has slots, so nothing past that is ever offered.
        while entries.len() < usize::from(CACHE_SLOTS) {
            if file_len < offset + ENTRY_HEADER_SIZE {
                break;
            }
            let cache_key = read_u64(&mut reader)?;
            let width = read_u16(&mut reader)?;
            let height = read_u16(&mut reader)?;
            let len = u64::from(width) * u64::from(height) * 4;
            let data_offset = offset + ENTRY_HEADER_SIZE;

            // A length past the cache's capacity or past the end of the file means corruption;
            // keep the entries read so far.
            if len == 0 || u64::try_from(CACHE_BYTES).expect("fits") < len || file_len < data_offset + len {
                break;
            }

            reader.seek_relative(i64::try_from(len).expect("bounded by CACHE_BYTES"))?;
            offset = data_offset + len;

            // Key 0 is how mstsc marks an invalid entry; skip it rather than offer it.
            if cache_key == 0 || self.index.contains_key(&cache_key) {
                continue;
            }
            self.index.insert(
                cache_key,
                EntryLocation {
                    offset: data_offset,
                    width,
                    height,
                },
            );
            entries.push(CacheEntryMetadata {
                cache_key,
                bitmap_len: u32::try_from(len).expect("bounded by CACHE_BYTES"),
            });
        }

        self.reader = Some(reader.into_inner());
        Ok(entries)
    }

    fn read_entry(&mut self, cache_key: u64) -> io::Result<Option<PersistentCacheEntry>> {
        let Some(location) = self.index.get(&cache_key).copied() else {
            return Ok(None);
        };
        // INVARIANT: `index` is filled together with `reader` and cleared with it, so an indexed
        // entry always has the file it was measured in.
        let Some(file) = self.reader.as_mut() else {
            return Ok(None);
        };

        let mut data = vec![0; usize::from(location.width) * usize::from(location.height) * 4];
        file.seek(SeekFrom::Start(location.offset))?;
        file.read_exact(&mut data)?;
        swap_red_and_blue(&mut data);

        Ok(Some(PersistentCacheEntry {
            cache_key,
            width: location.width,
            height: location.height,
            data,
        }))
    }

    fn write(&mut self, entries: &[CachedBitmap<'_>]) -> io::Result<()> {
        // The rename below fails on Windows while this process still holds the file open.
        self.discard_import();

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        // UNTESTED: sessions sharing one cache file never observe a half-written file, because
        // each writes a temporary of its own (unique per process and per save) and renames it into
        // place; the last save wins. This matters since the default path is per user, not per
        // connection. When changing how the file is written, keep the unique temporary and the
        // rename, and the reader drop above, without which Windows refuses the rename.
        let mut temp_path = self.path.clone().into_os_string();
        temp_path.push(format!(
            ".{}-{}.tmp",
            std::process::id(),
            SAVE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let temp_path = PathBuf::from(temp_path);

        // Created before the cleanup below takes over, so a failed exclusive create never removes
        // a file this save did not make.
        let temp_file = create_private(&temp_path)?;
        let result = (|| {
            let mut writer = BufWriter::new(temp_file);
            writer.write_all(FILE_HEADER)?;
            let mut pixels = Vec::new();
            for entry in entries {
                writer.write_all(&entry.cache_key.to_le_bytes())?;
                writer.write_all(&entry.width.to_le_bytes())?;
                writer.write_all(&entry.height.to_le_bytes())?;
                pixels.clear();
                pixels.extend_from_slice(entry.data);
                swap_red_and_blue(&mut pixels);
                writer.write_all(&pixels)?;
            }
            writer
                .into_inner()
                .map_err(io::IntoInnerError::into_error)?
                .sync_all()?;
            fs::rename(&temp_path, &self.path)
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }
}

impl PersistentCacheStore for FilePersistentCache {
    fn entries(&mut self) -> Result<Vec<CacheEntryMetadata>, PersistentCacheError> {
        Ok(self.read_index()?)
    }

    fn load(&mut self, cache_key: u64) -> Result<Option<PersistentCacheEntry>, PersistentCacheError> {
        Ok(self.read_entry(cache_key)?)
    }

    fn import_finished(&mut self) {
        self.discard_import();
    }

    /// Replace the file with `entries`, keeping only what mstsc reads back: a non-zero key, non-empty
    /// pixels of exactly `width * height * 4` bytes, at most [`MAX_CACHE_IMPORT_ENTRY_BYTES`] each,
    /// the first entry of each key. With nothing left to write, the existing file is kept.
    fn save(&mut self, entries: &[CachedBitmap<'_>]) -> Result<(), PersistentCacheError> {
        let max_len = u64::try_from(MAX_CACHE_IMPORT_ENTRY_BYTES).expect("fits");
        let mut keys = HashSet::new();
        let entries: Vec<CachedBitmap<'_>> = entries
            .iter()
            .filter(|entry| {
                let len = u64::from(entry.width) * u64::from(entry.height) * 4;
                entry.cache_key != 0 && 0 < len && len <= max_len && u64::try_from(entry.data.len()) == Ok(len)
            })
            .filter(|entry| keys.insert(entry.cache_key))
            .copied()
            .collect();
        if entries.is_empty() {
            return Ok(());
        }
        Ok(self.write(&entries)?)
    }
}

/// Open the cache file for reading without stopping another session from replacing it.
///
/// On Windows a plain open denies deletion, so another session's save could not rename its file
/// over this one while an import is reading it.
fn open_shared(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;

        const FILE_SHARE_READ: u32 = 0x1;
        const FILE_SHARE_WRITE: u32 = 0x2;
        const FILE_SHARE_DELETE: u32 = 0x4;
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
    }
    options.open(path)
}

/// Create a new file only its owner can read, failing if it exists.
///
/// The cache holds desktop pixels, so on Unix it must not inherit a umask that lets other local
/// users read it; the rename keeps the mode set here. On Windows the file inherits the ACL of its
/// directory, which for the default path is the user's local application data.
fn create_private(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        options.mode(0o600);
    }
    options.open(path)
}

/// The default location of the graphics cache file for the current user: `egfx-cache.bin` under
/// `%LOCALAPPDATA%\IronRDP` on Windows, and under `$XDG_CACHE_HOME/ironrdp` (or `$HOME/.cache/ironrdp`
/// when `XDG_CACHE_HOME` is unset or not absolute) elsewhere.
///
/// A variable that is empty or holds a relative path is ignored, so the cache never lands in the
/// process's working directory; `None` when no usable location exists.
pub fn default_cache_path() -> Option<PathBuf> {
    const FILE_NAME: &str = "egfx-cache.bin";

    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };

    #[cfg(windows)]
    {
        absolute("LOCALAPPDATA").map(|dir| dir.join("IronRDP").join(FILE_NAME))
    }

    #[cfg(not(windows))]
    {
        absolute("XDG_CACHE_HOME")
            .or_else(|| absolute("HOME").map(|home| home.join(".cache")))
            .map(|dir| dir.join("ironrdp").join(FILE_NAME))
    }
}

/// Turn RGBA pixels into the file's BGRA, or back: the swap is its own inverse.
fn swap_red_and_blue(pixels: &mut [u8]) {
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

fn read_u16(reader: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

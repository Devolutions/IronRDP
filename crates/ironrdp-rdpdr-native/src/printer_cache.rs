//! File-backed [`RdpdrPrinterCacheStore`].

use core::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use ironrdp_rdpdr::{RdpdrPrinterCacheEntry, RdpdrPrinterCacheResult, RdpdrPrinterCacheStore};

const MAGIC: &[u8; 5] = b"IRPC\x02";
const EXTENSION: &str = "prncache";
const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024;

/// Stores each printer's cache entry as one file in a directory.
///
/// The directory is created on the first save. File names are a hash of the printer name, and
/// each file records the full name, so a hash collision reads as a missing entry. Writes go to a
/// temporary file that replaces the entry, so a crash leaves the previous entry intact.
///
/// Use one directory per server: [MS-RDPEPC] cache data describes the server's queue.
///
/// [MS-RDPEPC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepc/
pub struct FileRdpdrPrinterCacheStore {
    directory: PathBuf,
    // Serializes read-modify-write sequences from one process.
    lock: Mutex<()>,
}

impl fmt::Debug for FileRdpdrPrinterCacheStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRdpdrPrinterCacheStore")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl FileRdpdrPrinterCacheStore {
    /// Creates a store that keeps entries in `directory`.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            lock: Mutex::new(()),
        }
    }

    fn entry_path(&self, printer_name: &str) -> PathBuf {
        // FNV-1a keeps the file name short and stable across Rust releases.
        let hash = printer_name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        self.directory.join(format!("{hash:016x}.{EXTENSION}"))
    }

    fn read(&self, printer_name: &str) -> io::Result<Option<RdpdrPrinterCacheEntry>> {
        let path = self.entry_path(printer_name);
        let bytes = match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > MAX_ENTRY_BYTES => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "printer cache entry exceeds the size limit",
                ));
            }
            Ok(_) => fs::read(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let entry = decode_entry(&bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "printer cache entry is malformed"))?;
        Ok((entry.0 == printer_name).then_some(entry.1))
    }

    fn write(&self, printer_name: &str, entry: &RdpdrPrinterCacheEntry) -> io::Result<()> {
        static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

        let encoded = encode_entry(printer_name, entry)?;
        // Keep the previous entry when `read` would reject the new one.
        if u64::try_from(encoded.len()).map_or(true, |length| length > MAX_ENTRY_BYTES) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "printer cache entry exceeds the size limit",
            ));
        }
        fs::create_dir_all(&self.directory)?;
        let path = self.entry_path(printer_name);
        let temporary = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let result = fs::write(&temporary, encoded).and_then(|()| fs::rename(&temporary, &path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn remove(&self, printer_name: &str) -> io::Result<()> {
        // A file holding another printer's entry (hash collision) is kept; one that cannot be
        // read is removed so a corrupted entry does not outlive a delete event.
        if let Ok(None) = self.read(printer_name) {
            return Ok(());
        }
        match fs::remove_file(self.entry_path(printer_name)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RdpdrPrinterCacheStore for FileRdpdrPrinterCacheStore {
    fn load(&self, printer_name: &str) -> RdpdrPrinterCacheResult<Option<RdpdrPrinterCacheEntry>> {
        let _guard = self.guard();
        Ok(self.read(printer_name)?)
    }

    fn save(&self, printer_name: &str, entry: &RdpdrPrinterCacheEntry) -> RdpdrPrinterCacheResult<()> {
        let _guard = self.guard();
        Ok(self.write(printer_name, entry)?)
    }

    fn delete(&self, printer_name: &str) -> RdpdrPrinterCacheResult<()> {
        let _guard = self.guard();
        Ok(self.remove(printer_name)?)
    }
}

// Layout: MAGIC, then u32 LE length-prefixed UTF-8 printer name, the optional driver name and
// optional configuration driver name (each a presence byte and a length-prefixed UTF-8 string),
// and the length-prefixed configuration data.
fn encode_entry(printer_name: &str, entry: &RdpdrPrinterCacheEntry) -> io::Result<Vec<u8>> {
    fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
        let length = u32::try_from(bytes.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "printer cache field is too large"))?;
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(bytes);
        Ok(())
    }

    fn push_optional(out: &mut Vec<u8>, value: Option<&str>) -> io::Result<()> {
        match value {
            Some(value) => {
                out.push(1);
                push_bytes(out, value.as_bytes())
            }
            None => {
                out.push(0);
                Ok(())
            }
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    push_bytes(&mut out, printer_name.as_bytes())?;
    push_optional(&mut out, entry.driver_name.as_deref())?;
    push_optional(&mut out, entry.config_driver_name.as_deref())?;
    push_bytes(&mut out, &entry.config_data)?;
    Ok(out)
}

fn decode_entry(mut bytes: &[u8]) -> Option<(String, RdpdrPrinterCacheEntry)> {
    fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Option<&'a [u8]> {
        if bytes.len() < length {
            return None;
        }
        let (head, tail) = bytes.split_at(length);
        *bytes = tail;
        Some(head)
    }

    fn take_prefixed<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
        let length = u32::from_le_bytes(take(bytes, 4)?.try_into().ok()?);
        take(bytes, usize::try_from(length).ok()?)
    }

    fn take_optional(bytes: &mut &[u8]) -> Option<Option<String>> {
        match take(bytes, 1)? {
            [0] => Some(None),
            [1] => Some(Some(String::from_utf8(take_prefixed(bytes)?.to_vec()).ok()?)),
            _ => None,
        }
    }

    if take(&mut bytes, MAGIC.len())? != MAGIC {
        return None;
    }
    let printer_name = String::from_utf8(take_prefixed(&mut bytes)?.to_vec()).ok()?;
    let driver_name = take_optional(&mut bytes)?;
    let config_driver_name = take_optional(&mut bytes)?;
    let config_data = take_prefixed(&mut bytes)?.to_vec();
    bytes.is_empty().then_some((
        printer_name,
        RdpdrPrinterCacheEntry {
            driver_name,
            config_driver_name,
            config_data,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("ironrdp-printer-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn entries_survive_a_new_store_instance() {
        let directory = temporary_directory("persist");
        let entry = RdpdrPrinterCacheEntry {
            driver_name: Some("Server Driver".to_owned()),
            config_driver_name: Some("Server Driver".to_owned()),
            config_data: vec![1, 2, 3],
        };

        FileRdpdrPrinterCacheStore::new(&directory)
            .save(r"\\print\Office", &entry)
            .unwrap();

        let store = FileRdpdrPrinterCacheStore::new(&directory);
        assert_eq!(store.load(r"\\print\Office").unwrap(), Some(entry));
        assert_eq!(store.load("Other").unwrap(), None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn delete_removes_entries() {
        let directory = temporary_directory("delete");
        let store = FileRdpdrPrinterCacheStore::new(&directory);
        let entry = RdpdrPrinterCacheEntry {
            driver_name: None,
            config_driver_name: None,
            config_data: vec![9; 16],
        };
        store.save("Printer", &entry).unwrap();

        store.delete("Printer").unwrap();
        assert_eq!(store.load("Printer").unwrap(), None);
        store.delete("Missing").unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn oversized_saves_keep_the_previous_entry() {
        let directory = temporary_directory("oversized");
        let store = FileRdpdrPrinterCacheStore::new(&directory);
        let entry = RdpdrPrinterCacheEntry {
            driver_name: None,
            config_driver_name: None,
            config_data: vec![1],
        };
        store.save("Printer", &entry).unwrap();

        let oversized = RdpdrPrinterCacheEntry {
            config_data: vec![0; usize::try_from(MAX_ENTRY_BYTES).unwrap()],
            ..entry.clone()
        };
        assert!(store.save("Printer", &oversized).is_err());
        assert_eq!(store.load("Printer").unwrap(), Some(entry));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_entries_are_errors() {
        let directory = temporary_directory("malformed");
        let store = FileRdpdrPrinterCacheStore::new(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(store.entry_path("Printer"), b"not a cache entry").unwrap();

        assert!(store.load("Printer").is_err());
        store.delete("Printer").unwrap();
        assert_eq!(store.load("Printer").unwrap(), None);
        fs::remove_dir_all(directory).unwrap();
    }
}

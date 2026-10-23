use std::fs;
use std::path::PathBuf;

use ironrdp_client::egfx_cache::FilePersistentCache;
use ironrdp_egfx::cache::{CachedBitmap, PersistentCacheEntry, PersistentCacheStore as _};
use ironrdp_egfx::pdu::CacheEntryMetadata;
use proptest::prelude::*;
use uuid::Uuid;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ironrdp-egfx-cache-{}", Uuid::new_v4()));
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join("nested").join("egfx-cache.bin")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pixels(width: u16, height: u16, seed: u8) -> Vec<u8> {
    (0..usize::from(width) * usize::from(height) * 4)
        .map(|i| u8::try_from(i % 251).unwrap() ^ seed)
        .collect()
}

#[test]
fn a_missing_file_is_an_empty_cache() {
    // oracle: spec MS-RDPEGFX §3.3.1.5 (the persistent cache is optional, so none is an empty one)
    let dir = TempDir::new();
    let mut cache = FilePersistentCache::new(&dir.file());

    assert!(cache.entries().unwrap().is_empty());
    assert_eq!(cache.load(1).unwrap(), None);
}

#[test]
fn saved_entries_are_listed_in_order_and_load_back() {
    // oracle: property round-trip (save, then list and load)
    let dir = TempDir::new();
    let first = pixels(3, 2, 0x11);
    let second = pixels(64, 64, 0x22);

    FilePersistentCache::new(&dir.file())
        .save(&[
            CachedBitmap {
                cache_key: 0xDEAD_BEEF,
                width: 3,
                height: 2,
                data: &first,
            },
            CachedBitmap {
                cache_key: 7,
                width: 64,
                height: 64,
                data: &second,
            },
        ])
        .unwrap();

    let mut cache = FilePersistentCache::new(&dir.file());
    assert_eq!(
        cache.entries().unwrap(),
        [
            CacheEntryMetadata {
                cache_key: 0xDEAD_BEEF,
                bitmap_len: 3 * 2 * 4,
            },
            CacheEntryMetadata {
                cache_key: 7,
                bitmap_len: 64 * 64 * 4,
            },
        ]
    );
    assert_eq!(
        cache.load(7).unwrap(),
        Some(PersistentCacheEntry {
            cache_key: 7,
            width: 64,
            height: 64,
            data: second,
        })
    );
    assert_eq!(
        cache.load(0xDEAD_BEEF).unwrap(),
        Some(PersistentCacheEntry {
            cache_key: 0xDEAD_BEEF,
            width: 3,
            height: 2,
            data: first,
        })
    );
    assert_eq!(cache.load(8).unwrap(), None);
}

#[test]
fn saving_replaces_the_file_even_after_a_listing() {
    // oracle: mutant (a save that fails or appends while the listing holds the file)
    let dir = TempDir::new();
    let old = pixels(2, 2, 1);
    let new = pixels(4, 1, 2);
    let mut cache = FilePersistentCache::new(&dir.file());
    cache
        .save(&[CachedBitmap {
            cache_key: 1,
            width: 2,
            height: 2,
            data: &old,
        }])
        .unwrap();

    // A session lists and imports, then saves its own cache over the file it read.
    assert_eq!(cache.entries().unwrap().len(), 1);
    cache
        .save(&[CachedBitmap {
            cache_key: 2,
            width: 4,
            height: 1,
            data: &new,
        }])
        .unwrap();

    let mut reread = FilePersistentCache::new(&dir.file());
    let keys: Vec<u64> = reread.entries().unwrap().iter().map(|entry| entry.cache_key).collect();
    assert_eq!(keys, [2]);
}

#[test]
fn a_truncated_file_keeps_the_entries_before_the_cut() {
    // oracle: mutant (a cut file that loses every entry or reads past the end)
    let dir = TempDir::new();
    let data = pixels(8, 8, 3);
    FilePersistentCache::new(&dir.file())
        .save(&[
            CachedBitmap {
                cache_key: 1,
                width: 8,
                height: 8,
                data: &data,
            },
            CachedBitmap {
                cache_key: 2,
                width: 8,
                height: 8,
                data: &data,
            },
        ])
        .unwrap();
    let bytes = fs::read(dir.file()).unwrap();
    fs::write(dir.file(), &bytes[..bytes.len() - 10]).unwrap();

    let mut cache = FilePersistentCache::new(&dir.file());
    let keys: Vec<u64> = cache.entries().unwrap().iter().map(|entry| entry.cache_key).collect();

    assert_eq!(keys, [1]);
}

#[test]
fn a_failed_save_leaves_no_temporary_behind() {
    // oracle: mutant (a failed save that leaves its temporary file next to the cache)
    let dir = TempDir::new();
    let target = dir.file();
    // A non-empty directory where the cache file should be makes the final rename fail.
    fs::create_dir_all(target.join("occupied")).unwrap();
    let data = pixels(2, 2, 1);

    let result = FilePersistentCache::new(&target).save(&[CachedBitmap {
        cache_key: 1,
        width: 2,
        height: 2,
        data: &data,
    }]);

    assert!(result.is_err());
    let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| Some(name.as_os_str()) != target.file_name())
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn saving_nothing_valid_keeps_the_previous_cache() {
    // oracle: mutant (an empty save erasing the stored cache)
    let dir = TempDir::new();
    let data = pixels(2, 2, 7);
    let mut cache = FilePersistentCache::new(&dir.file());
    cache
        .save(&[CachedBitmap {
            cache_key: 1,
            width: 2,
            height: 2,
            data: &data,
        }])
        .unwrap();

    cache
        .save(&[CachedBitmap {
            cache_key: 2,
            width: 2,
            height: 2,
            data: &[],
        }])
        .unwrap();

    let keys: Vec<u64> = FilePersistentCache::new(&dir.file())
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.cache_key)
        .collect();
    assert_eq!(keys, [1]);
}

#[test]
fn a_repeated_key_is_written_once() {
    // oracle: spec MS-RDPEGFX §2.2.2.16.1 (cacheKey is a unique key)
    let dir = TempDir::new();
    let first = pixels(2, 2, 1);
    let second = pixels(3, 1, 2);
    FilePersistentCache::new(&dir.file())
        .save(&[
            CachedBitmap {
                cache_key: 5,
                width: 2,
                height: 2,
                data: &first,
            },
            CachedBitmap {
                cache_key: 5,
                width: 3,
                height: 1,
                data: &second,
            },
        ])
        .unwrap();

    let mut cache = FilePersistentCache::new(&dir.file());
    assert_eq!(cache.entries().unwrap().len(), 1);
    assert_eq!(cache.load(5).unwrap().unwrap().data, first);
}

#[test]
fn a_foreign_file_is_an_error_not_a_cache() {
    // oracle: mutant (a file without the signature read as a cache)
    let dir = TempDir::new();
    fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    fs::write(dir.file(), b"not a cache file").unwrap();

    assert!(FilePersistentCache::new(&dir.file()).entries().is_err());
}

#[test]
fn another_version_of_the_format_is_an_error_not_a_cache() {
    // oracle: reference mstscax.dll 10.0.26100.9711 CacheFileReader::Open (the 12-byte header,
    // version included, must match)
    let dir = TempDir::new();
    fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    fs::write(dir.file(), b"RDP8bmp\0\x03\0\0\0").unwrap();

    assert!(FilePersistentCache::new(&dir.file()).entries().is_err());
}

/// The bytes of one cache file entry as mstsc lays it out.
fn mstsc_entry(cache_key: u64, width: u16, height: u16, bgra: &[u8]) -> Vec<u8> {
    let mut bytes = cache_key.to_le_bytes().to_vec();
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(bgra);
    bytes
}

const MSTSC_HEADER: &[u8] = b"RDP8bmp\0\x06\0\0\0";

#[test]
fn the_file_is_written_in_the_mstsc_format() {
    // oracle: reference mstscax.dll 10.0.26100.9711 RdpCacheDatabase::CacheExport and
    // awakecoding/FreeRDP@16a7c2ec2fd0:libfreerdp/cache/persistent.c persistent_cache_write_entry_v3
    // (signature, version 6, then key, width, height and BGRA pixels per entry, no count)
    let dir = TempDir::new();
    FilePersistentCache::new(&dir.file())
        .save(&[CachedBitmap {
            cache_key: 0x0102_0304_0506_0708,
            width: 2,
            height: 1,
            data: &[0x10, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0x80],
        }])
        .unwrap();

    let mut expected = MSTSC_HEADER.to_vec();
    expected.extend(mstsc_entry(
        0x0102_0304_0506_0708,
        2,
        1,
        &[0x30, 0x20, 0x10, 0xFF, 0x60, 0x50, 0x40, 0x80],
    ));
    assert_eq!(fs::read(dir.file()).unwrap(), expected);
}

#[test]
fn a_file_in_the_mstsc_format_loads_as_rgba() {
    // oracle: reference mstscax.dll 10.0.26100.9711 RdpCacheDatabase::CacheImportOffer and
    // CacheImportReply (entries follow the header until the end of the file, pixels in BGRA)
    let dir = TempDir::new();
    let mut file = MSTSC_HEADER.to_vec();
    file.extend(mstsc_entry(7, 1, 1, &[0x01, 0x02, 0x03, 0xFF]));
    file.extend(mstsc_entry(9, 1, 2, &[0x0A, 0x0B, 0x0C, 0xFF, 0x0D, 0x0E, 0x0F, 0xFF]));
    fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    fs::write(dir.file(), file).unwrap();

    let mut cache = FilePersistentCache::new(&dir.file());
    let keys: Vec<u64> = cache.entries().unwrap().iter().map(|entry| entry.cache_key).collect();
    assert_eq!(keys, [7, 9]);
    assert_eq!(
        cache.load(9).unwrap(),
        Some(PersistentCacheEntry {
            cache_key: 9,
            width: 1,
            height: 2,
            data: vec![0x0C, 0x0B, 0x0A, 0xFF, 0x0F, 0x0E, 0x0D, 0xFF],
        })
    );
}

#[test]
fn entries_mstsc_would_refuse_are_not_written() {
    // oracle: reference mstscax.dll 10.0.26100.9711 RdpCacheDatabase::CacheImportOffer (a key of 0
    // or pixels past 65,536 bytes make it offer nothing from the file)
    let dir = TempDir::new();
    let small = pixels(1, 1, 1);
    let large = pixels(129, 128, 2);
    FilePersistentCache::new(&dir.file())
        .save(&[
            CachedBitmap {
                cache_key: 0,
                width: 1,
                height: 1,
                data: &small,
            },
            CachedBitmap {
                cache_key: 1,
                width: 129,
                height: 128,
                data: &large,
            },
            CachedBitmap {
                cache_key: 2,
                width: 1,
                height: 1,
                data: &small,
            },
        ])
        .unwrap();

    let keys: Vec<u64> = FilePersistentCache::new(&dir.file())
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.cache_key)
        .collect();
    assert_eq!(keys, [2]);
}

#[test]
fn a_key_0_entry_in_the_file_is_skipped() {
    // oracle: mutant (an entry mstsc treats as invalid offered to the server)
    let dir = TempDir::new();
    let mut file = MSTSC_HEADER.to_vec();
    file.extend(mstsc_entry(0, 1, 1, &[1, 2, 3, 4]));
    file.extend(mstsc_entry(5, 1, 1, &[5, 6, 7, 8]));
    fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    fs::write(dir.file(), file).unwrap();

    let keys: Vec<u64> = FilePersistentCache::new(&dir.file())
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.cache_key)
        .collect();
    assert_eq!(keys, [5]);
}

#[test]
fn malformed_entries_are_not_written() {
    // oracle: mutant (an entry whose data does not cover its size written to disk)
    let dir = TempDir::new();
    let short = vec![0; 5];
    let good = pixels(1, 1, 4);
    FilePersistentCache::new(&dir.file())
        .save(&[
            CachedBitmap {
                cache_key: 1,
                width: 2,
                height: 2,
                data: &short,
            },
            CachedBitmap {
                cache_key: 2,
                width: 1,
                height: 1,
                data: &good,
            },
        ])
        .unwrap();

    let keys: Vec<u64> = FilePersistentCache::new(&dir.file())
        .entries()
        .unwrap()
        .iter()
        .map(|entry| entry.cache_key)
        .collect();
    assert_eq!(keys, [2]);
}

/// Entries with distinct non-zero keys and pixels of exactly `width * height * 4` bytes.
fn entries() -> impl Strategy<Value = Vec<(u64, u16, u16, Vec<u8>)>> {
    proptest::collection::btree_map(1..=u64::MAX, (1u16..=24, 1u16..=24, any::<u8>()), 0..8).prop_map(|map| {
        map.into_iter()
            .map(|(cache_key, (width, height, seed))| (cache_key, width, height, pixels(width, height, seed)))
            .collect()
    })
}

fn bitmaps(entries: &[(u64, u16, u16, Vec<u8>)]) -> Vec<CachedBitmap<'_>> {
    entries
        .iter()
        .map(|(cache_key, width, height, data)| CachedBitmap {
            cache_key: *cache_key,
            width: *width,
            height: *height,
            data,
        })
        .collect()
}

#[test]
fn any_saved_cache_lists_and_loads_back_unchanged() {
    // oracle: property round-trip (save, then list in order and load every entry)
    proptest!(ProptestConfig::with_cases(64), |(entries in entries())| {
        let dir = TempDir::new();
        FilePersistentCache::new(&dir.file()).save(&bitmaps(&entries)).unwrap();

        let mut cache = FilePersistentCache::new(&dir.file());
        let listed = cache.entries().unwrap();
        let keys: Vec<u64> = listed.iter().map(|entry| entry.cache_key).collect();
        let expected_keys: Vec<u64> = entries.iter().map(|entry| entry.0).collect();
        prop_assert_eq!(keys, expected_keys);
        for (cache_key, width, height, data) in entries {
            prop_assert_eq!(
                cache.load(cache_key).unwrap(),
                Some(PersistentCacheEntry { cache_key, width, height, data })
            );
        }
    });
}

#[test]
fn any_truncated_cache_lists_a_prefix_of_what_was_saved() {
    // oracle: mutant (a cut file that panics, reads past the end or invents entries)
    proptest!(ProptestConfig::with_cases(64), |(entries in entries(), cut in any::<prop::sample::Index>())| {
        let dir = TempDir::new();
        FilePersistentCache::new(&dir.file()).save(&bitmaps(&entries)).unwrap();
        // Saving nothing leaves no file to cut.
        let Ok(bytes) = fs::read(dir.file()) else {
            prop_assert!(entries.is_empty());
            return Ok(());
        };
        fs::write(dir.file(), &bytes[..cut.index(bytes.len() + 1)]).unwrap();

        let mut cache = FilePersistentCache::new(&dir.file());
        if let Ok(listed) = cache.entries() {
            let keys: Vec<u64> = listed.iter().map(|entry| entry.cache_key).collect();
            let saved: Vec<u64> = entries.iter().map(|entry| entry.0).collect();
            prop_assert!(saved.starts_with(&keys));
            for key in keys {
                prop_assert!(cache.load(key).unwrap().is_some());
            }
        }
    });
}

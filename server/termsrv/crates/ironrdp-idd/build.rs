use std::path::{Path, PathBuf};

type BuildResult<T> = Result<T, String>;

fn main() -> BuildResult<()> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    // Tell `rustc` about our custom cfg flag so `-D unexpected_cfgs` doesn't fail CI.
    println!("cargo:rustc-check-cfg=cfg(ironrdp_idd_link)");

    // Opt-in linking for the IDD/IddCx libraries and driver-style linker flags.
    // This keeps `cargo check` working on typical developer machines that do not
    // have the full WDK/IddCx import libraries configured.
    println!("cargo:rerun-if-env-changed=IRONRDP_IDD_LINK");
    println!("cargo:rerun-if-env-changed=IRONRDP_IDDCX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=IRONRDP_IDDCX_STUB_VERSION");
    println!("cargo:rerun-if-env-changed=IRONRDP_WINDOWS_KITS_ROOT");
    println!("cargo:rerun-if-env-changed=IRONRDP_WDF_UMDF_LIB_DIR");
    println!("cargo:rerun-if-env-changed=IRONRDP_WDF_UMDF_STUB_VERSION");
    println!("cargo:rerun-if-env-changed=IRONRDP_WDF_STUB_BUILD_NUMBER");

    if std::env::var_os("IRONRDP_IDD_LINK").is_none() {
        return Ok(());
    }

    // Compile-time gate for the parts of the crate that reference IddCx globals.
    println!("cargo:rustc-cfg=ironrdp_idd_link");

    if let Some(iddcx_lib_dir) = resolve_iddcx_stub_lib_dir() {
        println!("cargo:rustc-link-search=native={}", iddcx_lib_dir.display());
        println!("cargo:warning=Using iddcxstub.lib from {}", iddcx_lib_dir.display());
    } else {
        println!(
            "cargo:warning=iddcxstub.lib not found (WDK missing). Run server/termsrv/crates/ironrdp-idd/scripts/find-wdk-tools.ps1 or set IRONRDP_IDDCX_LIB_DIR"
        );
    }

    println!("cargo:rustc-link-lib=dylib=d3d11");
    println!("cargo:rustc-link-lib=dylib=dxgi");
    println!("cargo:rustc-link-lib=dylib=iddcxstub");

    // WDF UMDF v2 stub library — required for WdfDriverCreate dispatch table.
    //
    // Patch only the official WDK stub's bind-info build number in a private
    // OUT_DIR copy to avoid pre-release rejection without changing its function table.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by Cargo"));
    let kits_root = std::env::var_os("IRONRDP_WINDOWS_KITS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"));

    let preferred_wdf_stub_version =
        std::env::var("IRONRDP_WDF_UMDF_STUB_VERSION").unwrap_or_else(|_| "2.33".to_owned());

    let wdf_dir = resolve_wdf_umdf_stub_lib_dir(&kits_root, &preferred_wdf_stub_version)
        .ok_or_else(|| "WdfDriverStubUm.lib not found; install the WDK or set IRONRDP_WDF_UMDF_LIB_DIR".to_owned())?;
    let source_stub = wdf_dir.join("WdfDriverStubUm.lib");
    let actual_version = wdf_dir.file_name().and_then(|name| name.to_str());
    link_wdf_stub(&source_stub, &out_dir, "WDK", actual_version)?;
    println!("cargo:rustc-link-lib=WdfDriverStubUm");

    // NOTE: This crate targets UMDF (a user-mode driver). Avoid kernel-mode driver
    // linker flags here; they break linking against the normal MSVC CRT.
    Ok(())
}

fn resolve_iddcx_stub_lib_dir() -> Option<PathBuf> {
    if let Some(override_dir) = std::env::var_os("IRONRDP_IDDCX_LIB_DIR") {
        let dir = PathBuf::from(override_dir);
        if dir.join("iddcxstub.lib").is_file() {
            return Some(dir);
        }

        println!(
            "cargo:warning=IRONRDP_IDDCX_LIB_DIR is set but iddcxstub.lib was not found in {}",
            dir.display()
        );
    }

    let kits_root = std::env::var_os("IRONRDP_WINDOWS_KITS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"));

    // Default to the 1.4 IddCx stub so the driver can use remote-IDD DDIs when the target OS
    // exposes them at runtime. This is independent from the INF's class-extension service name.
    // Can be overridden for diagnostics/experimentation.
    let preferred_stub_version = std::env::var("IRONRDP_IDDCX_STUB_VERSION").unwrap_or_else(|_| "1.4".to_owned());

    find_windows_kits_iddcx_stub_lib_dir(&kits_root, &preferred_stub_version)
}

fn find_windows_kits_iddcx_stub_lib_dir(kits_root: &Path, preferred_stub_version: &str) -> Option<PathBuf> {
    let lib_root = kits_root.join("Lib");
    if !lib_root.is_dir() {
        println!(
            "cargo:warning=Windows Kits Lib folder not found: {}",
            lib_root.display()
        );
        return None;
    }

    let arch_folder = match std::env::var("CARGO_CFG_TARGET_ARCH").ok().as_deref() {
        Some("x86_64") => "x64",
        Some("x86") => "x86",
        Some("aarch64") => "arm64",
        other => {
            println!("cargo:warning=unsupported target arch for iddcx.lib discovery: {other:?}");
            return None;
        }
    };

    let mut candidates = Vec::new();

    let Ok(entries) = std::fs::read_dir(&lib_root) else {
        return None;
    };

    for entry in entries.flatten() {
        let kits_version_dir = entry.path();
        if !kits_version_dir.is_dir() {
            continue;
        }

        for kind in ["um", "km"] {
            let arch_dir = kits_version_dir.join(kind).join(arch_folder);
            let iddcx_root = arch_dir.join("iddcx");
            if !iddcx_root.is_dir() {
                continue;
            }

            let Ok(iddcx_versions) = std::fs::read_dir(&iddcx_root) else {
                continue;
            };

            for version_entry in iddcx_versions.flatten() {
                let version_dir = version_entry.path();
                if !version_dir.is_dir() {
                    continue;
                }

                if version_dir.join("iddcxstub.lib").is_file() {
                    candidates.push(version_dir);
                }
            }
        }
    }

    if let Some(preferred) = candidates.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(preferred_stub_version))
    }) {
        return Some(preferred.clone());
    }

    println!(
        "cargo:warning=preferred iddcxstub version '{}' not found, using highest available",
        preferred_stub_version
    );
    candidates.sort();
    candidates.pop()
}

/// Looks for `WdfDriverStubUm.lib` under `<kits_root>\Lib\wdf\umdf\<arch>\<version>\`.
///
/// Returns the directory containing the highest-version `WdfDriverStubUm.lib` found.
fn resolve_wdf_umdf_stub_lib_dir(kits_root: &Path, preferred_stub_version: &str) -> Option<PathBuf> {
    if let Some(override_dir) = std::env::var_os("IRONRDP_WDF_UMDF_LIB_DIR") {
        let dir = PathBuf::from(override_dir);
        if dir.join("WdfDriverStubUm.lib").is_file() {
            return Some(dir);
        }
        println!(
            "cargo:warning=IRONRDP_WDF_UMDF_LIB_DIR is set but WdfDriverStubUm.lib was not found in {}",
            dir.display()
        );
    }

    let arch_folder = match std::env::var("CARGO_CFG_TARGET_ARCH").ok().as_deref() {
        Some("x86_64") => "x64",
        Some("x86") => "x86",
        Some("aarch64") => "arm64",
        other => {
            println!("cargo:warning=unsupported target arch for WdfDriverStubUm.lib discovery: {other:?}");
            return None;
        }
    };

    // Layout: <kits_root>\Lib\wdf\umdf\<arch>\<version>\WdfDriverStubUm.lib
    let umdf_arch_dir = kits_root.join("Lib").join("wdf").join("umdf").join(arch_folder);

    let Ok(entries) = std::fs::read_dir(&umdf_arch_dir) else {
        println!(
            "cargo:warning=WDF UMDF lib folder not found: {}",
            umdf_arch_dir.display()
        );
        return None;
    };

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let version_dir = entry.path();
        if version_dir.is_dir() && version_dir.join("WdfDriverStubUm.lib").is_file() {
            candidates.push(version_dir);
        }
    }

    if let Some(preferred) = candidates.iter().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(preferred_stub_version))
    }) {
        return Some(preferred.clone());
    }

    println!(
        "cargo:warning=preferred WDF UMDF stub version '{}' not found, using highest available",
        preferred_stub_version
    );

    candidates.sort();
    candidates.pop()
}

struct WdfBindInfoRecord {
    major: u32,
    minor: u32,
    build: u32,
    build_offset: usize,
}

fn find_wdf_bind_info_record(archive: &[u8]) -> BuildResult<WdfBindInfoRecord> {
    if !archive.starts_with(b"!<arch>\n") {
        return Err("invalid WDF library archive header".to_owned());
    }

    let mut records = Vec::new();
    let mut offset = 8;
    while offset < archive.len() {
        let header_end = offset
            .checked_add(60)
            .ok_or_else(|| "WDF archive member offset overflowed".to_owned())?;
        let header = archive
            .get(offset..header_end)
            .ok_or_else(|| "truncated WDF archive member header".to_owned())?;
        if &header[58..60] != b"`\n" {
            return Err("invalid WDF archive member trailer".to_owned());
        }

        let member_size = core::str::from_utf8(&header[48..58])
            .map_err(|_| "invalid WDF archive member size".to_owned())?
            .trim()
            .parse::<usize>()
            .map_err(|_| "invalid WDF archive member size".to_owned())?;
        let member_end = header_end
            .checked_add(member_size)
            .ok_or_else(|| "WDF archive member size overflowed".to_owned())?;
        let member = archive
            .get(header_end..member_end)
            .ok_or_else(|| "truncated WDF archive member".to_owned())?;
        let name_end = header[..16].iter().position(|byte| *byte == b' ').unwrap_or(16);
        let member_name = &header[..name_end];
        if !matches!(member_name, b"/" | b"//" | b"/SYM64/") {
            records.extend(parse_wdf_bind_info_in_coff_object(member, header_end)?);
        }

        offset = member_end
            .checked_add(member_size % 2)
            .ok_or_else(|| "WDF archive member offset overflowed".to_owned())?;
        if offset > archive.len() {
            return Err("truncated WDF archive member padding".to_owned());
        }
    }

    match records.as_slice() {
        [record] => Ok(WdfBindInfoRecord {
            major: record.major,
            minor: record.minor,
            build: record.build,
            build_offset: record.build_offset,
        }),
        [] => Err("WDF library has no WdfBindInfo symbol".to_owned()),
        _ => Err("WDF library has multiple WdfBindInfo symbols".to_owned()),
    }
}

fn parse_wdf_bind_info_in_coff_object(
    object: &[u8],
    archive_member_offset: usize,
) -> BuildResult<Vec<WdfBindInfoRecord>> {
    let Some(machine) = read_u16(object, 0) else {
        return Ok(Vec::new());
    };
    let version_offset = match machine {
        0x014c => 8,
        0x8664 | 0xa641 | 0xaa64 => 16,
        _ => return Ok(Vec::new()),
    };

    let section_count = usize::from(read_u16(object, 2).ok_or_else(|| "truncated COFF object header".to_owned())?);
    let symbol_table_offset =
        usize::try_from(read_u32(object, 8).ok_or_else(|| "truncated COFF object header".to_owned())?)
            .map_err(|_| "COFF symbol table offset is not representable".to_owned())?;
    let symbol_count = usize::try_from(read_u32(object, 12).ok_or_else(|| "truncated COFF object header".to_owned())?)
        .map_err(|_| "COFF symbol count is not representable".to_owned())?;
    let optional_header_size =
        usize::from(read_u16(object, 16).ok_or_else(|| "truncated COFF object header".to_owned())?);
    let section_table_offset = 20usize
        .checked_add(optional_header_size)
        .ok_or_else(|| "COFF section table offset overflowed".to_owned())?;
    let section_table_size = section_count
        .checked_mul(40)
        .ok_or_else(|| "COFF section table size overflowed".to_owned())?;
    let section_table_end = section_table_offset
        .checked_add(section_table_size)
        .ok_or_else(|| "COFF section table size overflowed".to_owned())?;
    if object.get(section_table_offset..section_table_end).is_none() {
        return Err("truncated COFF section table".to_owned());
    }
    if symbol_table_offset == 0 || symbol_count == 0 {
        return Ok(Vec::new());
    }

    let symbol_table_size = symbol_count
        .checked_mul(18)
        .ok_or_else(|| "COFF symbol table size overflowed".to_owned())?;
    let string_table_offset = symbol_table_offset
        .checked_add(symbol_table_size)
        .ok_or_else(|| "COFF string table offset overflowed".to_owned())?;
    if object.get(symbol_table_offset..string_table_offset).is_none() {
        return Err("truncated COFF symbol table".to_owned());
    }
    let string_table_size =
        usize::try_from(read_u32(object, string_table_offset).ok_or_else(|| "missing COFF string table".to_owned())?)
            .map_err(|_| "COFF string table size is not representable".to_owned())?;
    if string_table_size < 4 {
        return Err("invalid COFF string table size".to_owned());
    }
    let string_table_end = string_table_offset
        .checked_add(string_table_size)
        .ok_or_else(|| "COFF string table size overflowed".to_owned())?;
    let string_table = object
        .get(string_table_offset..string_table_end)
        .ok_or_else(|| "truncated COFF string table".to_owned())?;

    let mut records = Vec::new();
    let mut symbol_index = 0usize;
    while symbol_index < symbol_count {
        let symbol_offset = symbol_table_offset
            .checked_add(
                symbol_index
                    .checked_mul(18)
                    .ok_or_else(|| "COFF symbol offset overflowed".to_owned())?,
            )
            .ok_or_else(|| "COFF symbol offset overflowed".to_owned())?;
        let symbol_end = symbol_offset
            .checked_add(18)
            .ok_or_else(|| "COFF symbol offset overflowed".to_owned())?;
        let symbol = object
            .get(symbol_offset..symbol_end)
            .ok_or_else(|| "truncated COFF symbol table".to_owned())?;
        let name = coff_symbol_name(&symbol[..8], string_table)?;
        let value = read_u32(symbol, 8).ok_or_else(|| "truncated COFF symbol".to_owned())?;
        let section = read_i16(symbol, 12).ok_or_else(|| "truncated COFF symbol".to_owned())?;
        let storage_class = symbol[16];
        let auxiliary_count = usize::from(symbol[17]);

        if name == b"WdfBindInfo" || name == b"_WdfBindInfo" {
            if !matches!(storage_class, 2 | 3) || section <= 0 {
                return Err("WdfBindInfo is not a defined COFF data symbol".to_owned());
            }
            let section_index = usize::try_from(section - 1).expect("positive section index");
            if section_count <= section_index {
                return Err("WdfBindInfo references a missing COFF section".to_owned());
            }
            let section_header_offset = section_table_offset + section_index * 40;
            let section_size = usize::try_from(
                read_u32(object, section_header_offset + 16)
                    .ok_or_else(|| "truncated COFF section header".to_owned())?,
            )
            .map_err(|_| "COFF section size is not representable".to_owned())?;
            let section_data_offset = usize::try_from(
                read_u32(object, section_header_offset + 20)
                    .ok_or_else(|| "truncated COFF section header".to_owned())?,
            )
            .map_err(|_| "COFF section offset is not representable".to_owned())?;
            let section_data_end = section_data_offset
                .checked_add(section_size)
                .ok_or_else(|| "COFF section data size overflowed".to_owned())?;
            let section_data = object
                .get(section_data_offset..section_data_end)
                .ok_or_else(|| "truncated COFF section data".to_owned())?;
            let value = usize::try_from(value).map_err(|_| "WdfBindInfo offset is not representable".to_owned())?;
            let record_size = usize::try_from(
                read_u32(section_data, value).ok_or_else(|| "truncated WdfBindInfo record".to_owned())?,
            )
            .map_err(|_| "WdfBindInfo size is not representable".to_owned())?;
            let record_end = value
                .checked_add(record_size)
                .ok_or_else(|| "WdfBindInfo size overflowed".to_owned())?;
            if record_size < version_offset + 16 || section_data.get(value..record_end).is_none() {
                return Err("invalid WdfBindInfo record size".to_owned());
            }

            let major = read_u32(section_data, value + version_offset)
                .ok_or_else(|| "truncated WdfBindInfo version".to_owned())?;
            let minor = read_u32(section_data, value + version_offset + 4)
                .ok_or_else(|| "truncated WdfBindInfo version".to_owned())?;
            let build = read_u32(section_data, value + version_offset + 8)
                .ok_or_else(|| "truncated WdfBindInfo version".to_owned())?;
            let function_count = read_u32(section_data, value + version_offset + 12)
                .ok_or_else(|| "truncated WdfBindInfo version".to_owned())?;
            if major != 2 || !(128..=1024).contains(&function_count) {
                return Err("unsupported WdfBindInfo version or function count".to_owned());
            }

            let build_offset = archive_member_offset
                .checked_add(section_data_offset)
                .and_then(|offset| offset.checked_add(value))
                .and_then(|offset| offset.checked_add(version_offset + 8))
                .ok_or_else(|| "WdfBindInfo build offset overflowed".to_owned())?;
            records.push(WdfBindInfoRecord {
                major,
                minor,
                build,
                build_offset,
            });
        }

        symbol_index = symbol_index
            .checked_add(1 + auxiliary_count)
            .ok_or_else(|| "COFF symbol index overflowed".to_owned())?;
        if symbol_count < symbol_index {
            return Err("invalid COFF auxiliary symbol count".to_owned());
        }
    }

    Ok(records)
}

fn coff_symbol_name<'a>(name: &'a [u8], string_table: &'a [u8]) -> BuildResult<&'a [u8]> {
    if name[..4] == [0; 4] {
        let offset = usize::try_from(read_u32(name, 4).ok_or_else(|| "truncated COFF symbol name".to_owned())?)
            .map_err(|_| "COFF symbol name offset is not representable".to_owned())?;
        if offset < 4 {
            return Err("invalid COFF symbol name offset".to_owned());
        }
        let name = string_table
            .get(offset..)
            .ok_or_else(|| "COFF symbol name is outside its string table".to_owned())?;
        let end = name
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "unterminated COFF symbol name".to_owned())?;
        Ok(&name[..end])
    } else {
        let end = name.iter().position(|byte| *byte == 0).unwrap_or(name.len());
        Ok(&name[..end])
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    Some(u16::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_i16(bytes: &[u8], offset: usize) -> Option<i16> {
    let end = offset.checked_add(2)?;
    Some(i16::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn prepare_patched_wdf_stub_lib(
    source_stub: &Path,
    out_dir: &Path,
    expected_version: Option<&str>,
) -> BuildResult<Option<PathBuf>> {
    let mut patched = std::fs::read(source_stub).map_err(|error| error.to_string())?;
    let build_number = std::env::var("IRONRDP_WDF_STUB_BUILD_NUMBER")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(26100);

    let expected_version = expected_version
        .map(|version| {
            let (major, minor) = version
                .split_once('.')
                .ok_or_else(|| format!("invalid WDF stub version {version}"))?;
            Ok::<(u32, u32), String>((
                major
                    .parse::<u32>()
                    .map_err(|_| format!("invalid WDF stub version {version}"))?,
                minor
                    .parse::<u32>()
                    .map_err(|_| format!("invalid WDF stub version {version}"))?,
            ))
        })
        .transpose()?;
    let record = find_wdf_bind_info_record(&patched)?;
    if expected_version.is_some_and(|version| version != (record.major, record.minor)) {
        return Err(format!(
            "WDF stub version {}.{} does not match the selected version",
            record.major, record.minor
        ));
    }
    if record.build != 0 {
        println!(
            "cargo:warning={} already has WDF build {}; using the source library",
            source_stub.display(),
            record.build
        );
        return Ok(None);
    }

    let build_end = record
        .build_offset
        .checked_add(4)
        .ok_or_else(|| "WdfBindInfo build offset overflowed".to_owned())?;
    patched[record.build_offset..build_end].copy_from_slice(&build_number.to_le_bytes());
    println!(
        "cargo:warning=patched WDF {}.{} bind-info build number to {build_number}",
        record.major, record.minor
    );

    let patched_dir = out_dir.join("wdf-patched");
    std::fs::create_dir_all(&patched_dir).map_err(|error| error.to_string())?;
    let patched_path = patched_dir.join("WdfDriverStubUm.lib");
    std::fs::write(&patched_path, patched).map_err(|error| error.to_string())?;
    Ok(Some(patched_dir))
}

fn link_wdf_stub(
    source_stub: &Path,
    out_dir: &Path,
    source_label: &str,
    expected_version: Option<&str>,
) -> BuildResult<()> {
    let patched_dir = prepare_patched_wdf_stub_lib(source_stub, out_dir, expected_version)
        .map_err(|error| format!("failed to prepare {source_label} WdfDriverStubUm.lib: {error}"))?;
    if let Some(patched_dir) = patched_dir {
        println!("cargo:rustc-link-search=native={}", patched_dir.display());
        println!(
            "cargo:warning=Using patched {source_label} WdfDriverStubUm.lib from {}",
            source_stub.display()
        );
    } else {
        let link_dir = source_stub
            .parent()
            .ok_or_else(|| "WdfDriverStubUm.lib path has no parent directory".to_owned())?;
        println!("cargo:rustc-link-search=native={}", link_dir.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("ironrdp-wdf-patcher-{}-{unique}", std::process::id()));
        std::fs::create_dir(&path).expect("create test directory");
        path
    }

    fn archive_with_record(symbol_name: &[u8], machine: u16) -> Vec<u8> {
        let section_data_size = 88;
        let symbol_table_offset = 20 + 40 + section_data_size;
        let version_offset = if machine == 0x014c { 8 } else { 16 };
        let string_table_size = if symbol_name.len() > 8 {
            4 + symbol_name.len() + 1
        } else {
            4
        };
        let mut object = vec![0; symbol_table_offset + 18 + string_table_size];
        object[0..2].copy_from_slice(&machine.to_le_bytes());
        object[2..4].copy_from_slice(&1u16.to_le_bytes());
        object[8..12].copy_from_slice(&(symbol_table_offset as u32).to_le_bytes());
        object[12..16].copy_from_slice(&1u32.to_le_bytes());

        let section_header = 20;
        object[section_header..section_header + 5].copy_from_slice(b".data");
        object[section_header + 16..section_header + 20].copy_from_slice(&(section_data_size as u32).to_le_bytes());
        object[section_header + 20..section_header + 24].copy_from_slice(&60u32.to_le_bytes());

        let record = 60;
        object[record..record + 4].copy_from_slice(&(section_data_size as u32).to_le_bytes());
        object[record + version_offset..record + version_offset + 4].copy_from_slice(&2u32.to_le_bytes());
        object[record + version_offset + 4..record + version_offset + 8].copy_from_slice(&33u32.to_le_bytes());
        object[record + version_offset + 8..record + version_offset + 12].copy_from_slice(&0u32.to_le_bytes());
        object[record + version_offset + 12..record + version_offset + 16].copy_from_slice(&288u32.to_le_bytes());

        let symbol = symbol_table_offset;
        if symbol_name.len() > 8 {
            object[symbol + 4..symbol + 8].copy_from_slice(&4u32.to_le_bytes());
            object[symbol_table_offset + 18..symbol_table_offset + 22]
                .copy_from_slice(&(string_table_size as u32).to_le_bytes());
            object[symbol_table_offset + 22..symbol_table_offset + 22 + symbol_name.len()].copy_from_slice(symbol_name);
        } else {
            object[symbol..symbol + symbol_name.len()].copy_from_slice(symbol_name);
            object[symbol_table_offset + 18..symbol_table_offset + 22].copy_from_slice(&4u32.to_le_bytes());
        }
        object[symbol + 12..symbol + 14].copy_from_slice(&1i16.to_le_bytes());
        object[symbol + 16] = 2;

        let mut archive = b"!<arch>\n".to_vec();
        let mut header = vec![b' '; 60];
        header[..9].copy_from_slice(b"test.obj/");
        header[48..58].copy_from_slice(format!("{:<10}", object.len()).as_bytes());
        header[58..60].copy_from_slice(b"`\n");
        archive.extend_from_slice(&header);
        archive.extend_from_slice(&object);
        archive
    }

    // oracle: mutant patch a version-shaped archive record without the WdfBindInfo symbol.
    #[test]
    fn patcher_does_not_patch_a_version_shaped_decoy() {
        let temp_dir = test_directory();
        let source_stub = temp_dir.join("WdfDriverStubUm.lib");
        std::fs::write(&source_stub, archive_with_record(b"Decoy", 0x8664)).expect("write fixture archive");

        let result = prepare_patched_wdf_stub_lib(&source_stub, &temp_dir, Some("2.33"));
        std::fs::remove_dir_all(&temp_dir).expect("remove test directory");

        assert!(result.is_err(), "the decoy record must not be patched");
    }

    // oracle: mutant patch a byte sequence other than the named WdfBindInfo build field.
    #[test]
    fn patcher_changes_the_named_bind_info_build_field_only() {
        let temp_dir = test_directory();
        let source_stub = temp_dir.join("WdfDriverStubUm.lib");
        let original = archive_with_record(b"WdfBindInfo", 0x8664);
        std::fs::write(&source_stub, &original).expect("write fixture archive");

        let patched_dir = prepare_patched_wdf_stub_lib(&source_stub, &temp_dir, Some("2.33"))
            .expect("prepare WDF stub")
            .expect("zero-build WDF stub is patched");
        let patched = std::fs::read(patched_dir.join("WdfDriverStubUm.lib")).expect("read patched archive");
        let build_number = std::env::var("IRONRDP_WDF_STUB_BUILD_NUMBER")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(26100);
        let mut expected = original;
        expected[152..156].copy_from_slice(&build_number.to_le_bytes());
        std::fs::remove_dir_all(&temp_dir).expect("remove test directory");

        assert_eq!(patched, expected);
    }

    // oracle: mutant patch an x86 WdfBindInfo build field using the 64-bit structure layout.
    #[test]
    fn patcher_uses_the_x86_bind_info_layout() {
        let record = find_wdf_bind_info_record(&archive_with_record(b"WdfBindInfo", 0x014c))
            .expect("locate x86 WdfBindInfo symbol");

        assert_eq!(record.build_offset, 144);
    }
}

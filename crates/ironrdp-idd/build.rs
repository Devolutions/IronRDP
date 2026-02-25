#[cfg(windows)]
use windows as _;

use std::path::{Path, PathBuf};

fn main() {
    if !cfg!(target_os = "windows") {
        return;
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
        return;
    }

    // Compile-time gate for the parts of the crate that reference IddCx globals.
    println!("cargo:rustc-cfg=ironrdp_idd_link");

    if let Some(iddcx_lib_dir) = resolve_iddcx_stub_lib_dir() {
        println!("cargo:rustc-link-search=native={}", iddcx_lib_dir.display());
    } else {
        println!(
            "cargo:warning=iddcxstub.lib not found (WDK missing). Run crates/ironrdp-idd/scripts/find-wdk-tools.ps1 or set IRONRDP_IDDCX_LIB_DIR"
        );
    }

    println!("cargo:rustc-link-lib=dylib=d3d11");
    println!("cargo:rustc-link-lib=dylib=dxgi");
    println!("cargo:rustc-link-lib=dylib=iddcxstub");

    // WDF UMDF v2 stub library — required for WdfDriverCreate dispatch table.
    //
    // We prefer the official WDK stub and patch only its bind-info build number
    // in a private OUT_DIR copy. This avoids pre-release rejection while keeping
    // the original WDK function-table count intact.
    println!("cargo:rerun-if-env-changed=IRONRDP_WDF_USE_LOCAL_STUB");
    println!("cargo:rerun-if-changed=WdfDriverStubUm.lib");
    let manifest_dir = std::path::PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by Cargo"),
    );
    let local_wdf_lib = manifest_dir.join("WdfDriverStubUm.lib");
    let use_local = match std::env::var("IRONRDP_WDF_USE_LOCAL_STUB") {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no")
        }
        Err(_) => false,
    };
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR set by Cargo"));
    let kits_root = std::env::var_os("IRONRDP_WINDOWS_KITS_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10"));

    let preferred_wdf_stub_version = std::env::var("IRONRDP_WDF_UMDF_STUB_VERSION")
        .unwrap_or_else(|_| "2.33".to_owned());

    if use_local && local_wdf_lib.is_file() {
        link_wdf_stub_with_patch_or_fallback(&local_wdf_lib, &out_dir, &manifest_dir, "bundled");
    } else if !use_local {
        if let Some(wdf_dir) = resolve_wdf_umdf_stub_lib_dir(&kits_root, &preferred_wdf_stub_version) {
            let source_stub = wdf_dir.join("WdfDriverStubUm.lib");
            link_wdf_stub_with_patch_or_fallback(&source_stub, &out_dir, &wdf_dir, "WDK");
        } else if local_wdf_lib.is_file() {
            println!("cargo:warning=Falling back to bundled WdfDriverStubUm.lib");
            link_wdf_stub_with_patch_or_fallback(&local_wdf_lib, &out_dir, &manifest_dir, "bundled");
        } else {
            println!(
                "cargo:warning=WdfDriverStubUm.lib not found. Set IRONRDP_WDF_UMDF_LIB_DIR, install the WDK, or add a bundled copy."
            );
        }
    } else {
        println!(
            "cargo:warning=Bundled WdfDriverStubUm.lib requested but missing; set IRONRDP_WDF_USE_LOCAL_STUB=0 to use WDK stubs"
        );
    }
    println!("cargo:rustc-link-lib=WdfDriverStubUm");

    // NOTE: This crate targets UMDF (a user-mode driver). Avoid kernel-mode driver
    // linker flags here; they break linking against the normal MSVC CRT.
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

    // Keep stub selection aligned with the INF's `UmdfExtensions=IddCx0102` by default.
    // Can be overridden for diagnostics/experimentation.
    let preferred_stub_version = std::env::var("IRONRDP_IDDCX_STUB_VERSION")
        .unwrap_or_else(|_| "1.2".to_owned());

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

fn prepare_patched_wdf_stub_lib(source_stub: &Path, out_dir: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(source_stub).ok()?;
    let mut patched = bytes;
    let build_number = std::env::var("IRONRDP_WDF_STUB_BUILD_NUMBER")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(26100);

    let mut patched_entries = 0usize;
    if patched.len() >= 16 {
        for offset in 0..=(patched.len() - 16) {
            let major = u32::from_le_bytes([
                patched[offset],
                patched[offset + 1],
                patched[offset + 2],
                patched[offset + 3],
            ]);
            let minor = u32::from_le_bytes([
                patched[offset + 4],
                patched[offset + 5],
                patched[offset + 6],
                patched[offset + 7],
            ]);
            let build = u32::from_le_bytes([
                patched[offset + 8],
                patched[offset + 9],
                patched[offset + 10],
                patched[offset + 11],
            ]);
            let func_count = u32::from_le_bytes([
                patched[offset + 12],
                patched[offset + 13],
                patched[offset + 14],
                patched[offset + 15],
            ]);

            if major == 2 && (1..=99).contains(&minor) && build == 0 && (128..=1024).contains(&func_count) {
                patched[offset + 8..offset + 12].copy_from_slice(&build_number.to_le_bytes());
                patched_entries += 1;
            }
        }
    }

    if patched_entries == 0 {
        println!(
            "cargo:warning=did not find WDF bind-info build field to patch in {}",
            source_stub.display()
        );
    } else {
        println!(
            "cargo:warning=patched {} WDF bind-info entry(ies) to build {}",
            patched_entries,
            build_number
        );
    }

    let patched_dir = out_dir.join("wdf-patched");
    std::fs::create_dir_all(&patched_dir).ok()?;
    let patched_path = patched_dir.join("WdfDriverStubUm.lib");
    std::fs::write(&patched_path, patched).ok()?;
    Some(patched_dir)
}

fn link_wdf_stub_with_patch_or_fallback(
    source_stub: &Path,
    out_dir: &Path,
    fallback_link_search_dir: &Path,
    source_label: &str,
) {
    if let Some(patched_dir) = prepare_patched_wdf_stub_lib(source_stub, out_dir) {
        println!("cargo:rustc-link-search=native={}", patched_dir.display());
        println!(
            "cargo:warning=Using patched {source_label} WdfDriverStubUm.lib from {}",
            source_stub.display()
        );
    } else {
        println!(
            "cargo:warning=failed to patch {source_label} WdfDriverStubUm.lib build number; using unpatched source"
        );
        println!("cargo:rustc-link-search=native={}", fallback_link_search_dir.display());
    }
}

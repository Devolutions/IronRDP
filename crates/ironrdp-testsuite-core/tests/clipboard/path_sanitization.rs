//! Tests for the `is_absolute_path`, `sanitize_file_path`, and
//! `is_windows_device_name` security helpers.
//!
//! These functions are private implementation details of `ironrdp-cliprdr`, exposed
//! via the `__test` feature gate for external testing. They are security-critical
//! pure functions that sanitize remote-peer file paths before use.

use ironrdp_cliprdr::{is_absolute_path, is_windows_device_name, sanitize_file_path};

// ── is_absolute_path ────────────────────────────────────────────────

#[test]
fn is_absolute_path_unix() {
    // Unix absolute paths
    assert!(is_absolute_path("/"));
    assert!(is_absolute_path("/path/to/file"));
    assert!(is_absolute_path("/usr/bin/bash"));
    assert!(is_absolute_path("/home/user/document.txt"));

    // Unix relative paths (should be false)
    assert!(!is_absolute_path("file.txt"));
    assert!(!is_absolute_path("subfolder/file.txt"));
    assert!(!is_absolute_path("./file.txt"));
    assert!(!is_absolute_path("../file.txt"));
}

#[test]
fn is_absolute_path_windows() {
    // Windows absolute paths with backslash
    assert!(is_absolute_path("C:\\"));
    assert!(is_absolute_path("C:\\path\\to\\file"));
    assert!(is_absolute_path("D:\\Windows\\System32"));
    assert!(is_absolute_path("Z:\\data\\file.txt"));

    // Windows absolute paths with forward slash
    assert!(is_absolute_path("C:/"));
    assert!(is_absolute_path("C:/path/to/file"));
    assert!(is_absolute_path("D:/Windows/System32"));

    // Windows drive-relative paths (C:relative - should be detected as absolute)
    assert!(is_absolute_path("C:file.txt"));
    assert!(is_absolute_path("D:relative"));
    assert!(is_absolute_path("Z:path"));
}

#[test]
fn is_absolute_path_unc() {
    // UNC paths with backslash
    assert!(is_absolute_path("\\\\server\\share"));
    assert!(is_absolute_path("\\\\server\\share\\file.txt"));
    assert!(is_absolute_path("\\\\192.168.1.1\\data"));

    // UNC paths with forward slash
    assert!(is_absolute_path("//server/share"));
    assert!(is_absolute_path("//server/share/file.txt"));
}

#[test]
fn is_absolute_path_long_paths() {
    // Windows long path prefix
    assert!(is_absolute_path("\\\\?\\C:\\very\\long\\path"));
    assert!(is_absolute_path("\\\\?\\D:\\path"));

    // Windows long UNC paths
    assert!(is_absolute_path("\\\\?\\UNC\\server\\share"));
    assert!(is_absolute_path("\\\\?\\UNC\\server\\share\\file.txt"));

    // Device paths
    assert!(is_absolute_path("\\\\.\\device"));
    assert!(is_absolute_path("\\\\.\\PhysicalDrive0"));
}

#[test]
fn is_absolute_path_relative() {
    // Simple relative paths
    assert!(!is_absolute_path("file.txt"));
    assert!(!is_absolute_path("document.pdf"));

    // Relative paths with subdirectories
    assert!(!is_absolute_path("folder\\file.txt"));
    assert!(!is_absolute_path("folder/file.txt"));
    assert!(!is_absolute_path("a\\b\\c\\file.txt"));

    // Current and parent directory references
    assert!(!is_absolute_path("."));
    assert!(!is_absolute_path(".."));
    assert!(!is_absolute_path(".\\file.txt"));
    assert!(!is_absolute_path("../file.txt"));

    // Empty string
    assert!(!is_absolute_path(""));
}

// ── sanitize_file_path ──────────────────────────────────────────────

#[test]
fn sanitize_file_path_basic() {
    let result = sanitize_file_path("file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, None);
}

#[test]
fn sanitize_file_path_strips_trailing_nulls() {
    let result = sanitize_file_path("file.txt\0\0\0").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, None);
}

#[test]
fn sanitize_file_path_strips_traversal_preserves_relative() {
    // Traversal components are stripped, but remaining safe path is preserved
    let result = sanitize_file_path("../../../etc/passwd").unwrap();
    assert_eq!(result.name, "passwd");
    assert_eq!(result.relative_path, Some("etc".to_owned()));

    let result = sanitize_file_path("..\\..\\system32\\config\\SAM").unwrap();
    assert_eq!(result.name, "SAM");
    assert_eq!(result.relative_path, Some("system32\\config".to_owned()));
}

#[test]
fn sanitize_file_path_windows_absolute_path() {
    // Absolute paths are stripped to basename only (drive letter removed)
    let result = sanitize_file_path("C:\\Users\\victim\\Desktop\\file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, Some("Users\\victim\\Desktop".to_owned()));
}

#[test]
fn sanitize_file_path_relative_path_preserved() {
    // Per MS-RDPECLIP 3.1.1.2, file lists use relative paths
    let result = sanitize_file_path("temp\\file1.txt").unwrap();
    assert_eq!(result.name, "file1.txt");
    assert_eq!(result.relative_path, Some("temp".to_owned()));

    let result = sanitize_file_path("folder\\sub\\file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, Some("folder\\sub".to_owned()));
}

#[test]
fn sanitize_file_path_unix_relative_path() {
    // Unix-style separators are also handled
    let result = sanitize_file_path("temp/subdir/file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, Some("temp\\subdir".to_owned()));
}

#[test]
fn sanitize_file_path_mixed_separators() {
    let result = sanitize_file_path("folder/sub\\file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, Some("folder\\sub".to_owned()));
}

#[test]
fn sanitize_file_path_rejects_empty() {
    assert!(sanitize_file_path("").is_none());
    assert!(sanitize_file_path("\0\0\0").is_none());
}

#[test]
fn sanitize_file_path_rejects_traversal_only() {
    assert!(sanitize_file_path("..").is_none());
    assert!(sanitize_file_path(".").is_none());
    assert!(sanitize_file_path("../..").is_none());
}

#[test]
fn sanitize_file_path_rejects_embedded_nulls() {
    // Embedded nulls could cause C-based filesystem APIs to truncate the name
    assert!(sanitize_file_path("safe\0evil").is_none());
    assert!(sanitize_file_path("file\0.txt").is_none());
    assert!(sanitize_file_path("dir/file\0name.txt").is_none());
}

#[test]
fn sanitize_file_path_directory_entry() {
    // Directory entries end with a separator; the name is the dir itself
    let result = sanitize_file_path("temp\\").unwrap();
    assert_eq!(result.name, "temp");
    assert_eq!(result.relative_path, None);

    let result = sanitize_file_path("folder\\subfolder\\").unwrap();
    assert_eq!(result.name, "subfolder");
    assert_eq!(result.relative_path, Some("folder".to_owned()));
}

#[test]
fn sanitize_file_path_unc_path() {
    // UNC paths - after split and filtering, first component is the server name.
    // Since we can't detect UNC purely from components (prefix is stripped by split),
    // the server/share components become relative path parts.
    let result = sanitize_file_path("\\\\server\\share\\file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    // Server and share become part of the relative path since we can't
    // distinguish them from regular path components after splitting.
    assert_eq!(result.relative_path, Some("server\\share".to_owned()));
}

#[test]
fn sanitize_file_path_long_path_prefix() {
    // Windows long path prefix \\?\C:\path
    let result = sanitize_file_path("\\\\?\\C:\\Users\\file.txt").unwrap();
    assert_eq!(result.name, "file.txt");
    assert_eq!(result.relative_path, Some("Users".to_owned()));
}

#[test]
fn sanitize_file_path_triple_dot_not_traversal() {
    // "..." is not a traversal component, it's a valid (if unusual) filename
    let result = sanitize_file_path("...").unwrap();
    assert_eq!(result.name, "...");
    assert_eq!(result.relative_path, None);
}

#[test]
fn sanitize_file_path_allows_windows_reserved_device_names() {
    // Windows reserved device names pass through sanitize_file_path.
    // Backends that write to disk on Windows must reject these separately.
    for name in ["CON", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "LPT9"] {
        let result = sanitize_file_path(name).unwrap();
        assert_eq!(result.name, name, "reserved name {name} should pass through");
        assert_eq!(result.relative_path, None);
    }

    // Also verify they pass through with extensions and in subdirectories
    let result = sanitize_file_path("CON.txt").unwrap();
    assert_eq!(result.name, "CON.txt");

    let result = sanitize_file_path("folder\\NUL").unwrap();
    assert_eq!(result.name, "NUL");
    assert_eq!(result.relative_path, Some("folder".to_owned()));
}

#[test]
fn sanitize_file_path_unicode_lookalike_separators_pass_through() {
    // Unicode look-alike separators are NOT treated as path separators.
    // This is a documented limitation - the sanitizer only splits on
    // ASCII '/' (U+002F) and '\' (U+005C). OS-level normalization
    // handles these if needed.
    let fullwidth_solidus = "folder\u{FF0F}file.txt"; // U+FF0F fullwidth solidus
    let result = sanitize_file_path(fullwidth_solidus).unwrap();
    assert_eq!(
        result.name, fullwidth_solidus,
        "fullwidth solidus should not split path"
    );
    assert_eq!(result.relative_path, None);

    let fullwidth_reverse = "folder\u{FF3C}file.txt"; // U+FF3C fullwidth reverse solidus
    let result = sanitize_file_path(fullwidth_reverse).unwrap();
    assert_eq!(
        result.name, fullwidth_reverse,
        "fullwidth reverse solidus should not split path"
    );
    assert_eq!(result.relative_path, None);

    let division_slash = "folder\u{2215}file.txt"; // U+2215 division slash
    let result = sanitize_file_path(division_slash).unwrap();
    assert_eq!(result.name, division_slash, "division slash should not split path");
    assert_eq!(result.relative_path, None);
}

// ── is_windows_device_name ──────────────────────────────────────────

#[test]
fn device_name_detects_standard_names() {
    for name in ["CON", "PRN", "AUX", "NUL"] {
        assert!(is_windows_device_name(name), "{name} should be detected as device name");
    }
}

#[test]
fn device_name_detects_numbered_ports() {
    for i in 1..=9 {
        let com = format!("COM{i}");
        let lpt = format!("LPT{i}");
        assert!(is_windows_device_name(&com), "{com} should be detected");
        assert!(is_windows_device_name(&lpt), "{lpt} should be detected");
    }
}

#[test]
fn device_name_is_case_insensitive() {
    assert!(is_windows_device_name("con"));
    assert!(is_windows_device_name("Con"));
    assert!(is_windows_device_name("nul"));
    assert!(is_windows_device_name("Lpt1"));
}

#[test]
fn device_name_detects_names_with_extension() {
    assert!(is_windows_device_name("CON.txt"));
    assert!(is_windows_device_name("nul.tar.gz"));
    assert!(is_windows_device_name("COM1.log"));
}

#[test]
fn device_name_rejects_safe_names() {
    assert!(!is_windows_device_name("file.txt"));
    assert!(!is_windows_device_name("CONSOLE"));
    assert!(!is_windows_device_name("COM10"));
    assert!(!is_windows_device_name("LPT10"));
    assert!(!is_windows_device_name(""));
    assert!(!is_windows_device_name("CONX"));
    assert!(!is_windows_device_name("NULLIFY"));
    assert!(!is_windows_device_name(".hidden"));
}

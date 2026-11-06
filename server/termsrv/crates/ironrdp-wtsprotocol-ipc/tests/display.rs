use ironrdp_wtsprotocol_ipc::display::DisplaySize;
use serde as _;
use serde_json as _;

// oracle: spec MS-RDPEDISP §3.1.5.2; a layout request changes its remote session.
#[test]
fn publishing_a_new_size_preserves_other_sessions() {
    let directory = std::env::temp_dir().join(format!("ironrdp-display-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let first = DisplaySize::new(1280, 720).unwrap();
    let second = DisplaySize::new(1600, 900).unwrap();
    let resized = DisplaySize::new(1366, 768).unwrap();
    first.publish(&directory, 5, 1).unwrap();
    second.publish(&directory, 6, 2).unwrap();
    resized.publish(&directory, 5, 1).unwrap();
    assert_eq!(DisplaySize::read(&directory, 5).unwrap(), resized);
    assert_eq!(DisplaySize::read(&directory, 6).unwrap(), second);
    assert_eq!(
        DisplaySize::read(&directory, 7).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    std::fs::remove_file(directory.join("display-size-5.bin")).unwrap();
    std::fs::remove_file(directory.join("display-size-6.bin")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

// oracle: spec MS-RDPEDISP §2.2.2.2.1; widths are even and dimensions are 200..=8192.
#[test]
fn rejects_dimensions_outside_the_display_control_domain() {
    for (width, height) in [(198, 768), (201, 768), (8194, 768), (1280, 199), (1280, 8193)] {
        assert_eq!(
            DisplaySize::new(width, height).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput,
            "{width}x{height}"
        );
    }
}

// oracle: spec MS-RDPEDISP §2.2.2.1; the advertised single-monitor area is 3840 * 2400.
#[test]
fn rejects_a_layout_larger_than_the_advertised_area() {
    assert_eq!(
        DisplaySize::new(3840, 2401).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

// oracle: property round-trip
#[test]
fn boundary_dimensions_survive_publication() {
    let directory = std::env::temp_dir().join(format!("ironrdp-display-boundaries-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    for (width, height) in [(200, 200), (8192, 200), (200, 8192), (3840, 2400)] {
        let size = DisplaySize::new(width, height).unwrap();
        size.publish(&directory, 5, 1).unwrap();
        let read = DisplaySize::read(&directory, 5).unwrap();
        assert_eq!((read.width(), read.height()), (width, height));
    }
    std::fs::remove_file(directory.join("display-size-5.bin")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

// oracle: mutant refuse malformed, truncated, oversized, or out-of-domain driver requests
#[test]
fn rejects_invalid_file_records() {
    let directory = std::env::temp_dir().join(format!("ironrdp-display-invalid-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("display-size-5.bin");
    for bytes in [
        b"IDS1\x00\x05\xd0".as_slice(),
        b"IDS2\x00\x05\xd0\x02".as_slice(),
        b"IDS1\x00\x05\xd0\x02\x00".as_slice(),
        b"IDS1\x01\x05\xd0\x02".as_slice(),
    ] {
        std::fs::write(&path, bytes).unwrap();
        assert!(DisplaySize::read(&directory, 5).is_err(), "{bytes:?}");
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

// oracle: mutant prevent writing a display request for unassigned session zero
#[test]
fn rejects_an_unassigned_session() {
    let directory = std::env::temp_dir();
    let size = DisplaySize::new(1280, 720).unwrap();
    assert_eq!(
        size.publish(&directory, 0, 1).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        DisplaySize::read(&directory, 0).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

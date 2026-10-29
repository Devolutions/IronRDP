use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::TsUrbSelectConfig;
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::utils::UsbConfigDesc;

/// An interface descriptor (9 bytes, mass-storage/SCSI/BOT) followed by a bulk-IN
/// endpoint descriptor (7 bytes) — the kind of payload that follows the 9-byte
/// configuration-descriptor header on a real device.
const TRAILING: [u8; 16] = [
    0x09, 0x04, 0x00, 0x00, 0x01, 0x08, 0x06, 0x50, 0x00, // interface descriptor
    0x07, 0x05, 0x81, 0x02, 0x00, 0x02, 0x00, // endpoint descriptor (bulk IN, 512)
];

fn full_config_desc() -> UsbConfigDesc {
    UsbConfigDesc {
        length: 9,
        descriptor_type: 0x02,
        total_length: u16::try_from(9 + TRAILING.len()).expect("fits in u16"),
        num_interfaces: 1,
        configuration_value: 1,
        configuration: 0,
        attributes: 0x80,
        max_power: 50,
        trailing: TRAILING.to_vec(),
    }
}

/// [MS-RDPEUSB] 2.2.9.2 requires the full configuration descriptor (all
/// interface/endpoint/class-specific bytes, `wTotalLength` in total) when
/// `ConfigurationDescriptorIsValid` is set — real Windows walks `wTotalLength`
/// and rejects a header-only descriptor with `0x80070057`. The encoded form must
/// therefore span `total_length` bytes and round-trip.
#[test]
fn full_config_descriptor_round_trips() {
    let original = full_config_desc();
    let encoded = encode_vec(&original).expect("encode should succeed");
    assert_eq!(encoded.len(), usize::from(original.total_length));
    let decoded: UsbConfigDesc = decode(&encoded).expect("full descriptor should decode");
    assert_eq!(original, decoded);
}

/// A header-only descriptor (only 9 bytes present even though `wTotalLength`
/// claims more) must still decode, with `trailing` left empty.
#[test]
fn header_only_descriptor_still_decodes() {
    // 9-byte header claiming wTotalLength = 759 (0x02F7), with no trailing bytes.
    let header_only = [0x09, 0x02, 0xF7, 0x02, 0x03, 0x01, 0x00, 0x80, 0x32];
    let decoded: UsbConfigDesc = decode(&header_only).expect("header-only descriptor should decode");
    assert_eq!(decoded.total_length, 759);
    assert!(decoded.trailing.is_empty());
}

/// Encoding must refuse a descriptor whose header disagrees with the bytes that
/// follow — a mismatched `wTotalLength`, or a `bLength` that is not the 9-byte
/// header. Real Windows rejects such a descriptor with `0x80070057`, so the
/// inconsistency is caught before it reaches the wire.
#[test]
fn inconsistent_header_fails_to_encode() {
    let mut wrong_total = full_config_desc();
    wrong_total.total_length += 4; // claims more than `trailing` provides
    assert!(encode_vec(&wrong_total).is_err());

    let mut wrong_length = full_config_desc();
    wrong_length.length = 10; // bLength must be the 9-byte header length
    assert!(encode_vec(&wrong_length).is_err());
}

/// The descriptor is the last field of TS_URB_SELECT_CONFIGURATION; the full
/// (trailing-carrying) form must round-trip through the containing URB too.
#[test]
fn select_configuration_carries_full_descriptor() {
    let original = TsUrbSelectConfig {
        usbd_ifaces: Vec::new(),
        desc: Some(full_config_desc()),
    };
    let encoded = encode_vec(&original).expect("encode should succeed");
    let decoded: TsUrbSelectConfig = decode(&encoded).expect("URB with full descriptor should decode");
    assert_eq!(original, decoded);
}

// Windows submits OUT control requests without a data stage, such as a HID SET_IDLE, as
// TRANSFER_IN_REQUEST messages whose transfer flags keep the OUT direction.
#[test]
fn transfer_in_accepts_out_control_requests_without_data() {
    use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::{TsUrbIn, TsUrbInKind};

    // TS_URB_HEADER: Size 20, URB_FUNCTION_CLASS_INTERFACE, RequestId 7; TransferFlags 0 (OUT);
    // RequestTypeReservedBits, Request SET_IDLE, Value 0, Index 1, Padding.
    let ts_urb = [20, 0, 27, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0x0a, 0, 0, 1, 0, 0, 0];
    let urb: TsUrbIn = decode(&ts_urb).expect("a data-less OUT class request decodes");
    let TsUrbInKind::VendorClassReq(request) = urb.kind.clone() else {
        panic!("expected a vendor or class request");
    };
    assert_eq!(request.transfer_flags, 0);
    assert_eq!(request.request, 0x0a);
    assert_eq!(request.index, 1);

    // Backends that hand TS_URBs to the operating system encode it back as it was.
    let urb = TsUrbIn {
        kind: TsUrbInKind::VendorClassReq(request),
        header: urb.header,
    };
    assert_eq!(encode_vec(&urb).expect("it encodes back"), ts_urb);
}

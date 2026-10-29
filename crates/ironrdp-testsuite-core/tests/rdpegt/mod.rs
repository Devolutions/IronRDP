use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpegt::{GeometryClient, GeometryEvent, GeometryRect, MappedGeometry, MappedGeometryPacket, RegionData};

const MAPPING_ID: u64 = 0x8000_7ABA_0004_0222;

/// MS-RDPEGT 4.1, followed by its reserved byte.
const UPDATE_FIXTURE: &str = "78000000 01000000 22020400 BA7A0080 01000000 00000000 E2010300 00000000 \
                              10000000 8A000000 F0010000 7E010000 23010000 72000000 78040000 CA020000 \
                              02000000 30000000 20000000 01000000 01000000 00000000 00000000 00000000 \
                              E0010000 F4000000 00000000 00000000 E0010000 F4000000 00";

/// MS-RDPEGT 4.2, followed by its reserved byte.
const CLEAR_FIXTURE: &str = "48000000 01000000 22020400 BA7A0080 02000000 00000000 00000000 00000000 \
                             00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
                             00000000 00000000 00";

fn bytes(fixture: &str) -> Vec<u8> {
    hex::decode(fixture.replace(' ', "")).unwrap()
}

fn rect(left: i32, top: i32, right: i32, bottom: i32) -> GeometryRect {
    GeometryRect {
        left,
        top,
        right,
        bottom,
    }
}

fn update_packet() -> MappedGeometryPacket {
    MappedGeometryPacket {
        version: 1,
        mapping_id: MAPPING_ID,
        geometry: Some(MappedGeometry {
            top_level_id: 0x0003_01E2,
            rect: rect(16, 138, 496, 382),
            top_level_rect: rect(291, 114, 1144, 714),
            region: Some(RegionData {
                bound: rect(0, 0, 480, 244),
                rects: vec![rect(0, 0, 480, 244)],
            }),
        }),
    }
}

#[test]
fn update_and_clear_match_the_specification_examples() {
    let update = bytes(UPDATE_FIXTURE);
    assert_eq!(decode::<MappedGeometryPacket>(&update).unwrap(), update_packet());
    // The reserved trailing byte is not counted by cbGeometryData.
    assert_eq!(encode_vec(&update_packet()).unwrap(), update[..120]);

    let clear = bytes(CLEAR_FIXTURE);
    let packet = MappedGeometryPacket {
        version: 1,
        mapping_id: MAPPING_ID,
        geometry: None,
    };
    assert_eq!(decode::<MappedGeometryPacket>(&clear).unwrap(), packet);
    assert_eq!(encode_vec(&packet).unwrap(), clear[..72]);
}

#[test]
fn malformed_packets_are_rejected() {
    let mut update = bytes(UPDATE_FIXTURE);
    // cbGeometryData larger than the received data.
    update[0] = 0xFF;
    assert!(decode::<MappedGeometryPacket>(&update).is_err());

    // Left greater than Right.
    let mut update = bytes(UPDATE_FIXTURE);
    update[32..36].copy_from_slice(&1000i32.to_le_bytes());
    assert!(decode::<MappedGeometryPacket>(&update).is_err());

    // Unsupported geometry type.
    let mut update = bytes(UPDATE_FIXTURE);
    update[64] = 0x03;
    assert!(decode::<MappedGeometryPacket>(&update).is_err());

    // Region rectangle count beyond the geometry buffer.
    let mut update = bytes(UPDATE_FIXTURE);
    update[80] = 0x02;
    assert!(decode::<MappedGeometryPacket>(&update).is_err());

    // Unknown update type.
    let mut clear = bytes(CLEAR_FIXTURE);
    clear[16] = 0x03;
    assert!(decode::<MappedGeometryPacket>(&clear).is_err());
}

#[test]
fn visible_region_is_placed_in_desktop_coordinates() {
    let geometry = update_packet().geometry.unwrap();
    assert_eq!(geometry.desktop_rect(), rect(307, 252, 787, 496));
    assert_eq!(geometry.visible_rects(), [rect(307, 252, 787, 496)]);

    // Two visible parts, the second partly outside the tracked rectangle.
    let partial = MappedGeometry {
        region: Some(RegionData {
            bound: rect(0, 0, 480, 244),
            rects: vec![rect(0, 0, 100, 50), rect(400, 200, 600, 300)],
        }),
        ..geometry
    };
    assert_eq!(
        partial.visible_rects(),
        [rect(307, 252, 407, 302), rect(707, 452, 787, 496)]
    );

    // An empty region is ignored.
    let empty = MappedGeometry {
        region: Some(RegionData {
            bound: rect(0, 0, 0, 0),
            rects: Vec::new(),
        }),
        ..geometry
    };
    assert_eq!(empty.visible_rects(), [rect(307, 252, 787, 496)]);
}

#[test]
fn client_tracks_mappings_and_notifies_changes() {
    let events: Arc<Mutex<Vec<(u64, bool)>>> = Arc::default();
    let recorded = Arc::clone(&events);
    let mut client = GeometryClient::new().with_observer(Box::new(move |event| {
        recorded.lock().unwrap().push(match event {
            GeometryEvent::Updated { mapping_id, .. } => (mapping_id, true),
            GeometryEvent::Cleared { mapping_id } => (mapping_id, false),
        });
    }));
    client.start(5).unwrap();

    assert!(client.process(5, &bytes(UPDATE_FIXTURE)).unwrap().is_empty());
    assert_eq!(client.mapping(MAPPING_ID), update_packet().geometry.as_ref());
    // Malformed packets are ignored without failing the channel.
    assert!(client.process(5, &[0x01]).unwrap().is_empty());
    client.process(5, &bytes(CLEAR_FIXTURE)).unwrap();
    assert!(client.mapping(MAPPING_ID).is_none());
    // Clearing an unknown mapping is ignored.
    client.process(5, &bytes(CLEAR_FIXTURE)).unwrap();

    client.process(5, &bytes(UPDATE_FIXTURE)).unwrap();
    client.close(5);
    assert_eq!(client.mappings().count(), 0);
    assert_eq!(
        *events.lock().unwrap(),
        [
            (MAPPING_ID, true),
            (MAPPING_ID, false),
            (MAPPING_ID, true),
            (MAPPING_ID, false)
        ]
    );
}

use ironrdp_core::{Decode as _, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_rdpel::client::LocationClient;
use ironrdp_rdpel::pdu::{
    BaseLocation3dPdu, FourByteFloat, FourByteSignedInteger, LocationPdu, LocationSource, ProtocolVersion, ReadyPdu,
};

fn roundtrip(pdu: LocationPdu) -> LocationPdu {
    let encoded = encode_vec(&pdu).expect("encode location PDU");
    ironrdp_core::decode(&encoded).expect("decode location PDU")
}

fn decode_dvc_message(message: &ironrdp_dvc::DvcMessage) -> LocationPdu {
    let encoded = encode_vec(message.as_ref()).expect("encode DVC message");
    ironrdp_core::decode(&encoded).expect("decode location PDU")
}

fn decode_svc_message(message: &ironrdp_svc::SvcMessage) -> LocationPdu {
    let encoded = message.encode_unframed_pdu().expect("encode unframed DRDYNVC PDU");
    let dvc: DrdynvcClientPdu = ironrdp_core::decode(&encoded).expect("decode DRDYNVC PDU");
    let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) = dvc else {
        panic!("expected unsplit DRDYNVC data");
    };
    ironrdp_core::decode(data.data()).expect("decode location PDU")
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < f64::EPSILON);
}

#[test]
fn ready_pdu_has_specified_wire_layout() {
    let pdu = LocationPdu::ServerReady(ReadyPdu {
        protocol_version: ProtocolVersion::V2,
        flags: Some(0),
    });
    let encoded = encode_vec(&pdu).expect("encode server ready");

    assert_eq!(
        encoded,
        [
            0x01, 0x00, // PDUTYPE_SERVER_READY
            0x0E, 0x00, 0x00, 0x00, // pduLength
            0x00, 0x00, 0x02, 0x00, // RDPLOCATION_PROTOCOL_VERSION_200
            0x00, 0x00, 0x00, 0x00, // flags
        ]
    );
    assert_eq!(roundtrip(pdu), pdu);
}

#[test]
fn variable_width_numbers_match_spec_bit_layout() {
    let integer = FourByteSignedInteger::new(-32).expect("valid signed integer");
    assert_eq!(encode_vec(&integer).expect("encode integer"), [0x60, 0x20]);
    let decoded_integer: FourByteSignedInteger = ironrdp_core::decode(&[0x60, 0x20]).expect("decode signed integer");
    assert_eq!(decoded_integer.value(), -32);

    let coordinate = FourByteFloat::new(-45.5).expect("valid coordinate");
    assert_eq!(encode_vec(&coordinate).expect("encode coordinate"), [0x65, 0xC7]);
    let decoded_coordinate: FourByteFloat = ironrdp_core::decode(&[0x65, 0xC7]).expect("decode coordinate");
    assert_close(decoded_coordinate.value(), -45.5);

    let precise = FourByteFloat::new(1.2345678).expect("valid seven-place float");
    assert!((precise.value() - 1.2345678).abs() < f64::EPSILON);
}

#[test]
fn base_location_roundtrips_without_optional_version_two_fields() {
    let pdu =
        LocationPdu::BaseLocation3d(BaseLocation3dPdu::coordinates(45.5, -73.5, 123).expect("valid base location"));
    let encoded = encode_vec(&pdu).expect("encode base location");

    assert_eq!(
        encoded,
        [
            0x03, 0x00, // PDUTYPE_BASE_LOCATION3D
            0x0C, 0x00, 0x00, 0x00, // pduLength
            0x45, 0xC7, // latitude
            0x66, 0xDF, // longitude
            0x40, 0x7B, // altitude
        ]
    );
    assert_eq!(roundtrip(pdu), pdu);
}

#[test]
fn base_location_roundtrips_all_version_two_optional_fields() {
    let pdu = LocationPdu::BaseLocation3d(BaseLocation3dPdu {
        latitude: FourByteFloat::new(45.5).expect("valid latitude"),
        longitude: FourByteFloat::new(-73.5).expect("valid longitude"),
        altitude: FourByteSignedInteger::new(123).expect("valid altitude"),
        speed: Some(FourByteFloat::new(1.5).expect("valid speed")),
        heading: Some(FourByteFloat::new(90.0).expect("valid heading")),
        horizontal_accuracy: Some(FourByteFloat::new(5.0).expect("valid accuracy")),
        source: Some(LocationSource::Gnss),
    });

    assert_eq!(roundtrip(pdu), pdu);
}

#[test]
fn encoder_rejects_reserved_flags_and_incomplete_optional_fields() {
    assert!(
        encode_vec(&LocationPdu::ClientReady(ReadyPdu {
            protocol_version: ProtocolVersion::V1,
            flags: Some(1),
        }))
        .is_err()
    );
    assert!(
        encode_vec(&LocationPdu::BaseLocation3d(BaseLocation3dPdu {
            latitude: FourByteFloat::new(45.5).expect("valid latitude"),
            longitude: FourByteFloat::new(-73.5).expect("valid longitude"),
            altitude: FourByteSignedInteger::new(123).expect("valid altitude"),
            speed: Some(FourByteFloat::new(1.5).expect("valid speed")),
            heading: None,
            horizontal_accuracy: None,
            source: None,
        }))
        .is_err()
    );
}

#[test]
fn client_requires_ready_exchange_and_selects_delta_dimension() {
    let mut client = LocationClient::new();
    client.start(7).expect("start location channel");
    assert!(!client.ready());
    assert!(client.send_location(45.5, -73.5, 100).is_err());

    let server_ready = LocationPdu::ServerReady(ReadyPdu {
        protocol_version: ProtocolVersion::V2,
        flags: Some(0),
    });
    let response = client
        .process(7, &encode_vec(&server_ready).expect("encode server ready"))
        .expect("process server ready");
    assert!(client.ready());
    assert_eq!(
        decode_dvc_message(&response[0]),
        LocationPdu::ClientReady(ReadyPdu::v1())
    );

    assert!(client.send_location(91.0, 0.0, 100).is_err());

    let base = client
        .send_location(45.501239, -73.567899, 100)
        .expect("send base location");
    assert!(matches!(decode_svc_message(&base[0]), LocationPdu::BaseLocation3d(_)));

    let delta_2d = client
        .send_location(45.501249, -73.567889, 100)
        .expect("send 2D location delta");
    let LocationPdu::Location2dDelta(delta) = decode_svc_message(&delta_2d[0]) else {
        panic!("expected 2D location delta");
    };
    assert_close(delta.latitude_delta.value(), -0.00001);
    assert_close(delta.longitude_delta.value(), -0.00001);

    let delta_3d = client
        .send_location(45.501259, -73.567879, 105)
        .expect("send 3D location delta");
    let LocationPdu::Location3dDelta(delta) = decode_svc_message(&delta_3d[0]) else {
        panic!("expected 3D location delta");
    };
    assert_close(delta.latitude_delta.value(), -0.00001);
    assert_close(delta.longitude_delta.value(), -0.00001);
    assert_eq!(delta.altitude_delta.value(), -5);

    client.close(7);
    assert!(!client.ready());
    assert!(client.send_location(45.5, -73.5, 100).is_err());
}

#[test]
fn uncommitted_location_does_not_advance_delta_state() {
    let mut client = LocationClient::new();
    client.start(7).expect("start location channel");
    let server_ready = LocationPdu::ServerReady(ReadyPdu::v1());
    client
        .process(7, &encode_vec(&server_ready).expect("encode server ready"))
        .expect("process server ready");

    let (_, first_messages) = client
        .prepare_location(45.5, -73.5, 100)
        .expect("prepare first location");
    assert!(matches!(
        decode_svc_message(&first_messages[0]),
        LocationPdu::BaseLocation3d(_)
    ));

    let (second, second_messages) = client
        .prepare_location(45.6, -73.6, 100)
        .expect("prepare replacement location");
    assert!(matches!(
        decode_svc_message(&second_messages[0]),
        LocationPdu::BaseLocation3d(_)
    ));

    client.commit_location(second);
    let (_, third_messages) = client
        .prepare_location(45.7, -73.7, 100)
        .expect("prepare delta location");
    assert!(matches!(
        decode_svc_message(&third_messages[0]),
        LocationPdu::Location2dDelta(_)
    ));
}

#[test]
fn location_decoder_rejects_mismatched_pdu_length() {
    let mut encoded = encode_vec(&LocationPdu::ServerReady(ReadyPdu::v1())).expect("encode ready PDU");
    encoded[2] = 0x0D;

    assert!(LocationPdu::decode(&mut ironrdp_core::ReadCursor::new(&encoded)).is_err());
}

#[test]
fn client_ignores_malformed_server_pdu() {
    let mut client = LocationClient::new();
    client.start(7).expect("start location channel");
    let mut encoded = encode_vec(&LocationPdu::ServerReady(ReadyPdu::v1())).expect("encode ready PDU");
    encoded[2] = 0x0D;

    assert!(client.process(7, &encoded).expect("ignore malformed PDU").is_empty());
    assert!(!client.ready());
}

// oracle: spec MS-RDPEL §§1.3, 2.2.2.1-2.2.2.5, 3.1.5.1, and 3.2.5.
#[test]
fn server_reconstructs_base_and_delta_locations_and_rejects_out_of_sequence_updates() {
    use std::sync::{Arc, Mutex};

    use ironrdp_rdpel::pdu::{Location2dDeltaPdu, Location3dDeltaPdu};
    use ironrdp_rdpel::server::{LocationFix, LocationHandler, LocationServer};

    struct Recorder {
        ready: Arc<Mutex<Vec<ProtocolVersion>>>,
        fixes: Arc<Mutex<Vec<LocationFix>>>,
        closed: Arc<Mutex<usize>>,
    }

    impl LocationHandler for Recorder {
        fn on_ready(&mut self, version: ProtocolVersion) {
            self.ready.lock().unwrap().push(version);
        }

        fn on_location(&mut self, fix: LocationFix) {
            self.fixes.lock().unwrap().push(fix);
        }

        fn on_closed(&mut self) {
            *self.closed.lock().unwrap() += 1;
        }
    }

    let ready = Arc::new(Mutex::new(Vec::new()));
    let fixes = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(Mutex::new(0usize));
    let mut server = LocationServer::new(Box::new(Recorder {
        ready: Arc::clone(&ready),
        fixes: Arc::clone(&fixes),
        closed: Arc::clone(&closed),
    }));
    let mut client = LocationClient::new();

    let server_start = server.start(11).expect("start server");
    client.start(11).expect("start client");
    let client_ready = client
        .process(11, &encode_vec(&decode_dvc_message(&server_start[0])).unwrap())
        .expect("client processes server ready");
    server
        .process(11, &encode_vec(&decode_dvc_message(&client_ready[0])).unwrap())
        .expect("server processes client ready");
    assert_eq!(*ready.lock().unwrap(), [ProtocolVersion::V1]);

    // Delta before BaseLocation3d must be ignored.
    let premature_delta = encode_vec(&LocationPdu::Location2dDelta(
        Location2dDeltaPdu::coordinates(0.01, 0.01).unwrap(),
    ))
    .unwrap();
    server.process(11, &premature_delta).unwrap();
    assert!(fixes.lock().unwrap().is_empty());

    for svc in client.send_location(0.3, -73.3, 42).unwrap() {
        server
            .process(11, &encode_vec(&decode_svc_message(&svc)).unwrap())
            .unwrap();
    }
    for svc in client.send_location(0.2, -73.2, 42).unwrap() {
        server
            .process(11, &encode_vec(&decode_svc_message(&svc)).unwrap())
            .unwrap();
    }
    for svc in client.send_location(0.0, -73.0, 55).unwrap() {
        server
            .process(11, &encode_vec(&decode_svc_message(&svc)).unwrap())
            .unwrap();
    }

    let recorded = fixes.lock().unwrap().clone();
    assert_eq!(recorded.len(), 3);
    assert_close(recorded[0].latitude, 0.3);
    assert_close(recorded[0].longitude, -73.3);
    assert_eq!(recorded[0].altitude, 42);
    assert_close(recorded[1].latitude, 0.2);
    assert_close(recorded[1].longitude, -73.2);
    assert_eq!(recorded[1].altitude, 42);
    assert_close(recorded[2].latitude, 0.0);
    assert_close(recorded[2].longitude, -73.0);
    assert_eq!(recorded[2].altitude, 55);

    // An out-of-range delta must invalidate the server's delta base until a new BaseLocation3d arrives.
    let bad_delta = encode_vec(&LocationPdu::Location2dDelta(
        Location2dDeltaPdu::coordinates(-100.0, 0.0).unwrap(),
    ))
    .unwrap();
    server.process(11, &bad_delta).unwrap();
    let follow_delta = encode_vec(&LocationPdu::Location2dDelta(
        Location2dDeltaPdu::coordinates(-0.125, 0.0).unwrap(),
    ))
    .unwrap();
    server.process(11, &follow_delta).unwrap();
    assert_eq!(fixes.lock().unwrap().len(), 3);

    // Re-establish a V2 base with speed and heading, then step through non-dyadic decimal deltas to 0.0 and 360.0.
    let v2_base = encode_vec(&LocationPdu::BaseLocation3d(BaseLocation3dPdu {
        latitude: FourByteFloat::new(46.3).unwrap(),
        longitude: FourByteFloat::new(-71.3).unwrap(),
        altitude: FourByteSignedInteger::new(10).unwrap(),
        speed: Some(FourByteFloat::new(0.3).unwrap()),
        heading: Some(FourByteFloat::new(359.7).unwrap()),
        horizontal_accuracy: Some(FourByteFloat::new(5.0).unwrap()),
        source: Some(LocationSource::Gnss),
    }))
    .unwrap();
    server.process(11, &v2_base).unwrap();

    let v2_delta_2d = encode_vec(&LocationPdu::Location2dDelta(Location2dDeltaPdu {
        latitude_delta: FourByteFloat::new(0.1).unwrap(),
        longitude_delta: FourByteFloat::new(-0.1).unwrap(),
        speed_delta: Some(FourByteFloat::new(0.1).unwrap()),
        heading_delta: Some(FourByteFloat::new(-0.1).unwrap()),
    }))
    .unwrap();
    server.process(11, &v2_delta_2d).unwrap();

    let v2_delta_3d = encode_vec(&LocationPdu::Location3dDelta(Location3dDeltaPdu {
        latitude_delta: FourByteFloat::new(0.2).unwrap(),
        longitude_delta: FourByteFloat::new(-0.2).unwrap(),
        altitude_delta: FourByteSignedInteger::new(-5).unwrap(),
        speed_delta: Some(FourByteFloat::new(0.2).unwrap()),
        heading_delta: Some(FourByteFloat::new(-0.2).unwrap()),
    }))
    .unwrap();
    server.process(11, &v2_delta_3d).unwrap();

    let recorded = fixes.lock().unwrap().clone();
    assert_eq!(recorded.len(), 6);
    assert_close(recorded[3].latitude, 46.3);
    assert_close(recorded[3].longitude, -71.3);
    assert_eq!(recorded[3].altitude, 10);
    assert_close(recorded[3].speed.unwrap(), 0.3);
    assert_close(recorded[3].heading.unwrap(), 359.7);
    assert_close(recorded[3].horizontal_accuracy.unwrap(), 5.0);
    assert_eq!(recorded[3].source, Some(LocationSource::Gnss));

    assert_close(recorded[4].latitude, 46.2);
    assert_close(recorded[4].longitude, -71.2);
    assert_eq!(recorded[4].altitude, 10);
    assert_close(recorded[4].speed.unwrap(), 0.2);
    assert_close(recorded[4].heading.unwrap(), 359.8);

    assert_close(recorded[5].latitude, 46.0);
    assert_close(recorded[5].longitude, -71.0);
    assert_eq!(recorded[5].altitude, 15);
    assert_close(recorded[5].speed.unwrap(), 0.0);
    assert_close(recorded[5].heading.unwrap(), 360.0);
    assert_close(recorded[5].horizontal_accuracy.unwrap(), 5.0);
    assert_eq!(recorded[5].source, Some(LocationSource::Gnss));

    server.process(11, &[0xFF, 0x00, 0x01]).unwrap();
    server.process(11, &follow_delta).unwrap();
    assert_eq!(fixes.lock().unwrap().len(), 6);

    // Invalid base coordinates, telemetry, or motion deltas must be ignored and invalidate the delta base.
    server.process(11, &v2_base).unwrap();
    assert_eq!(fixes.lock().unwrap().len(), 7);

    let bad_heading_delta = encode_vec(&LocationPdu::Location2dDelta(Location2dDeltaPdu {
        latitude_delta: FourByteFloat::new(0.0).unwrap(),
        longitude_delta: FourByteFloat::new(0.0).unwrap(),
        speed_delta: Some(FourByteFloat::new(0.0).unwrap()),
        heading_delta: Some(FourByteFloat::new(-1.0).unwrap()),
    }))
    .unwrap();
    server.process(11, &bad_heading_delta).unwrap();
    server.process(11, &v2_delta_2d).unwrap();
    assert_eq!(fixes.lock().unwrap().len(), 7);

    for invalid_base in [
        BaseLocation3dPdu {
            latitude: FourByteFloat::new(95.0).unwrap(),
            longitude: FourByteFloat::new(-71.3).unwrap(),
            altitude: FourByteSignedInteger::new(10).unwrap(),
            speed: None,
            heading: None,
            horizontal_accuracy: None,
            source: None,
        },
        BaseLocation3dPdu {
            latitude: FourByteFloat::new(46.3).unwrap(),
            longitude: FourByteFloat::new(-71.3).unwrap(),
            altitude: FourByteSignedInteger::new(10).unwrap(),
            speed: Some(FourByteFloat::new(-0.1).unwrap()),
            heading: Some(FourByteFloat::new(90.0).unwrap()),
            horizontal_accuracy: Some(FourByteFloat::new(5.0).unwrap()),
            source: Some(LocationSource::Gnss),
        },
        BaseLocation3dPdu {
            latitude: FourByteFloat::new(46.3).unwrap(),
            longitude: FourByteFloat::new(-71.3).unwrap(),
            altitude: FourByteSignedInteger::new(10).unwrap(),
            speed: Some(FourByteFloat::new(0.3).unwrap()),
            heading: Some(FourByteFloat::new(360.1).unwrap()),
            horizontal_accuracy: Some(FourByteFloat::new(5.0).unwrap()),
            source: Some(LocationSource::Gnss),
        },
    ] {
        let encoded = encode_vec(&LocationPdu::BaseLocation3d(invalid_base)).unwrap();
        server.process(11, &encoded).unwrap();
    }
    assert_eq!(fixes.lock().unwrap().len(), 7);

    server.start(12).expect("restart server");
    let v2_ready = encode_vec(&LocationPdu::ClientReady(ReadyPdu {
        protocol_version: ProtocolVersion::V2,
        flags: Some(0),
    }))
    .unwrap();
    server.process(12, &v2_ready).expect("server negotiates V2");
    assert_eq!(*ready.lock().unwrap(), [ProtocolVersion::V1, ProtocolVersion::V2]);

    server.close(12);
    assert_eq!(*closed.lock().unwrap(), 1);
}

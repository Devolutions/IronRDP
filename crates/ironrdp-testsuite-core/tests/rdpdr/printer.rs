//! Multi-printer announcements and MS-RDPEPC printer cache data.

use std::sync::Arc;

use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    DeviceAnnounceHeader, NtStatus, RDPDR_PRINTER_ANNOUNCE_FLAG_ASCII, RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER,
    RDPDR_PRINTER_ANNOUNCE_FLAG_NETWORKPRINTER, ServerDeviceAnnounceResponse,
};
use ironrdp_rdpdr::pdu::epc::{
    AddPrinterCacheData, DeletePrinterCacheData, PrinterCacheData, PrinterUsingXps, RenamePrinterCacheData,
    UpdatePrinterCacheData,
};
use ironrdp_rdpdr::{
    InMemoryRdpdrPrinterCacheStore, NoopRdpdrBackend, Rdpdr, RdpdrPort, RdpdrPortKind, RdpdrPrinter,
    RdpdrPrinterCacheEntry, RdpdrPrinterCacheStore,
};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::{
    announce_client_id, encoded_server_client_id_confirm, negotiate_printer_capability, read_u32, read_u32_as_usize,
    utf16le_to_string,
};

#[derive(Debug, PartialEq, Eq)]
struct AnnouncedPrinter {
    device_id: u32,
    dos_name: String,
    flags: u32,
    driver_name: String,
    printer_name: String,
    cached_fields: Vec<u8>,
}

fn announced_printers(message: &SvcMessage) -> Vec<AnnouncedPrinter> {
    let encoded = message.encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x41, 0x44]); // RDPDR + DEVICELIST_ANNOUNCE

    let count = read_u32_as_usize(&encoded[4..]);
    let mut offset = 8;
    let mut printers = Vec::with_capacity(count);
    for _ in 0..count {
        let device_id = read_u32(&encoded[offset + 4..]);
        let dos_name = String::from_utf8(encoded[offset + 8..offset + 16].to_vec())
            .unwrap()
            .trim_end_matches('\0')
            .to_owned();
        let device_data_length = read_u32_as_usize(&encoded[offset + 16..]);
        let body = &encoded[offset + 20..offset + 20 + device_data_length];
        offset += 20 + device_data_length;

        let pnp_name_len = read_u32_as_usize(&body[8..]);
        let driver_name_len = read_u32_as_usize(&body[12..]);
        let print_name_len = read_u32_as_usize(&body[16..]);
        let cached_fields_len = read_u32_as_usize(&body[20..]);
        let driver_start = 24 + pnp_name_len;
        let print_start = driver_start + driver_name_len;
        let cached_start = print_start + print_name_len;
        assert_eq!(cached_start + cached_fields_len, body.len());

        printers.push(AnnouncedPrinter {
            device_id,
            dos_name,
            flags: read_u32(body),
            driver_name: utf16le_to_string(&body[driver_start..print_start]),
            printer_name: utf16le_to_string(&body[print_start..cached_start]),
            cached_fields: body[cached_start..].to_vec(),
        });
    }
    assert_eq!(offset, encoded.len());
    printers
}

/// Runs the initialization sequence and returns the post-logon printer announcement.
fn announce_after_logon(rdpdr: &mut Rdpdr) -> SvcMessage {
    let client_id = announce_client_id(rdpdr, 12);
    negotiate_printer_capability(rdpdr);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let mut messages = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(messages.len(), 1);
    messages.remove(0)
}

fn device_reply(device_id: u32, result_code: NtStatus) -> Vec<u8> {
    encode_vec(&RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
        device_id,
        result_code,
    }))
    .unwrap()
}

#[test]
fn several_printers_announce_with_distinct_ports_and_a_single_default() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_redirected_printer(
            RdpdrPrinter::new(10, "Local".to_owned(), "Local Driver".to_owned()).with_network(false),
        )
        .with_redirected_printer(
            RdpdrPrinter::new(11, r"\\print\Office".to_owned(), "Office Driver".to_owned()).with_default(true),
        )
        .with_redirected_printer(
            RdpdrPrinter::new(12, "Second Default".to_owned(), "Local Driver".to_owned())
                .with_network(false)
                .with_default(true)
                .with_cached_config_data(vec![0xAA, 0xBB]),
        );

    let announcement = announce_after_logon(&mut rdpdr);

    assert_eq!(
        announced_printers(&announcement),
        vec![
            AnnouncedPrinter {
                device_id: 10,
                dos_name: "PRN1".to_owned(),
                flags: 0,
                driver_name: "Local Driver".to_owned(),
                printer_name: "Local".to_owned(),
                cached_fields: Vec::new(),
            },
            AnnouncedPrinter {
                device_id: 11,
                dos_name: "PRN2".to_owned(),
                flags: RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER | RDPDR_PRINTER_ANNOUNCE_FLAG_NETWORKPRINTER,
                driver_name: "Office Driver".to_owned(),
                printer_name: r"\\print\Office".to_owned(),
                cached_fields: Vec::new(),
            },
            // MS-RDPEPC 2.2.2.1 allows a single default printer.
            AnnouncedPrinter {
                device_id: 12,
                dos_name: "PRN3".to_owned(),
                flags: 0,
                driver_name: "Local Driver".to_owned(),
                printer_name: "Second Default".to_owned(),
                cached_fields: vec![0xAA, 0xBB],
            },
        ]
    );
}

#[test]
fn legacy_printer_helpers_get_distinct_ports_and_one_capability() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_printer(41, "First".to_owned())
        .with_printer(42, "Second".to_owned());
    let client_id = announce_client_id(&mut rdpdr, 12);
    let server_capability = RdpdrPdu::CoreCapability(ironrdp_rdpdr::pdu::efs::CoreCapability {
        capabilities: vec![
            ironrdp_rdpdr::pdu::efs::CapabilityMessage::new_general(0),
            ironrdp_rdpdr::pdu::efs::CapabilityMessage::new_printer(),
        ],
        kind: ironrdp_rdpdr::pdu::efs::CoreCapabilityKind::ServerCoreCapabilityRequest,
    });
    let response = rdpdr.process(&encode_vec(&server_capability).unwrap()).unwrap();
    let encoded = response[0].encode_unframed_pdu().unwrap();
    // General and printer capability sets only.
    assert_eq!(u16::from_le_bytes([encoded[4], encoded[5]]), 2);

    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let announcement = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    let dos_names = announced_printers(&announcement[0])
        .into_iter()
        .map(|printer| printer.dos_name)
        .collect::<Vec<_>>();
    assert_eq!(dos_names, ["PRN1", "PRN2"]);
}

#[test]
fn printer_cache_data_pdus_round_trip() {
    let pdus = [
        PrinterCacheData::Add(AddPrinterCacheData {
            port_dos_name: "PRN2".to_owned(),
            pnp_name: String::new(),
            driver_name: "Server Driver".to_owned(),
            printer_name: r"\\print\Office".to_owned(),
            config_data: vec![1, 2, 3, 4],
        }),
        PrinterCacheData::Update(UpdatePrinterCacheData {
            printer_name: "Office".to_owned(),
            config_data: vec![5; 300],
        }),
        PrinterCacheData::Delete(DeletePrinterCacheData {
            printer_name: "Office".to_owned(),
        }),
        PrinterCacheData::Rename(RenamePrinterCacheData {
            old_printer_name: "Office".to_owned(),
            new_printer_name: "Office (renamed)".to_owned(),
        }),
    ];

    for pdu in pdus {
        let encoded = encode_vec(&pdu).unwrap();
        // RDPDR_CTYP_PRN + PAKID_PRN_CACHE_DATA
        assert_eq!(&encoded[..4], &[0x52, 0x50, 0x43, 0x50]);
        assert_eq!(decode::<PrinterCacheData>(&encoded).unwrap(), pdu);
    }

    let update = encode_vec(&PrinterCacheData::Update(UpdatePrinterCacheData {
        printer_name: "Office".to_owned(),
        config_data: vec![7, 8],
    }))
    .unwrap();
    // EventId, PrinterNameLen ("Office" + NUL in UTF-16), ConfigDataLen.
    assert_eq!(read_u32(&update[4..]), 2);
    assert_eq!(read_u32(&update[8..]), 14);
    assert_eq!(read_u32(&update[12..]), 2);
    assert_eq!(&update[update.len() - 2..], &[7, 8]);

    let xps = PrinterUsingXps {
        printer_id: 42,
        flags: 0xDEAD_BEEF,
    };
    assert_eq!(decode::<PrinterUsingXps>(&encode_vec(&xps).unwrap()).unwrap(), xps);
}

#[test]
fn printer_cache_data_rejects_truncated_and_unknown_events() {
    let mut truncated = encode_vec(&PrinterCacheData::Delete(DeletePrinterCacheData {
        printer_name: "Office".to_owned(),
    }))
    .unwrap();
    truncated.pop();
    assert!(decode::<PrinterCacheData>(&truncated).is_err());

    let mut unknown = encode_vec(&PrinterCacheData::Delete(DeletePrinterCacheData {
        printer_name: "Office".to_owned(),
    }))
    .unwrap();
    unknown[4..8].copy_from_slice(&9u32.to_le_bytes());
    assert!(decode::<PrinterCacheData>(&unknown).is_err());
}

#[test]
fn printer_cache_data_persists_and_replays_as_cached_fields() {
    let store = Arc::new(InMemoryRdpdrPrinterCacheStore::new());
    let shared_store: Arc<dyn RdpdrPrinterCacheStore> = Arc::<InMemoryRdpdrPrinterCacheStore>::clone(&store);
    let office = || RdpdrPrinter::new(42, "Office".to_owned(), "Local Driver".to_owned()).with_network(false);

    // First connection: the server adds and then updates the printer configuration.
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_printer_cache_store(Arc::clone(&shared_store))
        .with_redirected_printer(office());
    let first = announced_printers(&announce_after_logon(&mut rdpdr));
    assert!(first[0].cached_fields.is_empty());
    assert!(rdpdr.process(&device_reply(42, NtStatus::SUCCESS)).unwrap().is_empty());

    for pdu in [
        PrinterCacheData::Add(AddPrinterCacheData {
            port_dos_name: "PRN1".to_owned(),
            pnp_name: String::new(),
            driver_name: "Server Driver".to_owned(),
            printer_name: "Office".to_owned(),
            config_data: vec![1, 2, 3],
        }),
        PrinterCacheData::Update(UpdatePrinterCacheData {
            printer_name: "Office".to_owned(),
            config_data: vec![4, 5],
        }),
        // Not announced by this client: ignored.
        PrinterCacheData::Update(UpdatePrinterCacheData {
            printer_name: "Elsewhere".to_owned(),
            config_data: vec![6],
        }),
    ] {
        assert!(rdpdr.process(&encode_vec(&pdu).unwrap()).unwrap().is_empty());
    }
    assert_eq!(
        store.load("Office").unwrap(),
        Some(RdpdrPrinterCacheEntry {
            driver_name: Some("Server Driver".to_owned()),
            // After the add event, the server queue uses the manually installed driver.
            config_driver_name: Some("Server Driver".to_owned()),
            config_data: vec![4, 5],
        })
    );
    assert_eq!(store.load("Elsewhere").unwrap(), None);

    // The same channel re-reads the store when the server starts a new sequence.
    let reset = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(reset[0].driver_name, "Server Driver");
    assert_eq!(reset[0].cached_fields, [4, 5]);

    // Configuration produced for the server driver is replayed with it.
    assert!(rdpdr.process(&device_reply(42, NtStatus::SUCCESS)).unwrap().is_empty());
    let update = PrinterCacheData::Update(UpdatePrinterCacheData {
        printer_name: "Office".to_owned(),
        config_data: vec![6, 7],
    });
    rdpdr.process(&encode_vec(&update).unwrap()).unwrap();

    // Next connection: the cached driver and configuration are announced. The store can be
    // attached after the printer.
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_redirected_printer(office())
        .with_printer_cache_store(Arc::clone(&shared_store));
    let replayed = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(replayed[0].driver_name, "Server Driver");
    assert_eq!(replayed[0].cached_fields, [6, 7]);

    // A user-chosen driver wins over the cached driver, and the server driver's configuration
    // is not sent with it.
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_printer_cache_store(Arc::clone(&shared_store))
        .with_redirected_printer(office().with_explicit_driver(true));
    let explicit = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(explicit[0].driver_name, "Local Driver");
    assert!(explicit[0].cached_fields.is_empty());

    // Configuration preset for the local driver is dropped when the cached driver replaces it.
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_printer_cache_store(Arc::clone(&shared_store))
        .with_redirected_printer(office().with_cached_config_data(vec![0xEE]));
    store
        .save(
            "Office",
            &RdpdrPrinterCacheEntry {
                driver_name: Some("Server Driver".to_owned()),
                config_driver_name: Some("Other Driver".to_owned()),
                config_data: vec![8],
            },
        )
        .unwrap();
    let mismatched = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(mismatched[0].driver_name, "Server Driver");
    assert!(mismatched[0].cached_fields.is_empty());
}

#[test]
fn printer_cache_events_follow_a_rename_without_moving_the_entry() {
    let store = Arc::new(InMemoryRdpdrPrinterCacheStore::new());
    let shared_store: Arc<dyn RdpdrPrinterCacheStore> = Arc::<InMemoryRdpdrPrinterCacheStore>::clone(&store);
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_printer_cache_store(shared_store)
        .with_redirected_printer(RdpdrPrinter::new(42, "Office".to_owned(), "Driver".to_owned()));
    announce_after_logon(&mut rdpdr);
    assert!(rdpdr.process(&device_reply(42, NtStatus::SUCCESS)).unwrap().is_empty());

    for pdu in [
        PrinterCacheData::Rename(RenamePrinterCacheData {
            old_printer_name: "Office".to_owned(),
            new_printer_name: "Office 2".to_owned(),
        }),
        // Later messages use the new name.
        PrinterCacheData::Update(UpdatePrinterCacheData {
            printer_name: "Office 2".to_owned(),
            config_data: vec![1, 2],
        }),
        // The old name no longer refers to the printer.
        PrinterCacheData::Update(UpdatePrinterCacheData {
            printer_name: "Office".to_owned(),
            config_data: vec![3],
        }),
    ] {
        rdpdr.process(&encode_vec(&pdu).unwrap()).unwrap();
    }
    // The entry stays under the announced name, which the next connection announces again.
    assert_eq!(
        store.load("Office").unwrap().map(|entry| entry.config_data),
        Some(vec![1, 2])
    );
    assert_eq!(store.load("Office 2").unwrap(), None);

    let delete = PrinterCacheData::Delete(DeletePrinterCacheData {
        printer_name: "Office 2".to_owned(),
    });
    rdpdr.process(&encode_vec(&delete).unwrap()).unwrap();
    assert_eq!(store.load("Office").unwrap(), None);
}

#[test]
fn legacy_and_redirected_printers_share_a_single_default() {
    for legacy_first in [true, false] {
        let rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned());
        let redirected = RdpdrPrinter::new(11, "Redirected".to_owned(), "Driver".to_owned()).with_default(true);
        let mut rdpdr = if legacy_first {
            rdpdr
                .with_printer(10, "Legacy".to_owned())
                .with_redirected_printer(redirected)
        } else {
            rdpdr
                .with_redirected_printer(redirected)
                .with_printer(10, "Legacy".to_owned())
        };

        let defaults = announced_printers(&announce_after_logon(&mut rdpdr))
            .into_iter()
            .filter(|printer| printer.flags & RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER != 0)
            .map(|printer| printer.device_id)
            .collect::<Vec<_>>();
        assert_eq!(defaults, [if legacy_first { 10 } else { 11 }]);
    }
}

#[test]
fn printers_beyond_the_last_prn_port_are_skipped() {
    let mut devices = ironrdp_rdpdr::pdu::efs::Devices::new();
    for device_id in 1..=9999 {
        assert!(devices.add_printer_announce(device_id, 0, "Driver", "Printer", &[]));
    }

    assert!(!devices.add_printer_announce(10_000, 0, "Driver", "Printer", &[]));
    assert!(!devices.add_printer(10_001, "Printer".to_owned()));
    assert!(devices.device(10_000).is_none());
    assert!(devices.device(10_001).is_none());
}

#[test]
fn printer_announce_clears_the_ascii_flag() {
    let device =
        DeviceAnnounceHeader::new_printer_announce(42, 1, RDPDR_PRINTER_ANNOUNCE_FLAG_ASCII, "Driver", "Printer", &[]);

    assert_eq!(read_u32(device.device_data()), 0);
}

#[test]
fn printer_cache_data_without_a_store_is_ignored() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_redirected_printer(RdpdrPrinter::new(42, "Office".to_owned(), "Driver".to_owned()));
    let update = PrinterCacheData::Update(UpdatePrinterCacheData {
        printer_name: "Office".to_owned(),
        config_data: vec![1],
    });

    assert!(rdpdr.process(&encode_vec(&update).unwrap()).unwrap().is_empty());
}

#[test]
fn xps_mode_request_is_ignored_for_printers_without_xps_support() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_redirected_printer(RdpdrPrinter::new(42, "Office".to_owned(), "Driver".to_owned()));
    let xps = PrinterUsingXps {
        printer_id: 42,
        flags: 0,
    };

    assert!(rdpdr.process(&encode_vec(&xps).unwrap()).unwrap().is_empty());
}

#[test]
fn rejected_printer_is_announced_once_more_with_the_fallback_driver() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_redirected_printer(
        RdpdrPrinter::new(42, "Office".to_owned(), "Missing Driver".to_owned())
            .with_fallback_driver_name(Some("MS Publisher Imagesetter".to_owned()))
            .with_cached_config_data(vec![1, 2, 3]),
    );
    let first = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(first[0].driver_name, "Missing Driver");
    assert_eq!(first[0].cached_fields, [1, 2, 3]);

    let retry = rdpdr.process(&device_reply(42, NtStatus::UNSUCCESSFUL)).unwrap();
    assert_eq!(retry.len(), 2);
    // MS-RDPEFS 3.2.5.2.2: the rejected DeviceId is removed before it is reused.
    let remove = retry[0].encode_unframed_pdu().unwrap();
    assert_eq!(&remove[..4], &[0x72, 0x44, 0x4D, 0x44]); // RDPDR + DEVICELIST_REMOVE
    assert_eq!(read_u32(&remove[4..]), 1);
    assert_eq!(read_u32(&remove[8..]), 42);
    let retried = announced_printers(&retry[1]);
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].device_id, 42);
    assert_eq!(retried[0].dos_name, "PRN1");
    assert_eq!(retried[0].driver_name, "MS Publisher Imagesetter");
    assert_eq!(retried[0].flags, first[0].flags);
    // Cached configuration is driver-specific and belongs to the rejected driver.
    assert!(retried[0].cached_fields.is_empty());

    // A second rejection is final.
    assert!(
        rdpdr
            .process(&device_reply(42, NtStatus::UNSUCCESSFUL))
            .unwrap()
            .is_empty()
    );

    // A new initialization sequence starts again from the configured driver.
    let again = announced_printers(&announce_after_logon(&mut rdpdr));
    assert_eq!(again[0].driver_name, "Missing Driver");
}

#[test]
fn printers_join_parallel_ports_after_logon_while_serial_ports_go_first() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_redirected_printer(RdpdrPrinter::new(0xFFFF_FFFE, "Office".to_owned(), "Driver".to_owned()))
        .with_port(RdpdrPort::new(0x4000_0000, RdpdrPortKind::Serial, "COM3".to_owned()).unwrap())
        .with_port(RdpdrPort::new(0x4000_0001, RdpdrPortKind::Parallel, "LPT1".to_owned()).unwrap());

    let client_id = announce_client_id(&mut rdpdr, 12);
    let server_capability = RdpdrPdu::CoreCapability(ironrdp_rdpdr::pdu::efs::CoreCapability {
        capabilities: vec![
            ironrdp_rdpdr::pdu::efs::CapabilityMessage::new_general(0),
            ironrdp_rdpdr::pdu::efs::CapabilityMessage::new_printer(),
            ironrdp_rdpdr::pdu::efs::CapabilityMessage::new_port(),
        ],
        kind: ironrdp_rdpdr::pdu::efs::CoreCapabilityKind::ServerCoreCapabilityRequest,
    });
    rdpdr.process(&encode_vec(&server_capability).unwrap()).unwrap();

    // The serial port is announced on its own before logon.
    let confirm = rdpdr.process(&encoded_server_client_id_confirm(client_id)).unwrap();
    assert_eq!(confirm.len(), 1);
    let serial = confirm[0].encode_unframed_pdu().unwrap();
    assert_eq!(read_u32(&serial[4..]), 1);
    assert_eq!(read_u32(&serial[12..]), 0x4000_0000);
    assert_eq!(&serial[16..24], b"COM3    ");

    // The printer and the parallel port share the post-logon announcement, each with its own name.
    let logon = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(logon.len(), 1);
    let devices = super::announced_devices(&logon[0]);
    assert_eq!(
        devices,
        vec![
            (0xFFFF_FFFE, ironrdp_rdpdr::pdu::efs::DeviceType::Print),
            (0x4000_0001, ironrdp_rdpdr::pdu::efs::DeviceType::Parallel),
        ]
    );
    let encoded = logon[0].encode_unframed_pdu().unwrap();
    let printer_data_len = read_u32_as_usize(&encoded[24..]);
    assert_eq!(&encoded[16..24], b"PRN1    ");
    let parallel = 28 + printer_data_len;
    assert_eq!(&encoded[parallel + 8..parallel + 16], b"LPT1    ");
}

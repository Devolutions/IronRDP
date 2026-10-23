use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::PduResult;
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_svc::SvcProcessor as _;

use super::*;

#[derive(Default)]
struct RecordedServerDvc {
    received: Vec<Vec<u8>>,
}

impl_as_any!(RecordedServerDvc);

impl DvcProcessor for RecordedServerDvc {
    fn channel_name(&self) -> &str {
        "recorded"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.received.push(payload.to_vec());
        Ok(Vec::new())
    }
}

impl DvcServerProcessor for RecordedServerDvc {}

fn process_client_pdu(server: &mut DrdynvcServer, pdu: DrdynvcClientPdu) {
    let encoded = encode_vec(&pdu).expect("client PDU should encode");
    server.process(&encoded).expect("client PDU should be processed");
}

#[test]
fn server_decompresses_client_data() {
    let mut server = DrdynvcServer::new().with_dynamic_channel(RecordedServerDvc::default());
    server.start().expect("server should start");
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V3)),
    );
    let channel_id = server
        .get_channel_id_by_type::<RecordedServerDvc>()
        .expect("channel should be requested");
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Create(CreateResponsePdu::new(channel_id, CreationStatus::OK)),
    );

    // The MS-RDPEDYC 4.3.3 and 4.3.4 data blocks, sent by the client as two messages: the
    // second matches against the history left by the first.
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataFirstCompressed(DataFirstCompressedPdu::new(
            channel_id,
            1595,
            vec![0xe0, 0x26, 0x38, 0xc4, 0x3f, 0xf4, 0x74, 0x01],
        ))),
    );
    process_client_pdu(
        &mut server,
        DrdynvcClientPdu::Data(DrdynvcDataPdu::DataCompressed(DataCompressedPdu::new(
            channel_id,
            vec![0xe0, 0x26, 0x88, 0x7f, 0xe8, 0xf4, 0x02],
        ))),
    );

    let channel = server
        .dvc_by_id::<RecordedServerDvc>(channel_id)
        .expect("channel should be open");
    assert_eq!(channel.processor().received, vec![vec![b'q'; 1595], vec![b'q'; 1597]]);
}

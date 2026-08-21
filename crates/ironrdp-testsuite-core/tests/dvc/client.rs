use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_dvc::ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_dvc::pdu::{DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_dvc::{DrdynvcClient, DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_svc::SvcProcessor as _;

#[derive(Default)]
struct RecordedDvc {
    started_with: Option<u32>,
    received: Vec<Vec<u8>>,
}

impl_as_any!(RecordedDvc);

impl DvcProcessor for RecordedDvc {
    fn channel_name(&self) -> &str {
        "recorded"
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.started_with = Some(channel_id);
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.received.push(payload.to_vec());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for RecordedDvc {}

struct FailingDvc;

impl_as_any!(FailingDvc);

impl DvcProcessor for FailingDvc {
    fn channel_name(&self) -> &str {
        "failing"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Err(pdu_other_err!("failed to restore dynamic channel"))
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for FailingDvc {}

#[test]
fn established_dynamic_channel_routes_recorded_data_without_negotiation() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("recorded channel should attach");

    let payload = encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
        7,
        b"recorded data".to_vec(),
    ))))
    .expect("recorded DVC data should encode");
    assert!(
        client
            .process(&payload)
            .expect("recorded DVC data should route")
            .is_empty()
    );

    let channel = client
        .get_dvc_by_channel_id::<RecordedDvc>(7)
        .expect("recorded channel should remain attached");
    assert_eq!(channel.processor().started_with, Some(7));
    assert_eq!(channel.processor().received, vec![b"recorded data".to_vec()]);
}

#[test]
fn established_dynamic_channel_rejects_repeated_ids() {
    let mut client = DrdynvcClient::new();
    client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect("first attachment should succeed");

    let error = client
        .attach_established_dynamic_channel(7, RecordedDvc::default())
        .expect_err("duplicate channel ID should fail");

    assert!(error.to_string().contains("dynamic channel ID is already attached"));
}

#[test]
fn failed_established_dynamic_channel_attachment_is_not_registered() {
    let mut client = DrdynvcClient::new();

    let error = client
        .attach_established_dynamic_channel(7, FailingDvc)
        .expect_err("failing channel should not attach");

    assert!(error.to_string().contains("failed to restore dynamic channel"));
    assert!(client.get_dvc_by_channel_id::<FailingDvc>(7).is_none());
    assert!(
        client
            .attach_established_dynamic_channel(7, RecordedDvc::default())
            .is_ok()
    );
}

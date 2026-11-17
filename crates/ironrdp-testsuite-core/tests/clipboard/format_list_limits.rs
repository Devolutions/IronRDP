//! Bounds on clipboard formats dispatched to the backend.

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FileContentsRequest, FileContentsResponse, FormatDataRequest,
    FormatDataResponse, FormatList, FormatListResponse, LockDataId,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrServer};
use ironrdp_core::{AsAny, EncodeResult};
use ironrdp_svc::{SvcMessage, SvcProcessor};
use ironrdp_testsuite_core::clipboard::{TestBackend, init_ready_client_with_backend};
use rstest::rstest;

fn named(id: u32, name: &str) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::new(id)).with_name(ClipboardFormatName::new(name.to_owned()))
}

fn unnamed(id: ClipboardFormatId) -> ClipboardFormat {
    ClipboardFormat::new(id)
}

/// Encodes formats into a Format List, as [`FormatList::new_unicode`] and [`FormatList::new_ascii`] do.
type Encoder = fn(&[ClipboardFormat], bool) -> EncodeResult<FormatList<'static>>;

#[derive(Debug, Default)]
struct RemoteCopies(Vec<Vec<ClipboardFormat>>);

impl CliprdrBackend for RemoteCopies {
    fn temporary_directory(&self) -> &str {
        "/tmp"
    }
    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }
    fn on_ready(&mut self) {}
    fn on_request_format_list(&mut self) {}
    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}
    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.0.push(available_formats.to_vec());
    }
    fn on_format_data_request(&mut self, _: FormatDataRequest) {}
    fn on_format_data_response(&mut self, _: FormatDataResponse<'_>) {}
    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

impl AsAny for RemoteCopies {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// Hands `to` each of `messages`; returns its answers.
fn deliver(to: &mut impl SvcProcessor, messages: impl Into<Vec<SvcMessage>>) -> Vec<SvcMessage> {
    messages
        .into()
        .into_iter()
        .flat_map(|message| to.process(&message.encode_unframed_pdu().unwrap()).unwrap())
        .collect()
}

/// The Format List Responses that `messages` carry.
fn format_list_responses(messages: &[SvcMessage]) -> Vec<FormatListResponse> {
    messages
        .iter()
        .map(|message| {
            let bytes = message.encode_unframed_pdu().unwrap();
            match ironrdp_core::decode::<ClipboardPdu<'_>>(&bytes).unwrap() {
                ClipboardPdu::FormatListResponse(response) => response,
                other => panic!("expected a Format List Response, got {other:?}"),
            }
        })
        .collect()
}

// oracle: regression #2091 - the backend sees a bounded prefix of a duplicate bitmap offer.
#[test]
fn oversized_first_copy_completes_initialization_and_the_next_copy_replaces_it() {
    let mut server = CliprdrServer::new(Box::<RemoteCopies>::default());
    let mut client = CliprdrClient::new(Box::new(TestBackend));
    deliver(&mut client, server.start().unwrap());

    let mut formats = vec![unnamed(ClipboardFormatId::CF_DIB); 1024];
    formats.push(unnamed(ClipboardFormatId::CF_UNICODETEXT));
    let answer = deliver(&mut server, client.initiate_copy(&formats).unwrap());
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    deliver(&mut client, answer);
    assert!(client.is_ready());
    assert!(server.is_ready());
    let next = &formats[1024..];
    let answer = deliver(&mut server, client.initiate_copy(next).unwrap());
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    assert_eq!(
        server.downcast_backend::<RemoteCopies>().unwrap().0,
        [&formats[..1024], next]
    );
}

// oracle: mutant decoding an overlong name, dropping the preceding format, or continuing into the suffix.
#[rstest]
#[case::unicode(FormatList::new_unicode, "\u{10400}".repeat(512))]
#[case::ansi(FormatList::new_ascii, "\u{e9}".repeat(512))]
fn stops_before_an_overlong_name(#[case] encode: Encoder, #[case] at_limit: String) {
    let formats = [
        named(0xC001, &at_limit),
        named(0xC002, &format!("{at_limit}x")),
        named(0xC003, "suffix"),
    ];
    let list = encode(&formats, true).unwrap();
    assert_eq!(list.get_formats(true).unwrap(), formats);
    let mut client = init_ready_client_with_backend(Box::<RemoteCopies>::default());
    let bytes = ironrdp_core::encode_vec(&ClipboardPdu::FormatList(list)).unwrap();
    assert_eq!(
        format_list_responses(&client.process(&bytes).unwrap()),
        [FormatListResponse::Ok]
    );
    assert_eq!(client.downcast_backend::<RemoteCopies>().unwrap().0, [&formats[..1]]);
}

// oracle: mutant allocating and dispatching the entire fixed-width format list.
#[test]
fn bounds_short_format_lists() {
    let mut client = CliprdrClient::new(Box::<RemoteCopies>::default());
    let caps = Capabilities::new(ClipboardProtocolVersion::V2, ClipboardGeneralCapabilityFlags::empty());
    client
        .process(&ironrdp_core::encode_vec(&ClipboardPdu::Capabilities(caps)).unwrap())
        .unwrap();
    let formats = vec![unnamed(ClipboardFormatId::CF_DIB); 1025];
    let list = FormatList::new_unicode(&formats, false).unwrap();
    assert_eq!(list.get_formats(false).unwrap(), formats);
    let bytes = ironrdp_core::encode_vec(&ClipboardPdu::FormatList(list)).unwrap();
    assert_eq!(
        format_list_responses(&client.process(&bytes).unwrap()),
        [FormatListResponse::Ok]
    );
    assert_eq!(client.downcast_backend::<RemoteCopies>().unwrap().0, [&formats[..1024]]);
}

// oracle: mutant treating invalid UTF-16 within the accepted prefix as the end of the offer.
#[test]
fn reports_decode_errors_within_the_prefix() {
    let mut client = init_ready_client_with_backend(Box::<RemoteCopies>::default());
    // CB_FORMAT_LIST with one UTF-16 name containing an unpaired high surrogate.
    let bytes = [2, 0, 0, 0, 8, 0, 0, 0, 1, 0xc0, 0, 0, 0, 0xd8, 0, 0];
    assert!(client.process(&bytes).is_err());
    assert!(client.downcast_backend::<RemoteCopies>().unwrap().0.is_empty());
}

// oracle: mutant accepting an unterminated name of any length, or retaining a stale clipboard offer.
#[test]
fn an_overlong_unterminated_first_name_replaces_the_previous_offer_with_an_empty_prefix() {
    let mut client = init_ready_client_with_backend(Box::<RemoteCopies>::default());
    let normal = [unnamed(ClipboardFormatId::CF_UNICODETEXT)];
    let list = FormatList::new_unicode(&normal, true).unwrap();
    client
        .process(&ironrdp_core::encode_vec(&ClipboardPdu::FormatList(list)).unwrap())
        .unwrap();

    // CB_FORMAT_LIST with an ANSI name that reaches the end of the PDU.
    let mut bytes = vec![2, 0, 4, 0];
    bytes.extend_from_slice(&1029u32.to_le_bytes());
    bytes.extend_from_slice(&0xC001u32.to_le_bytes());
    bytes.extend(core::iter::repeat_n(b'x', 1025));
    assert_eq!(
        format_list_responses(&client.process(&bytes).unwrap()),
        [FormatListResponse::Ok]
    );
    assert_eq!(client.downcast_backend::<RemoteCopies>().unwrap().0, [&normal[..], &[]]);
}

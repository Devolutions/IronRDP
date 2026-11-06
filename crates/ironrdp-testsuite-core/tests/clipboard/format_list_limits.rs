//! What a Format List PDU may carry: at most [`MAX_FORMAT_COUNT`] formats, with names of at most
//! [`MAX_FORMAT_NAME_LENGTH`] characters.
//! A list over these limits is answered with CB_RESPONSE_FAIL, and the channel goes on.

use std::sync::{Arc, Mutex};

use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags, ClipboardPdu,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, FormatList, FormatListResponse,
    LockDataId, MAX_FORMAT_COUNT, MAX_FORMAT_NAME_LENGTH, OwnedFormatDataResponse,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrServer};
use ironrdp_core::{AsAny, EncodeResult, IntoOwned as _};
use ironrdp_svc::{SvcMessage, SvcProcessor};
use rstest::rstest;

use ironrdp_testsuite_core::clipboard::{TestBackend, init_ready_client_with_backend};

fn named(id: u32, name: &str) -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::new(id)).with_name(ClipboardFormatName::new(name.to_owned()))
}

fn unnamed(id: ClipboardFormatId) -> ClipboardFormat {
    ClipboardFormat::new(id)
}

/// Encodes formats into a Format List, as [`FormatList::new_unicode`] and [`FormatList::new_ascii`] do.
type Encoder = fn(&[ClipboardFormat], bool) -> EncodeResult<FormatList<'static>>;

/// msgFlags of a Format List PDU with Unicode names.
const UNICODE_NAMES: u16 = 0;
/// msgFlags of a Format List PDU with ANSI names (CB_ASCII_NAMES).
const ASCII_NAMES: u16 = 0x0004;

/// A Format List PDU with `flags` as its msgFlags and `body` as its format entries, as the server sends it.
fn format_list_pdu(flags: u16, body: &[u8]) -> Vec<u8> {
    let mut pdu = vec![0x02, 0x00]; // msgType CB_FORMAT_LIST
    pdu.extend_from_slice(&flags.to_le_bytes());
    pdu.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    pdu.extend_from_slice(body);
    pdu
}

fn decode_long_names(pdu: &[u8]) -> Result<Vec<ClipboardFormat>, ironrdp_pdu::PduError> {
    match ironrdp_core::decode::<ClipboardPdu<'_>>(pdu).unwrap() {
        ClipboardPdu::FormatList(list) => list.get_formats(true),
        other => panic!("expected a Format List, got {other:?}"),
    }
}

// oracle: property round-trip - a list at the limits decodes to the formats it was encoded from.
#[rstest]
#[case::unicode(FormatList::new_unicode)]
#[case::ansi(FormatList::new_ascii)]
fn takes_a_format_list_at_the_limits(#[case] encode: Encoder) {
    let name = "n".repeat(MAX_FORMAT_NAME_LENGTH);
    let list = vec![named(0xC001, &name); MAX_FORMAT_COUNT];

    let formats = encode(&list, true).unwrap().get_formats(true).unwrap();

    assert_eq!(formats.len(), MAX_FORMAT_COUNT);
    assert!(formats.iter().all(|format| format == &list[0]));
}

// oracle: property round-trip - a list of short format names at the limit decodes to the formats it was encoded from.
#[test]
fn takes_a_short_format_list_at_the_limit() {
    // Short format names: 32 bytes each.
    let list = vec![unnamed(ClipboardFormatId::CF_UNICODETEXT); MAX_FORMAT_COUNT];

    let formats = FormatList::new_unicode(&list, false)
        .unwrap()
        .get_formats(false)
        .unwrap();

    assert_eq!(formats, list);
}

// oracle: property round-trip - an ANSI list that ends in a format with an empty name decodes to the formats it was
// encoded from.
#[test]
fn keeps_a_last_format_with_an_empty_ansi_name() {
    // An empty ANSI name is its one-byte terminator, so the last entry is 5 bytes: one short of a Unicode entry.
    let list = [named(0xC001, "PNG"), unnamed(ClipboardFormatId::CF_DIB)];

    assert_eq!(
        FormatList::new_ascii(&list, true).unwrap().get_formats(true).unwrap(),
        list
    );
}

// oracle: mutant an ANSI name's terminator counted as two bytes, or an off-by-one in MAX_FORMAT_COUNT
#[test]
fn counts_empty_ansi_names_at_the_limit() {
    let list = vec![unnamed(ClipboardFormatId::CF_DIB); MAX_FORMAT_COUNT];
    assert_eq!(
        FormatList::new_ascii(&list, true)
            .unwrap()
            .get_formats(true)
            .unwrap()
            .len(),
        MAX_FORMAT_COUNT
    );
    let too_many = vec![unnamed(ClipboardFormatId::CF_DIB); MAX_FORMAT_COUNT + 1];
    assert!(
        FormatList::new_ascii(&too_many, true)
            .unwrap()
            .get_formats(true)
            .is_err()
    );
}

// oracle: mutant an off-by-one in MAX_FORMAT_COUNT
#[rstest]
#[case::unicode_long_names(FormatList::new_unicode, true)]
#[case::ansi_long_names(FormatList::new_ascii, true)]
#[case::unicode_short_names(FormatList::new_unicode, false)]
fn refuses_more_formats_than_the_limit(#[case] encode: Encoder, #[case] long_names: bool) {
    let list = vec![named(0xC001, "PNG"); MAX_FORMAT_COUNT + 1];

    assert!(encode(&list, long_names).unwrap().get_formats(long_names).is_err());
}

// oracle: mutant an off-by-one in MAX_FORMAT_NAME_LENGTH
#[rstest]
#[case::unicode(FormatList::new_unicode)]
#[case::ansi(FormatList::new_ascii)]
fn refuses_a_format_name_longer_than_the_limit(#[case] encode: Encoder) {
    let list = [named(0xC001, &"n".repeat(MAX_FORMAT_NAME_LENGTH + 1))];

    assert!(encode(&list, true).unwrap().get_formats(true).is_err());
}

// oracle: mutant an off-by-one in MAX_FORMAT_NAME_LENGTH, or a name without a terminator taken at any length
#[rstest]
#[case::unicode(UNICODE_NAMES, b"n\0")]
#[case::ansi(ASCII_NAMES, b"n")]
fn refuses_an_unterminated_name_longer_than_the_limit(#[case] flags: u16, #[case] character: &[u8]) {
    // One format whose name runs past the limit without a terminator.
    let mut body = 0xC001u32.to_le_bytes().to_vec();
    body.extend(core::iter::repeat_n(character, MAX_FORMAT_NAME_LENGTH + 1).flatten());

    assert!(decode_long_names(&format_list_pdu(flags, &body)).is_err());
}

// oracle: mutant a name that ends with the PDU refused or cut short
#[rstest]
#[case::unicode(UNICODE_NAMES, b"P\0N\0G\0")]
#[case::ansi(ASCII_NAMES, b"PNG")]
fn takes_an_unterminated_name_within_the_limit(#[case] flags: u16, #[case] name: &[u8]) {
    // As before the limits: a name that ends with the PDU is read to its end.
    let mut body = 0xC001u32.to_le_bytes().to_vec();
    body.extend_from_slice(name);

    assert_eq!(
        decode_long_names(&format_list_pdu(flags, &body)).unwrap(),
        [named(0xC001, "PNG")]
    );
}

// oracle: mutant an off-by-one in MAX_FORMAT_NAME_LENGTH for a name without a terminator
#[rstest]
#[case::unicode(UNICODE_NAMES, b"n\0")]
#[case::ansi(ASCII_NAMES, b"n")]
fn takes_an_unterminated_name_of_the_limit_s_length(#[case] flags: u16, #[case] character: &[u8]) {
    let mut body = 0xC001u32.to_le_bytes().to_vec();
    body.extend(core::iter::repeat_n(character, MAX_FORMAT_NAME_LENGTH).flatten());

    assert_eq!(
        decode_long_names(&format_list_pdu(flags, &body)).unwrap(),
        [named(0xC001, &"n".repeat(MAX_FORMAT_NAME_LENGTH))]
    );
}

/// Records the formats the remote copies, the text of each paste, and the formats the remote asks for.
#[derive(Debug, Default)]
struct RemoteCopies {
    copies: Arc<Mutex<Vec<Vec<ClipboardFormat>>>>,
    pastes: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<ClipboardFormatId>>>,
}

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
        self.copies.lock().unwrap().push(available_formats.to_vec());
    }
    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.requests.lock().unwrap().push(request.format);
    }
    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if let Ok(text) = response.to_unicode_string() {
            self.pastes.lock().unwrap().push(text);
        }
    }
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

/// A client in the Ready state, and what its backend hears of the remote's copies.
fn ready_client() -> (CliprdrClient, Arc<Mutex<Vec<Vec<ClipboardFormat>>>>) {
    let backend = RemoteCopies::default();
    let copies = Arc::clone(&backend.copies);
    (init_ready_client_with_backend(Box::new(backend)), copies)
}

/// Hands the client the remote's Format List PDU; returns its answers.
fn process(cliprdr: &mut CliprdrClient, pdu: &[u8]) -> Vec<FormatListResponse> {
    format_list_responses(&cliprdr.process(pdu).unwrap())
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

/// The Format Data Responses that `messages` carry.
fn format_data_responses(messages: &[SvcMessage]) -> Vec<OwnedFormatDataResponse> {
    messages
        .iter()
        .map(|message| {
            let bytes = message.encode_unframed_pdu().unwrap();
            match ironrdp_core::decode::<ClipboardPdu<'_>>(&bytes).unwrap() {
                ClipboardPdu::FormatDataResponse(response) => response.into_owned(),
                other => panic!("expected a Format Data Response, got {other:?}"),
            }
        })
        .collect()
}

fn encode(list: &[ClipboardFormat]) -> Vec<u8> {
    ironrdp_core::encode_vec(&ClipboardPdu::FormatList(FormatList::new_unicode(list, true).unwrap())).unwrap()
}

// oracle: regression #2091 - a list of duplicate CF_DIB entries past the limit is refused, not handed to the backend.
#[test]
fn refuses_a_list_of_duplicate_formats_past_the_limit() {
    let (mut cliprdr, copies) = ready_client();

    // Repeated supported formats, with long format names negotiated, as the issue's server sent them.
    let list = vec![unnamed(ClipboardFormatId::CF_DIB); MAX_FORMAT_COUNT + 1];

    assert_eq!(process(&mut cliprdr, &encode(&list)), [FormatListResponse::Fail]);
    // None of them reaches the backend, whose selection scanned the whole list for each entry.
    assert!(copies.lock().unwrap().iter().all(Vec::is_empty));
}

#[test]
fn answers_a_format_list_over_the_limit_with_a_failure_and_goes_on() {
    let (mut cliprdr, copies) = ready_client();

    let responses = process(
        &mut cliprdr,
        &encode(&vec![unnamed(ClipboardFormatId::CF_UNICODETEXT); MAX_FORMAT_COUNT + 1]),
    );

    // oracle: spec MS-RDPECLIP §2.2.3.2 - a Format List that isn't processed is answered with CB_RESPONSE_FAIL.
    assert_eq!(responses, [FormatListResponse::Fail]);
    // The remote clipboard changed to nothing the client can take.
    // oracle: mutant the backend left with the formats of the copy before
    assert_eq!(*copies.lock().unwrap(), [Vec::<ClipboardFormat>::new()]);

    // The next copy goes through.
    let list = [unnamed(ClipboardFormatId::CF_UNICODETEXT), named(0xC001, "HTML Format")];
    assert_eq!(process(&mut cliprdr, &encode(&list)), [FormatListResponse::Ok]);
    assert_eq!(copies.lock().unwrap().last().unwrap(), &list);
}

#[test]
fn answers_a_format_name_over_the_limit_with_a_failure() {
    let (mut cliprdr, copies) = ready_client();

    let list = [named(0xC001, &"n".repeat(MAX_FORMAT_NAME_LENGTH + 1))];

    // oracle: spec MS-RDPECLIP §2.2.3.2 - a Format List that isn't processed is answered with CB_RESPONSE_FAIL.
    assert_eq!(process(&mut cliprdr, &encode(&list)), [FormatListResponse::Fail]);
    // oracle: mutant the backend left with the formats of the copy before
    assert_eq!(*copies.lock().unwrap(), [Vec::<ClipboardFormat>::new()]);
}

#[test]
fn answers_a_format_list_whose_names_it_cannot_decode_with_a_failure() {
    let (mut cliprdr, copies) = ready_client();

    // A name with an unpaired surrogate.
    let mut body = 0xC001u32.to_le_bytes().to_vec();
    body.extend_from_slice(&[0x00, 0xD8, 0x00, 0x00]);

    // oracle: spec MS-RDPECLIP §2.2.3.2 - a Format List that isn't processed is answered with CB_RESPONSE_FAIL.
    assert_eq!(
        process(&mut cliprdr, &format_list_pdu(UNICODE_NAMES, &body)),
        [FormatListResponse::Fail]
    );
    // oracle: mutant the backend left with the formats of the copy before
    assert_eq!(*copies.lock().unwrap(), [Vec::<ClipboardFormat>::new()]);
}

#[test]
fn takes_a_server_copy_after_the_server_refuses_the_first_format_list() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));
    let backend = RemoteCopies::default();
    let pastes = Arc::clone(&backend.pastes);
    let mut client = CliprdrClient::new(Box::new(backend));

    // Capabilities and Monitor Ready, then the client's first copy, with more formats than the server takes.
    deliver(&mut client, server.start().unwrap());
    let first_copy = client
        .initiate_copy(&vec![unnamed(ClipboardFormatId::CF_UNICODETEXT); MAX_FORMAT_COUNT + 1])
        .unwrap();
    let answer = deliver(&mut server, first_copy);
    // oracle: spec MS-RDPECLIP §2.2.3.2 - a Format List that isn't processed is answered with CB_RESPONSE_FAIL.
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Fail]);
    deliver(&mut client, answer);

    // The server copies, and the client pastes it.
    // oracle: spec MS-RDPECLIP §1.3.2.1 - the Initialization Sequence's last step is the server's Format List Response,
    // which §3.1.5.2.3 lets refuse the list.
    let answer = deliver(
        &mut client,
        server
            .initiate_copy(&[unnamed(ClipboardFormatId::CF_UNICODETEXT)])
            .unwrap(),
    );
    deliver(&mut server, answer);
    deliver(
        &mut server,
        client.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).unwrap(),
    );
    let data = server
        .submit_format_data(FormatDataResponse::new_unicode_string("copied on the server"))
        .unwrap();
    deliver(&mut client, data);
    assert_eq!(*pastes.lock().unwrap(), ["copied on the server"]);
}

#[test]
fn refuses_format_data_requests_while_its_format_list_stands_refused() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));
    let backend = RemoteCopies::default();
    let requests = Arc::clone(&backend.requests);
    let mut client = CliprdrClient::new(Box::new(backend));

    // The client's first copy, with more formats than the server takes.
    deliver(&mut client, server.start().unwrap());
    let first_copy = client
        .initiate_copy(&vec![unnamed(ClipboardFormatId::CF_UNICODETEXT); MAX_FORMAT_COUNT + 1])
        .unwrap();
    let answer = deliver(&mut server, first_copy);
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Fail]);
    deliver(&mut client, answer);

    // The server asks for the text all the same.
    // oracle: spec MS-RDPECLIP §3.1.5.4.3 - a request after an unsuccessful Format List Response is answered with
    // CB_RESPONSE_FAIL and no data.
    let answer = deliver(
        &mut client,
        server.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).unwrap(),
    );
    let responses = format_data_responses(&answer);
    assert_eq!(responses.len(), 1);
    assert!(responses[0].is_error());
    assert!(responses[0].data().is_empty());
    // oracle: mutant the request handed to the backend, which could answer it with data
    assert!(requests.lock().unwrap().is_empty());
    deliver(&mut server, answer);

    // The next copy is accepted, and the server's request for it reaches the backend.
    // oracle: spec MS-RDPECLIP §3.1.5.2.4 - after a successful Format List Response, Format Data Requests are
    // processed.
    let copy = client
        .initiate_copy(&[unnamed(ClipboardFormatId::CF_UNICODETEXT)])
        .unwrap();
    let answer = deliver(&mut server, copy);
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    deliver(&mut client, answer);
    let answer = deliver(
        &mut client,
        server.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).unwrap(),
    );
    assert!(answer.is_empty());
    assert_eq!(*requests.lock().unwrap(), [ClipboardFormatId::CF_UNICODETEXT]);
}

#[test]
fn server_refuses_format_data_requests_while_its_format_list_stands_refused() {
    let backend = RemoteCopies::default();
    let requests = Arc::clone(&backend.requests);
    let mut server = CliprdrServer::new(Box::new(backend));
    let mut client = CliprdrClient::new(Box::new(TestBackend));

    // Capabilities and Monitor Ready, then the client's first copy, which the server takes.
    deliver(&mut client, server.start().unwrap());
    let first_copy = client
        .initiate_copy(&[unnamed(ClipboardFormatId::CF_UNICODETEXT)])
        .unwrap();
    let answer = deliver(&mut server, first_copy);
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    deliver(&mut client, answer);

    // The server copies more formats than the client takes.
    let refusal = deliver(
        &mut client,
        server
            .initiate_copy(&vec![unnamed(ClipboardFormatId::CF_UNICODETEXT); MAX_FORMAT_COUNT + 1])
            .unwrap(),
    );
    assert_eq!(format_list_responses(&refusal), [FormatListResponse::Fail]);
    deliver(&mut server, refusal);

    // The client asks for the text all the same.
    // oracle: spec MS-RDPECLIP §3.1.5.4.3 - a request after an unsuccessful Format List Response is answered with
    // CB_RESPONSE_FAIL and no data.
    let answer = deliver(
        &mut server,
        client.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).unwrap(),
    );
    let responses = format_data_responses(&answer);
    assert_eq!(responses.len(), 1);
    assert!(responses[0].is_error());
    assert!(responses[0].data().is_empty());
    // oracle: mutant the request handed to the backend, which could answer it with data
    assert!(requests.lock().unwrap().is_empty());
    deliver(&mut client, answer);

    // The next copy is accepted, and the client's request for it reaches the backend.
    // oracle: spec MS-RDPECLIP §3.1.5.2.4 - after a successful Format List Response, Format Data Requests are
    // processed.
    let answer = deliver(
        &mut client,
        server
            .initiate_copy(&[unnamed(ClipboardFormatId::CF_UNICODETEXT)])
            .unwrap(),
    );
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    deliver(&mut server, answer);
    let answer = deliver(
        &mut server,
        client.initiate_paste(ClipboardFormatId::CF_UNICODETEXT).unwrap(),
    );
    assert!(answer.is_empty());
    assert_eq!(*requests.lock().unwrap(), [ClipboardFormatId::CF_UNICODETEXT]);
}

#[test]
fn refuses_a_server_copy_over_the_limit_before_the_server_answers_the_first_format_list() {
    let mut server = CliprdrServer::new(Box::new(TestBackend));
    let backend = RemoteCopies::default();
    let copies = Arc::clone(&backend.copies);
    let mut client = CliprdrClient::new(Box::new(backend));

    // Capabilities and Monitor Ready, then the client's first copy, not yet delivered to the server.
    deliver(&mut client, server.start().unwrap());
    let first_copy = client
        .initiate_copy(&[unnamed(ClipboardFormatId::CF_UNICODETEXT)])
        .unwrap();

    // The server copies more formats than the client takes before that copy reaches it.
    let refusal = deliver(
        &mut client,
        server
            .initiate_copy(&vec![unnamed(ClipboardFormatId::CF_DIB); MAX_FORMAT_COUNT + 1])
            .unwrap(),
    );
    // oracle: spec MS-RDPECLIP §2.2.3.2 - a Format List that isn't processed is answered with CB_RESPONSE_FAIL.
    assert_eq!(format_list_responses(&refusal), [FormatListResponse::Fail]);
    // oracle: mutant the backend not told that the remote clipboard holds nothing it can take
    assert_eq!(*copies.lock().unwrap(), [Vec::<ClipboardFormat>::new()]);
    // oracle: spec MS-RDPECLIP §1.3.2.1 - the Initialization Sequence ends with the server's Format List Response,
    // which the server's own copy is not.
    assert!(!client.is_ready());
    deliver(&mut server, refusal);

    // The server's answer to the client's first copy completes initialization, and the server's next copy is taken.
    let answer = deliver(&mut server, first_copy);
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    deliver(&mut client, answer);
    // oracle: spec MS-RDPECLIP §1.3.2.1 - the Initialization Sequence ends with the server's Format List Response.
    assert!(client.is_ready());
    let list = [unnamed(ClipboardFormatId::CF_UNICODETEXT)];
    let answer = deliver(&mut client, server.initiate_copy(&list).unwrap());
    assert_eq!(format_list_responses(&answer), [FormatListResponse::Ok]);
    assert_eq!(copies.lock().unwrap().last().unwrap(), &list);
}

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_echo::client::EchoClient;
use ironrdp_echo::pdu::{EchoRequestPdu, EchoResponsePdu};
use ironrdp_echo::server::EchoServer;

#[test]
fn request_pdu_roundtrip() {
    let request = EchoRequestPdu::new(b"Hello world!".to_vec());
    let encoded = encode_vec(&request).expect("request should encode");
    let decoded: EchoRequestPdu = decode(&encoded).expect("request should decode");

    assert_eq!(decoded.payload(), b"Hello world!");
}

#[test]
fn response_pdu_roundtrip() {
    let response = EchoResponsePdu::new(b"Hello world!".to_vec());
    let encoded = encode_vec(&response).expect("response should encode");
    let decoded: EchoResponsePdu = decode(&encoded).expect("response should decode");

    assert_eq!(decoded.payload(), b"Hello world!");
}

#[test]
fn client_echoes_request_payload() {
    let mut client = EchoClient::new();
    let request = EchoRequestPdu::new(b"ping".to_vec());
    let encoded_request = encode_vec(&request).expect("request should encode");

    let responses = client
        .process(1, &encoded_request)
        .expect("client should process request");

    assert_eq!(responses.len(), 1);

    let encoded_response = encode_vec(responses[0].as_ref()).expect("response should encode");
    let response: EchoResponsePdu = decode(&encoded_response).expect("response should decode");
    assert_eq!(response.payload(), b"ping");
}

#[test]
fn server_with_initial_request_sends_one_message() {
    let mut server = EchoServer::new().with_initial_request(b"probe".to_vec());

    let messages = server.start(42).expect("server should start channel");

    assert_eq!(messages.len(), 1);

    let encoded_request = encode_vec(messages[0].as_ref()).expect("request should encode");
    let request: EchoRequestPdu = decode(&encoded_request).expect("request should decode");
    assert_eq!(request.payload(), b"probe");
}

#[test]
fn server_rejects_empty_requests() {
    let result = EchoServer::request_message(Vec::new());
    assert!(result.is_err());
}

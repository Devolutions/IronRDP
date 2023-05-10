use ironrdp_connector::ServerName;
use rstest::rstest;

#[rstest]
#[case("somehostname:2345", "somehostname")]
#[case("192.168.56.101:2345", "192.168.56.101")]
#[case("[2001:db8::8a2e:370:7334]:7171", "2001:db8::8a2e:370:7334")]
#[case("[2001:0db8:0000:0000:0000:8a2e:0370:7334]:433", "2001:db8::8a2e:370:7334")]
#[case("[::1]:2222", "::1")]
fn input_with_port_is_sanitized(#[case] input: &str, #[case] expected: &str) {
    let result = ServerName::new(input).into_inner();
    assert_eq!(result, expected);
}

#[rstest]
#[case("somehostname")]
#[case("192.168.56.101")]
#[case("2001:db8::8a2e:370:7334")]
#[case("2001:0db8:0000:0000:0000:8a2e:0370:7334")]
#[case("::1")]
fn input_without_port_is_left_untouched(#[case] input: &str) {
    let result = ServerName::new(input).into_inner();
    assert_eq!(result, input);
}

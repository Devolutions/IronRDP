use ironrdp_cliprdr_format::bitmap::{dib_to_png, dibv5_to_png, png_to_cf_dib, png_to_cf_dibv5};
use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html};

#[test]
fn dib_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    let png = dib_to_png(input).unwrap();
    let converted = png_to_cf_dib(&png).unwrap();
    assert_eq!(converted, input);
}

#[test]
fn dibv5_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dibv5.pdu");
    let png = dibv5_to_png(input).unwrap();
    let converted = png_to_cf_dibv5(&png).unwrap();
    assert_eq!(converted, input);
}

#[test]
fn html_failure() {
    // Empty
    assert!(cf_html_to_plain_html(&[]).is_err());
    // Garbage
    assert!(cf_html_to_plain_html(&[0x00, 0x00, 0x00, 0x00]).is_err());
    // No headers
    assert!(cf_html_to_plain_html(b"hello world").is_err());
    // Headers with fragment size not found
    assert!(cf_html_to_plain_html(b"Version:0.9\r\n<html>nopers</html>").is_err());
    // Out of bounds headers
    assert!(cf_html_to_plain_html(b"StartFragment:999\r\nEndFragment:9999\r\n<html>nopers</html>").is_err());
}

#[test]
fn test_cf_html_to_text() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_html.pdu");
    let actual = cf_html_to_plain_html(input).unwrap();

    // Validate that the output is valid HTML
    assert!(actual.starts_with("<b>Remote Desktop Protocol</b>"));
    assert!(actual.ends_with("</sup>"));

    // Validate roundtrip
    let mut cf_html = plain_html_to_cf_html(&actual);
    let roundtrip_html_text = cf_html_to_plain_html(&cf_html).unwrap();
    assert_eq!(actual, roundtrip_html_text);

    // Add some padding (CF_HTML is not null-terminated, we need to work with data which is
    // potentially padded with arbitrary fill bytes).
    cf_html.extend_from_slice(&[0xFFu8; 10]);
    let roundtrip_html_text = cf_html_to_plain_html(&cf_html).unwrap();
    assert_eq!(actual, roundtrip_html_text);
}

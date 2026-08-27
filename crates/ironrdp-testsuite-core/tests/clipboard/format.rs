use ironrdp_cliprdr_format::bitmap::{
    dib_to_png, dibv5_to_png, png_to_cf_dib, png_to_cf_dibv5, validate_dib, validate_dibv5,
};
use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html, validate_cf_html};

#[test]
fn dib_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    assert_eq!(validate_dib(input).unwrap(), input.len());
    let png = dib_to_png(input).unwrap();
    let converted = png_to_cf_dib(&png).unwrap();
    assert_eq!(converted, input);
}

#[test]
fn dibv5_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dibv5.pdu");
    assert_eq!(validate_dibv5(input).unwrap(), input.len());
    let png = dibv5_to_png(input).unwrap();
    let converted = png_to_cf_dibv5(&png).unwrap();
    assert_eq!(converted, input);
}

#[test]
fn html_failure() {
    // Empty
    assert!(cf_html_to_plain_html(&[]).is_err());
    // Garbage
    assert!(cf_html_to_plain_html(&[0x00, 0x01, 0x02, 0x03]).is_err());
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
    let cf_html = plain_html_to_cf_html(actual);
    let roundtrip_html_text = cf_html_to_plain_html(cf_html.as_bytes()).unwrap();
    assert_eq!(actual, roundtrip_html_text);

    // Add some padding (CF_HTML is not null-terminated, we need to work with data which is
    // potentially padded with arbitrary fill bytes).
    let mut cf_html = cf_html.into_bytes();
    cf_html.extend_from_slice(&[0xFF; 10]);
    assert_eq!(validate_cf_html(&cf_html).unwrap(), cf_html.len() - 10);
    let roundtrip_html_text = cf_html_to_plain_html(&cf_html).unwrap();
    assert_eq!(actual, roundtrip_html_text);
}

#[test]
fn html_without_context_is_validated_and_trimmed() {
    let fragment = "<b>Remote Desktop Protocol</b>";
    let template =
        "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let start_fragment = template.len() + "<!--StartFragment-->".len();
    let end_fragment = start_fragment + fragment.len();
    let mut cf_html = format!(
        "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n<!--StartFragment-->{fragment}<!--EndFragment-->"
    )
    .into_bytes();
    let payload_len = cf_html.len();
    cf_html.extend_from_slice(b"allocator padding");

    assert_eq!(validate_cf_html(&cf_html).unwrap(), payload_len);
    assert_eq!(cf_html_to_plain_html(&cf_html).unwrap(), fragment);
}

#[test]
fn html_rejects_missing_or_ambiguous_required_headers() {
    let valid = plain_html_to_cf_html("<b>Remote Desktop Protocol</b>");
    let missing_start_html = valid.replacen("StartHTML:", "Unknown:", 1);
    assert!(validate_cf_html(missing_start_html.as_bytes()).is_err());

    let embedded_end_html = valid.replacen("EndHTML:", "UnknownEndHTML:", 1);
    assert!(validate_cf_html(embedded_end_html.as_bytes()).is_err());

    let duplicate_version = format!("Version:1.0\r\n{valid}");
    assert!(validate_cf_html(duplicate_version.as_bytes()).is_err());
}

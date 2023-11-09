use thiserror::Error;

#[derive(Debug, Error)]
pub enum HtmlError {
    #[error("invalid CF_HTML format")]
    InvalidFormat,
    #[error("invalid UTF-8")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    #[error("failed to parse integer")]
    InvalidInteger(#[from] std::num::ParseIntError),
    #[error("invalid integer conversion")]
    InvalidConversion,
}

/// Convert `CF_HTML` format to plain text.
pub fn cf_html_to_text(input: &[u8]) -> Result<String, HtmlError> {
    let mut start_fragment = None;
    let mut end_fragment = None;

    let mut headers_cursor = input;

    let fragment = loop {
        // Line split logic is manual instead of using BufReader::read_line because
        // the line ending could be represented as `\r\n`, `\n` or even `\r`.
        const ENDLINE_CONTROLS: &[u8] = &[b'\r', b'\n'];

        // Failed to find the end of the line
        let end_pos = match headers_cursor.iter().position(|ch| ENDLINE_CONTROLS.contains(ch)) {
            Some(pos) => pos,
            None => return Err(HtmlError::InvalidFormat),
        };

        let line = String::from_utf8(headers_cursor[..end_pos].to_vec())?;

        let header_value_to_u32 = |value: &str| value.trim_start_matches('0').parse::<u32>();

        match line.split_once(':') {
            Some((key, value)) => match key {
                "StartFragment" => {
                    start_fragment = Some(header_value_to_u32(value)?);
                }
                "EndFragment" => {
                    end_fragment = Some(header_value_to_u32(value)?);
                }
                _ => {
                    // We are not interested in other headers.
                }
            },
            None => {
                if start_fragment.is_none() || end_fragment.is_none() {
                    // We reached the end of the headers, but we didn't find the required ones,
                    // so the format is invalid.
                    return Err(HtmlError::InvalidFormat);
                }
            }
        };

        if let (Some(start), Some(end)) = (start_fragment, end_fragment) {
            let start = usize::try_from(start).map_err(|_| HtmlError::InvalidConversion)?;
            let end = usize::try_from(end).map_err(|_| HtmlError::InvalidConversion)?;

            // Extract fragment from the original buffer.
            if start > end || end > input.len() {
                return Err(HtmlError::InvalidFormat);
            }

            break String::from_utf8(input[start..end].to_vec())?;
        }

        // INVARIANT: end_pos < headers_cursor.len() - 1
        // This is safe because we already checked above that the line ends with `\r` or `\n`.
        #[allow(clippy::arithmetic_side_effects)]
        {
            // Go to the next line, skipping any leftover `LF` if CRLF was used.
            let has_leftover_lf = end_pos + 1 != headers_cursor.len()
                && headers_cursor[end_pos] == b'\r'
                && headers_cursor[end_pos + 1] == b'\n';

            if has_leftover_lf {
                headers_cursor = &headers_cursor[end_pos + 2..];
            } else {
                headers_cursor = &headers_cursor[end_pos + 1..];
            }
        }
    };

    Ok(fragment)
}

/// Convert plain text HTML to `CF_HTML` format.
pub fn text_to_cf_html(fragment: &str) -> Vec<u8> {
    let mut buffer = Vec::new();

    // INVARIANT: key.len() + value.len() + ":\r\n".len() < usize::MAX
    // This is always true because we know `key` and `value` used in code below are
    // short and their sizes are far from `usize::MAX`.
    #[allow(clippy::arithmetic_side_effects)]
    let mut write_header = |key: &str, value: &str| {
        let size = key.len() + value.len() + ":\r\n".len();
        buffer.reserve(size);

        buffer.extend_from_slice(key.as_bytes());
        buffer.extend_from_slice(b":");
        let value_pos = buffer.len();
        buffer.extend_from_slice(value.as_bytes());
        buffer.extend_from_slice(b"\r\n");

        value_pos
    };

    const POS_PLACEHOLDER: &str = "0000000000";

    write_header("Version", "0.9");
    let start_html_placeholder_pos = write_header("StartHTML", POS_PLACEHOLDER);
    let end_html_placeholder_pos = write_header("EndHTML", POS_PLACEHOLDER);
    let start_fragment_placeholder_pos = write_header("StartFragment", POS_PLACEHOLDER);
    let end_fragment_placeholder_pos = write_header("EndFragment", POS_PLACEHOLDER);

    let start_html_pos = buffer.len();
    buffer.extend_from_slice(b"<html>\r\n<body>\r\n<!--StartFragment-->");

    let start_fragment_pos = buffer.len();
    buffer.extend_from_slice(fragment.as_bytes());

    let end_fragment_pos = buffer.len();
    buffer.extend_from_slice(b"<!--EndFragment-->\r\n</body>\r\n</html>");

    let end_html_pos = buffer.len();

    let start_html_pos_value = format!("{:0>10}", start_html_pos);
    let end_html_pos_value = format!("{:0>10}", end_html_pos);
    let start_fragment_pos_value = format!("{:0>10}", start_fragment_pos);
    let end_fragment_pos_value = format!("{:0>10}", end_fragment_pos);

    // INVARIANT: placeholder_pos + POS_PLACEHOLDER.len() < buffer.len()
    // This is always valid because we know that placeholder is always present in the buffer
    // fter the header is written and placeholder is within the bounds of the buffer.
    #[allow(clippy::arithmetic_side_effects)]
    let mut replace_placeholder = |placeholder_pos: usize, placeholder_value: &str| {
        buffer[placeholder_pos..placeholder_pos + POS_PLACEHOLDER.len()].copy_from_slice(placeholder_value.as_bytes());
    };

    replace_placeholder(start_html_placeholder_pos, &start_html_pos_value);
    replace_placeholder(end_html_placeholder_pos, &end_html_pos_value);
    replace_placeholder(start_fragment_placeholder_pos, &start_fragment_pos_value);
    replace_placeholder(end_fragment_placeholder_pos, &end_fragment_pos_value);

    buffer
}

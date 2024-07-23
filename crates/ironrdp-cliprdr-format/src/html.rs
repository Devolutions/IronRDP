use thiserror::Error;

#[derive(Debug, Error)]
pub enum HtmlError {
    #[error("invalid CF_HTML format")]
    InvalidFormat,
    #[error("invalid UTF-8")]
    InvalidUtf8(#[from] core::str::Utf8Error),
    #[error("failed to parse integer")]
    InvalidInteger(#[from] core::num::ParseIntError),
    #[error("invalid integer conversion")]
    InvalidConversion,
}

/// Converts `CF_HTML` format to plain HTML text.
///
/// Note that the `CF_HTML` format is using UTF-8, and the input is expected to be valid UTF-8.
/// However, there is no easy way to know the size of the `CF_HTML` payload:
/// 1) it’s typically not null-terminated, and
/// 2) reading the headers is already half of the work.
/// Because of that, this function takes the input as a byte slice and finds the end of the payload itself.
/// This is expected to be more convenient at the callsite.
pub fn cf_html_to_plain_html(input: &[u8]) -> Result<&str, HtmlError> {
    const EOL_CONTROL_CHARS: &[u8] = b"\r\n";

    let mut start_fragment = None;
    let mut end_fragment = None;

    // We’ll move the lower bound of this slice until all headers are read.
    let mut cursor = input;

    loop {
        let line = {
            // We use a custom logic for splitting lines, instead of something like `str::lines`.
            // That’s because `str::lines` does not split at carriage return (`\r`) not followed by line feed (`\n`).
            // In `CF_HTML` format, the line ending could be represented using `\r` alone.
            let eol_pos = cursor
                .iter()
                .position(|byte| EOL_CONTROL_CHARS.contains(byte))
                .ok_or(HtmlError::InvalidFormat)?;
            core::str::from_utf8(&cursor[..eol_pos])?
        };

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
                // At this point, we reached the end of the headers.
                if let (Some(start), Some(end)) = (start_fragment, end_fragment) {
                    let start = usize::try_from(start).map_err(|_| HtmlError::InvalidConversion)?;
                    let end = usize::try_from(end).map_err(|_| HtmlError::InvalidConversion)?;

                    // Ensure start and end values are properly bounded.
                    if !(start < end && end < input.len()) {
                        return Err(HtmlError::InvalidFormat);
                    }

                    // Extract the fragment from the original buffer.
                    let fragment = core::str::from_utf8(&input[start..end])?;

                    return Ok(fragment);
                } else {
                    // If required headers were not found, the input is considered invalid.
                    return Err(HtmlError::InvalidFormat);
                }
            }
        };

        // Skip EOL control characters and prepare for next line.
        cursor = &cursor[line.len()..];
        while let Some(b'\n' | b'\r') = cursor.first() {
            cursor = &cursor[1..];
        }
    }

    fn header_value_to_u32(value: &str) -> Result<u32, std::num::ParseIntError> {
        value.trim_start_matches('0').parse::<u32>()
    }
}

/// Converts plain HTML text to `CF_HTML` format.
pub fn plain_html_to_cf_html(fragment: &str) -> String {
    const POS_PLACEHOLDER: &str = "0000000000";

    let mut buffer = String::new();

    let mut write_header = |key: &str, value: &str| {
        // This relation holds: key.len() + value.len() + ":\r\n".len() < usize::MAX
        // Rationale: we know all possible values (see code below), and they are much smaller than `usize::MAX`.
        #[allow(clippy::arithmetic_side_effects)]
        let size = key.len() + value.len() + ":\r\n".len();
        buffer.reserve(size);

        buffer.push_str(key);
        buffer.push(':');
        let value_pos = buffer.len();
        buffer.push_str(value);
        buffer.push_str("\r\n");

        value_pos
    };

    write_header("Version", "0.9");

    let start_html_header_value_pos = write_header("StartHTML", POS_PLACEHOLDER);
    let end_html_header_value_pos = write_header("EndHTML", POS_PLACEHOLDER);
    let start_fragment_header_value_pos = write_header("StartFragment", POS_PLACEHOLDER);
    let end_fragment_header_value_pos = write_header("EndFragment", POS_PLACEHOLDER);

    let start_html_pos = buffer.len();
    buffer.push_str("<html>\r\n<body>\r\n<!--StartFragment-->");

    let start_fragment_pos = buffer.len();
    buffer.push_str(fragment);

    let end_fragment_pos = buffer.len();
    buffer.push_str("<!--EndFragment-->\r\n</body>\r\n</html>");

    let end_html_pos = buffer.len();

    let start_html_pos_value = format!("{:0>10}", start_html_pos);
    let end_html_pos_value = format!("{:0>10}", end_html_pos);
    let start_fragment_pos_value = format!("{:0>10}", start_fragment_pos);
    let end_fragment_pos_value = format!("{:0>10}", end_fragment_pos);

    let mut replace_placeholder = |value_begin_idx: usize, header_value: &str| {
        // We know that: value_begin_idx + POS_PLACEHOLDER.len() < usize::MAX
        // Rationale: the headers are written at the beginning, and we’re not indexing outside of the string.
        #[allow(clippy::arithmetic_side_effects)]
        let value_end_idx = value_begin_idx + POS_PLACEHOLDER.len();

        buffer.replace_range(value_begin_idx..value_end_idx, header_value);
    };

    replace_placeholder(start_html_header_value_pos, &start_html_pos_value);
    replace_placeholder(end_html_header_value_pos, &end_html_pos_value);
    replace_placeholder(start_fragment_header_value_pos, &start_fragment_pos_value);
    replace_placeholder(end_fragment_header_value_pos, &end_fragment_pos_value);

    buffer
}

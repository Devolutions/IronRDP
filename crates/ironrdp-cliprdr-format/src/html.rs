#[derive(Debug)]
pub enum HtmlError {
    InvalidFormat,
    InvalidUtf8(core::str::Utf8Error),
    InvalidInteger(core::num::ParseIntError),
    InvalidConversion,
}

impl core::fmt::Display for HtmlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HtmlError::InvalidFormat => write!(f, "invalid CF_HTML format"),
            HtmlError::InvalidUtf8(_error) => write!(f, "invalid UTF-8"),
            HtmlError::InvalidInteger(_error) => write!(f, "failed to parse integer"),
            HtmlError::InvalidConversion => write!(f, "invalid integer conversion"),
        }
    }
}

impl core::error::Error for HtmlError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            HtmlError::InvalidFormat => None,
            HtmlError::InvalidUtf8(utf8_error) => Some(utf8_error),
            HtmlError::InvalidInteger(parse_int_error) => Some(parse_int_error),
            HtmlError::InvalidConversion => None,
        }
    }
}

impl From<core::str::Utf8Error> for HtmlError {
    fn from(error: core::str::Utf8Error) -> Self {
        HtmlError::InvalidUtf8(error)
    }
}

impl From<core::num::ParseIntError> for HtmlError {
    fn from(error: core::num::ParseIntError) -> Self {
        HtmlError::InvalidInteger(error)
    }
}

struct CfHtml<'a> {
    fragment: &'a str,
    payload_len: usize,
}

#[derive(Default)]
struct CfHtmlHeaders<'a> {
    version: Option<&'a str>,
    start_html: Option<i64>,
    end_html: Option<i64>,
    start_fragment: Option<i64>,
    end_fragment: Option<i64>,
    start_selection: Option<i64>,
    end_selection: Option<i64>,
}

impl CfHtmlHeaders<'_> {
    fn has_required_fields(&self) -> bool {
        self.version.is_some()
            && self.start_html.is_some()
            && self.end_html.is_some()
            && self.start_fragment.is_some()
            && self.end_fragment.is_some()
    }
}

fn parse_cf_html(input: &[u8]) -> Result<CfHtml<'_>, HtmlError> {
    const EOL_CONTROL_CHARS: &[u8] = b"\r\n";
    const START_FRAGMENT_MARKER: &[u8] = b"<!--StartFragment-->";
    const END_FRAGMENT_MARKER: &[u8] = b"<!--EndFragment-->";

    let mut headers = CfHtmlHeaders::default();
    let mut header_len = 0;
    while !headers.has_required_fields() {
        header_len = parse_header_line(input, header_len, &mut headers)?;
    }

    if !matches!(headers.version, Some("0.9" | "1.0")) {
        return Err(HtmlError::InvalidFormat);
    }

    let start_fragment = positive_offset(headers.start_fragment)?;
    let end_fragment = positive_offset(headers.end_fragment)?;
    if !(header_len <= start_fragment && start_fragment < end_fragment && end_fragment <= input.len()) {
        return Err(HtmlError::InvalidFormat);
    }

    let marker_start = start_fragment
        .checked_sub(START_FRAGMENT_MARKER.len())
        .ok_or(HtmlError::InvalidFormat)?;
    let header_boundary = match headers.start_html {
        Some(-1) => marker_start,
        Some(start) => usize::try_from(start).map_err(|_| HtmlError::InvalidConversion)?,
        None => return Err(HtmlError::InvalidFormat),
    };
    while header_len < header_boundary {
        header_len = parse_header_line(input, header_len, &mut headers)?;
    }
    if header_len != header_boundary {
        return Err(HtmlError::InvalidFormat);
    }

    if input.get(marker_start..start_fragment) != Some(START_FRAGMENT_MARKER) {
        return Err(HtmlError::InvalidFormat);
    }
    let marker_end = end_fragment
        .checked_add(END_FRAGMENT_MARKER.len())
        .ok_or(HtmlError::InvalidFormat)?;
    if input.get(end_fragment..marker_end) != Some(END_FRAGMENT_MARKER) {
        return Err(HtmlError::InvalidFormat);
    }

    let payload_len = match (headers.start_html, headers.end_html) {
        (Some(-1), Some(-1)) => marker_end,
        (Some(start), Some(end)) if 0 <= start && 0 <= end => {
            let start = usize::try_from(start).map_err(|_| HtmlError::InvalidConversion)?;
            let end = usize::try_from(end).map_err(|_| HtmlError::InvalidConversion)?;
            if !(header_len == start && start <= marker_start && marker_end <= end && end <= input.len()) {
                return Err(HtmlError::InvalidFormat);
            }
            end
        }
        _ => return Err(HtmlError::InvalidFormat),
    };

    match (headers.start_selection, headers.end_selection) {
        (None, None) => {}
        (Some(start), Some(end)) => {
            let start = usize::try_from(start).map_err(|_| HtmlError::InvalidConversion)?;
            let end = usize::try_from(end).map_err(|_| HtmlError::InvalidConversion)?;
            if !(start_fragment <= start && start <= end && end <= end_fragment) {
                return Err(HtmlError::InvalidFormat);
            }
        }
        _ => return Err(HtmlError::InvalidFormat),
    }

    core::str::from_utf8(&input[..payload_len])?;
    let fragment = core::str::from_utf8(&input[start_fragment..end_fragment])?;

    fn parse_header_line<'a>(
        input: &'a [u8],
        offset: usize,
        headers: &mut CfHtmlHeaders<'a>,
    ) -> Result<usize, HtmlError> {
        let rest = input.get(offset..).ok_or(HtmlError::InvalidFormat)?;
        let eol_pos = rest
            .iter()
            .position(|byte| EOL_CONTROL_CHARS.contains(byte))
            .ok_or(HtmlError::InvalidFormat)?;
        let line = core::str::from_utf8(&rest[..eol_pos])?;
        let (key, value) = line.split_once(':').ok_or(HtmlError::InvalidFormat)?;
        match key {
            "Version" if headers.version.replace(value).is_some() => return Err(HtmlError::InvalidFormat),
            "StartHTML" if headers.start_html.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            "EndHTML" if headers.end_html.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            "StartFragment" if headers.start_fragment.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            "EndFragment" if headers.end_fragment.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            "StartSelection" if headers.start_selection.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            "EndSelection" if headers.end_selection.replace(parse_offset(value)?).is_some() => {
                return Err(HtmlError::InvalidFormat);
            }
            _ => {}
        }

        let mut next = offset.checked_add(eol_pos).ok_or(HtmlError::InvalidConversion)?;
        while matches!(input.get(next), Some(b'\n' | b'\r')) {
            next = next.checked_add(1).ok_or(HtmlError::InvalidConversion)?;
        }
        Ok(next)
    }

    fn parse_offset(value: &str) -> Result<i64, HtmlError> {
        Ok(value.trim().parse()?)
    }

    fn positive_offset(value: Option<i64>) -> Result<usize, HtmlError> {
        usize::try_from(value.ok_or(HtmlError::InvalidFormat)?).map_err(|_| HtmlError::InvalidConversion)
    }

    Ok(CfHtml { fragment, payload_len })
}

/// Validates a `CF_HTML` payload and returns its logical byte length.
pub fn validate_cf_html(input: &[u8]) -> Result<usize, HtmlError> {
    Ok(parse_cf_html(input)?.payload_len)
}

/// Converts `CF_HTML` format to plain HTML text.
///
/// Note that the `CF_HTML` format is using UTF-8, and the input is expected to be valid UTF-8.
/// However, there is no easy way to know the size of the `CF_HTML` payload:
/// 1) it’s typically not null-terminated, and
/// 2) reading the headers is already half of the work.
///
/// Because of that, this function takes the input as a byte slice and finds the end of the payload itself.
/// This is expected to be more convenient at the callsite.
pub fn cf_html_to_plain_html(input: &[u8]) -> Result<&str, HtmlError> {
    Ok(parse_cf_html(input)?.fragment)
}

/// Converts plain HTML text to `CF_HTML` format.
pub fn plain_html_to_cf_html(fragment: &str) -> String {
    const POS_PLACEHOLDER: &str = "0000000000";

    let mut buffer = String::new();

    let mut write_header = |key: &str, value: &str| {
        // This relation holds: key.len() + value.len() + ":\r\n".len() < usize::MAX
        // Rationale: we know all possible values (see code below), and they are much smaller than `usize::MAX`.
        #[expect(clippy::arithmetic_side_effects)]
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

    let start_html_pos_value = format!("{start_html_pos:0>10}");
    let end_html_pos_value = format!("{end_html_pos:0>10}");
    let start_fragment_pos_value = format!("{start_fragment_pos:0>10}");
    let end_fragment_pos_value = format!("{end_fragment_pos:0>10}");

    let mut replace_placeholder = |value_begin_idx: usize, header_value: &str| {
        // We know that: value_begin_idx + POS_PLACEHOLDER.len() < usize::MAX
        // Rationale: the headers are written at the beginning, and we’re not indexing outside of the string.
        #[expect(clippy::arithmetic_side_effects)]
        let value_end_idx = value_begin_idx + POS_PLACEHOLDER.len();

        buffer.replace_range(value_begin_idx..value_end_idx, header_value);
    };

    replace_placeholder(start_html_header_value_pos, &start_html_pos_value);
    replace_placeholder(end_html_header_value_pos, &end_html_pos_value);
    replace_placeholder(start_fragment_header_value_pos, &start_fragment_pos_value);
    replace_placeholder(end_fragment_header_value_pos, &end_fragment_pos_value);

    buffer
}

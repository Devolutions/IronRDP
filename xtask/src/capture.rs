pub fn paths_from_ls_files(output: &[u8]) -> Vec<&[u8]> {
    output
        .split(|byte| *byte == b'\0')
        .filter(|path| is_capture_path(path))
        .collect()
}

fn is_capture_path(path: &[u8]) -> bool {
    path.rsplit(|byte| *byte == b'.')
        .next()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(b"pcap") || extension.eq_ignore_ascii_case(b"pcapng"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_capture_extensions_without_case_sensitivity() {
        assert_eq!(
            paths_from_ls_files(b"session.PCAP\0session.PCAPNG\0session.txt\0"),
            [b"session.PCAP".as_slice(), b"session.PCAPNG".as_slice()]
        );
    }

    #[test]
    fn handles_quoted_and_newline_file_names() {
        assert_eq!(
            paths_from_ls_files(b"\"quoted\".pcap\0newline\nsession.PCAPNG\0session.txt\0"),
            [b"\"quoted\".pcap".as_slice(), b"newline\nsession.PCAPNG".as_slice()]
        );
    }
}

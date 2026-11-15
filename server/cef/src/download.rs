use std::collections::BTreeMap;

#[cfg(feature = "runtime")]
pub mod runtime;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub file_bytes: u64,
    pub session_bytes: u64,
    pub concurrent: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 1024 * 1024 * 1024,
            session_bytes: 4 * 1024 * 1024 * 1024,
            concurrent: 2,
        }
    }
}

impl Limits {
    pub fn validate(self) -> anyhow::Result<Self> {
        anyhow::ensure!(
            0 < self.file_bytes && self.file_bytes <= 1024 * 1024 * 1024,
            "file limit must be 1..=1073741824 bytes"
        );
        anyhow::ensure!(
            self.file_bytes <= self.session_bytes,
            "session limit must not be smaller than the file limit"
        );
        anyhow::ensure!(self.session_bytes <= i64::MAX as u64, "session limit is too large");
        anyhow::ensure!((1..=2).contains(&self.concurrent), "download concurrency must be 1..=2");
        Ok(self)
    }
}

#[derive(Debug)]
pub struct Budget {
    limits: Limits,
    used: u64,
    observed: BTreeMap<u32, u64>,
}

impl Budget {
    pub fn new(limits: Limits) -> anyhow::Result<Self> {
        Ok(Self {
            limits: limits.validate()?,
            used: 0,
            observed: BTreeMap::new(),
        })
    }

    pub fn start(&mut self, id: u32) -> anyhow::Result<()> {
        anyhow::ensure!(!self.observed.contains_key(&id), "duplicate download identifier");
        anyhow::ensure!(
            self.observed.len() < self.limits.concurrent,
            "download concurrency limit reached"
        );
        anyhow::ensure!(
            self.used < self.limits.session_bytes,
            "download session byte budget exhausted"
        );
        self.observed.insert(id, 0);
        Ok(())
    }

    pub fn observe(&mut self, id: u32, actual: u64) -> anyhow::Result<()> {
        let previous = self
            .observed
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("unknown download identifier"))?;
        self.used = self.used.saturating_add(actual.saturating_sub(*previous));
        *previous = (*previous).max(actual);
        anyhow::ensure!(actual <= self.limits.file_bytes, "download exceeds the file byte limit");
        anyhow::ensure!(
            self.used <= self.limits.session_bytes,
            "download exceeds the session byte budget"
        );
        Ok(())
    }

    pub fn finish(&mut self, id: u32) {
        self.observed.remove(&id);
    }

    pub fn used(&self) -> u64 {
        self.used
    }
}

pub fn safe_name(name: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !name.is_empty() && name.len() <= 180,
        "download filename must contain 1..=180 UTF-8 bytes"
    );
    anyhow::ensure!(
        !name.starts_with('.')
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|ch| ch.is_control()                 || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')),
        "unsafe download filename"
    );
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    anyhow::ensure!(
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.len() == 4
                && stem.as_bytes()[3].is_ascii_digit()),
        "reserved download filename"
    );
    Ok(name.to_owned())
}

pub fn collision_name(name: &str, attempt: u32) -> String {
    if attempt == 0 {
        return name.to_owned();
    }
    match name.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem} ({attempt}).{extension}"),
        None => format!("{name} ({attempt})"),
    }
}

pub fn pdf_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| {
            if ch.is_control()
                || matches!(
                    ch,
                    '/' | '\\'
                        | ':'
                        | '*'
                        | '?'
                        | '"'
                        | '<'
                        | '>'
                        | '|'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                )
            {
                '-'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['.', ' ']);
    let stem = trimmed
        .rsplit_once('.')
        .filter(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
        .map_or(trimmed, |(base, _)| base.trim_end_matches(['.', ' ']));
    let mut bounded = String::new();
    for ch in stem.chars() {
        if bounded.len() + ch.len_utf8() > 120 {
            break;
        }
        bounded.push(ch);
    }
    let bounded = bounded.trim_end_matches(['.', ' ']);
    if bounded.is_empty() {
        return "page.pdf".to_owned();
    }
    let candidate = format!("{bounded}.pdf");
    safe_name(&candidate).unwrap_or_else(|_| "page.pdf".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_file_name_sanitizes_arbitrary_page_titles() {
        // oracle: mutant rejecting titles with colons, reserved DOS stems, or empty/oversized strings during PrintToPDF.
        assert_eq!(pdf_file_name("Quarterly: Report / Q1?"), "Quarterly- Report - Q1-.pdf");
        assert_eq!(pdf_file_name("Receipt.PDF"), "Receipt.pdf");
        assert_eq!(pdf_file_name("   ...   "), "page.pdf");
        assert_eq!(pdf_file_name("PRN"), "page.pdf");
        let long = "é".repeat(200);
        let generated = pdf_file_name(&long);
        assert!(safe_name(&generated).is_ok());
        assert!(generated.ends_with(".pdf"));
    }
}

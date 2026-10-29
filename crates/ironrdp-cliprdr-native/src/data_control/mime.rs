//! MIME type matching that tolerates charset parameters.

/// Find a MIME type match in `available`, tolerating charset differences.
///
/// A source may offer `text/plain` while a consumer asks for
/// `text/plain;charset=utf-8`, or the other way round. An exact match wins;
/// for `text/` types the charset parameter is then stripped or added, and
/// finally any offered type with the same base type is accepted.
///
/// Returns the string from `available` that should be used for the request, so
/// that the compositor receives a type the source actually offered.
///
/// The last step means a source that offers only another charset is still matched, so the bytes
/// of a read can be in a different charset than the one requested. This crate hands them over as
/// the source sent them and leaves any conversion to the caller. To see which type a read will
/// use, call this function on [`DataControl::selection_mime_types`] first.
///
/// [`DataControl::selection_mime_types`]: super::DataControl::selection_mime_types
///
/// ```
/// use ironrdp_cliprdr_native::data_control::find_mime_match;
///
/// let available = vec!["text/plain;charset=utf-8".to_owned()];
/// assert_eq!(
///     find_mime_match("text/plain", &available),
///     Some("text/plain;charset=utf-8")
/// );
/// assert_eq!(find_mime_match("image/png", &available), None);
/// ```
#[must_use]
pub fn find_mime_match<'a>(requested: &str, available: &'a [String]) -> Option<&'a str> {
    let mut available = available.iter().map(String::as_str);
    if let Some(found) = find_charset_variant(requested, available.clone()) {
        return Some(found);
    }

    if !requested.starts_with("text/") {
        return None;
    }
    let base = requested.split(';').next()?;

    // The last resort: another charset of the same base type.
    available.find(|m| m.split(';').next() == Some(base))
}

/// Find the entry of `available` that is `requested` apart from its charset parameter.
///
/// An exact match wins. For `text/` types the charset is then stripped from the request, or
/// `;charset=utf-8` is added to it, and the result is looked up. An entry in another charset never
/// matches, because its bytes are not what the request asks for. [`find_mime_match`] adds that as
/// a last resort for reading, where the caller takes what the source offers.
pub(crate) fn find_charset_variant<'a>(
    requested: &str,
    mut available: impl Iterator<Item = &'a str> + Clone,
) -> Option<&'a str> {
    if let Some(found) = available.clone().find(|m| *m == requested) {
        return Some(found);
    }

    if !requested.starts_with("text/") {
        return None;
    }

    if let Some((base, _charset)) = requested.split_once(';') {
        // The request carries a charset, so try the bare type.
        return available.find(|m| *m == base);
    }

    // The request has none, so try the common charset spellings.
    [";charset=utf-8", ";charset=UTF-8"].into_iter().find_map(|suffix| {
        let with_charset = format!("{requested}{suffix}");
        available.clone().find(|m| *m == with_charset)
    })
}

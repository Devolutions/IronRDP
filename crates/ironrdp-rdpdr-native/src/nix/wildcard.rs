//! Directory-query name patterns, matched as MS-FSA [2.1.4.4] Algorithm for Determining if a FileName Is in an Expression describes.
//!
//! Windows sends the DOS wildcards its Win32 layer produces: `<` (DOS_STAR), `>` (DOS_QM) and `"` (DOS_DOT), besides `*` and `?`.
//!
//! [2.1.4.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0b034646-4e23-4874-8488-2adac231ff23

/// Whether `component` is a pattern rather than a literal name.
pub(crate) fn contains_wildcards(component: &str) -> bool {
    component.contains(['*', '?', '<', '>', '"'])
}

/// Whether `name` matches `expression`, ignoring case.
///
/// The match runs in time proportional to the product of both lengths, whatever wildcards the expression holds.
pub(crate) fn is_name_in_expression(name: &str, expression: &str) -> bool {
    if name.is_empty() || expression.is_empty() {
        return name.is_empty() && expression.is_empty();
    }
    if matches!(expression, "*" | "*.*") {
        return true;
    }

    let name = name.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
    let expression = expression.chars().flat_map(char::to_lowercase).collect::<Vec<_>>();
    let last_dot = name.iter().rposition(|character| *character == '.');

    // INVARIANT: `rest[i]` holds whether `name[i..]` matches the part of the expression after the current character.
    let mut rest = vec![false; name.len() + 1];
    rest[name.len()] = true;
    // Every pass writes each entry of `matched` before reading it, so one buffer serves every pass.
    let mut matched = vec![false; name.len() + 1];
    for wildcard in expression.iter().rev() {
        for i in (0..=name.len()).rev() {
            let character = name.get(i).copied();
            matched[i] = match wildcard {
                '*' => rest[i] || (character.is_some() && matched[i + 1]),
                '?' => character.is_some() && rest[i + 1],
                // DOS_STAR matches up to, but not including, the final period.
                '<' => rest[i] || (character.is_some() && last_dot != Some(i) && matched[i + 1]),
                // DOS_QM matches one character, or nothing at a period or the end of the name.
                '>' => match character {
                    Some(character) if character != '.' => rest[i + 1],
                    _ => rest[i],
                },
                // DOS_DOT matches a period, or nothing at the end of the name.
                '"' => match character {
                    Some('.') => rest[i + 1],
                    Some(_) => false,
                    None => rest[i],
                },
                literal => character == Some(*literal) && rest[i + 1],
            };
        }
        std::mem::swap(&mut rest, &mut matched);
    }
    rest[0]
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    // oracle: spec MS-FSA §2.1.4.4
    #[rstest]
    #[case::star_matches_everything("*", "report.txt", true)]
    #[case::star_dot_star_matches_a_name_without_extension("*.*", "README", true)]
    #[case::star_suffix("*.txt", "notes.txt", true)]
    #[case::star_suffix_mismatch("*.txt", "notes.doc", false)]
    #[case::question_mark_is_one_character("a?c", "abc", true)]
    #[case::question_mark_needs_a_character("a?c", "ac", false)]
    #[case::case_is_ignored("*.TXT", "notes.txt", true)]
    #[case::dos_star_stops_before_the_final_period("<.txt", "a.b.txt", true)]
    #[case::dos_star_dot_dos_dot_star_matches_without_extension("<\"*", "README", true)]
    #[case::dos_star_dos_dot_matches_only_without_extension("<\"", "README", true)]
    #[case::dos_star_dos_dot_rejects_an_extension("<\"", "notes.txt", false)]
    #[case::dos_qm_skips_at_a_period("a>.txt", "a.txt", true)]
    #[case::dos_qm_matches_one_character("a>.txt", "ab.txt", true)]
    #[case::dos_qm_matches_at_most_one_character("a>.txt", "abc.txt", false)]
    #[case::literal("notes.txt", "notes.txt", true)]
    #[case::empty_name("*", "", false)]
    #[case::dos_star_after_the_final_period("a\"<\"*", "a.txt", true)]
    fn matches_names_against_expressions(#[case] expression: &str, #[case] name: &str, #[case] expected: bool) {
        assert_eq!(is_name_in_expression(name, expression), expected);
    }
}

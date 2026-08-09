use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;
use tinyjson::JsonValue;

const ALLOWED_TYPES: &[&str] = &[
    "fix", "feat", "build", "chore", "ci", "docs", "style", "refactor", "test", "perf",
];

const CANONICAL_SCOPES: &[&str] = &[
    "meta",
    "core",
    "error",
    "pdu",
    "str",
    "bulk",
    "graphics",
    "config",
    "input",
    "connector",
    "session",
    "driver",
    "svc",
    "dvc",
    "cliprdr",
    "rdpdr",
    "rdpsnd",
    "displaycontrol",
    "echo",
    "egfx",
    "rdpeusb",
    "rdcleanpath",
    "tls",
    "mstsgu",
    "vmconnect",
    "client",
    "viewer",
    "agent",
    "daemon",
    "rpc",
    "activex",
    "server",
    "web",
    "ffi",
    "replay",
    "xtask",
    "release",
    "pr-automation",
    "agents",
];

pub fn check_message(event_file: Option<&Path>) -> anyhow::Result<()> {
    let event_file = match event_file {
        Some(path) => path.to_owned(),
        None => std::env::var_os("GITHUB_EVENT_PATH")
            .map(Into::into)
            .context("missing --event-file and GITHUB_EVENT_PATH")?,
    };

    let event = std::fs::read_to_string(&event_file)
        .with_context(|| format!("read GitHub event file: {}", event_file.display()))?;
    let event: JsonValue = event.parse().context("parse GitHub event JSON")?;
    let (title, body) = pull_request_message(&event)?;

    validate_message(title, body).map_err(anyhow::Error::msg)?;

    println!("Pull request title and body follow repository conventions");

    Ok(())
}

fn pull_request_message(event: &JsonValue) -> anyhow::Result<(&str, Option<&str>)> {
    let event = json_object(event, "GitHub event")?;
    let pull_request = event
        .get("pull_request")
        .context("GitHub event is missing `pull_request`")?;
    let pull_request = json_object(pull_request, "`pull_request`")?;

    let title = pull_request.get("title").context("`pull_request` is missing `title`")?;
    let title = match title {
        JsonValue::String(title) => title.as_str(),
        _ => anyhow::bail!("`pull_request.title` must be a string"),
    };

    let body = match pull_request.get("body").context("`pull_request` is missing `body`")? {
        JsonValue::String(body) => Some(body.as_str()),
        JsonValue::Null => None,
        _ => anyhow::bail!("`pull_request.body` must be a string or null"),
    };

    Ok((title, body))
}

fn json_object<'a>(value: &'a JsonValue, name: &str) -> anyhow::Result<&'a HashMap<String, JsonValue>> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => anyhow::bail!("{name} must be an object"),
    }
}

fn validate_message(title: &str, body: Option<&str>) -> Result<(), String> {
    let (commit_type, breaking_title) = validate_title(title)?;
    let breaking_footer = if let Some(body) = body {
        validate_body(body)?
    } else {
        false
    };

    if commit_type == "refactor" && (breaking_title || breaking_footer) {
        return Err("breaking restructuring must use `fix`, not `refactor`".into());
    }

    Ok(())
}

fn validate_title(title: &str) -> Result<(&str, bool), String> {
    if title.contains(['\r', '\n']) {
        return Err("pull request title must contain exactly one line".into());
    }

    let (prefix, description) = title
        .split_once(": ")
        .ok_or("title must match `<type>[optional scope][!]: <description>`")?;

    if description.is_empty() || description.trim() != description {
        return Err("title description must be non-empty without surrounding whitespace".into());
    }

    let breaking = prefix.ends_with('!');
    let prefix = prefix.strip_suffix('!').unwrap_or(prefix);
    let (commit_type, scope) = if let Some((commit_type, scope)) = prefix.split_once('(') {
        let scope = scope
            .strip_suffix(')')
            .ok_or("title scope must end with `)` before the optional `!`")?;

        if commit_type.is_empty() || scope.is_empty() || scope.contains(['(', ')']) {
            return Err("title must contain at most one non-empty scope".into());
        }

        (commit_type, Some(scope))
    } else {
        if prefix.contains(')') {
            return Err("title scope must start with `(`".into());
        }

        (prefix, None)
    };

    if !ALLOWED_TYPES.contains(&commit_type) {
        return Err(format!(
            "unsupported title type `{commit_type}`; expected one of: {}",
            ALLOWED_TYPES.join(", ")
        ));
    }

    if commit_type == "test" {
        if let Some(scope) = scope
            && !["core", "extra"].contains(&scope)
        {
            return Err("`test` titles may use only the `core` or `extra` scope".into());
        }
    } else if let Some(scope) = scope {
        if !CANONICAL_SCOPES.contains(&scope) {
            return Err(format!("unsupported title scope `{scope}`"));
        }
    }

    Ok((commit_type, breaking))
}

fn validate_body(body: &str) -> Result<bool, String> {
    if body.trim().is_empty() {
        return Ok(false);
    }

    let body = if body.contains('\r') {
        let body = body.replace("\r\n", "\n");
        if body.contains('\r') {
            return Err("pull request body must not contain bare carriage returns".into());
        }
        Cow::Owned(body)
    } else {
        Cow::Borrowed(body)
    };

    if body.lines().any(is_checklist_item) {
        return Err("pull request body must not contain checklist items".into());
    }

    let mut saw_footer = false;
    let mut breaking = false;

    for paragraph in body.split("\n\n") {
        if paragraph.is_empty() {
            continue;
        }

        let mut footer_lines = 0;
        let mut known_footer = false;
        let mut content_lines = 0;

        for line in paragraph.lines() {
            if line.is_empty() {
                continue;
            }

            if malformed_breaking_change(line) {
                return Err(
                    "breaking-change footer must use `BREAKING CHANGE: <description>` or `BREAKING-CHANGE: <description>`"
                        .into(),
                );
            }

            if let Some(token) = footer_token(line)? {
                footer_lines += 1;
                known_footer |= is_known_footer(token);
                breaking |= matches!(token, "BREAKING CHANGE" | "BREAKING-CHANGE");
            } else {
                content_lines += 1;
            }
        }

        if known_footer && content_lines > 0 {
            return Err("commit body and footers must be separated by a blank line".into());
        }

        if footer_lines > 0 && (known_footer || saw_footer) {
            saw_footer = true;
        } else if saw_footer && content_lines > 0 {
            return Err("commit body content must not follow a footer block".into());
        }
    }

    Ok(breaking)
}

fn is_checklist_item(line: &str) -> bool {
    let line = line.trim_start();
    ["- [ ]", "- [x]", "- [X]", "* [ ]", "* [x]", "* [X]"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

fn malformed_breaking_change(line: &str) -> bool {
    ["BREAKING CHANGE", "BREAKING-CHANGE"].iter().any(|token| {
        line.strip_prefix(token)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(':') && !suffix.starts_with(": "))
    })
}

fn footer_token(line: &str) -> Result<Option<&str>, String> {
    let (token, value) = if let Some((token, value)) = line.split_once(": ") {
        (token, value)
    } else if let Some((token, value)) = line.split_once(" #") {
        (token, value)
    } else {
        return Ok(None);
    };

    let valid_token = !token.is_empty()
        && (token == "BREAKING CHANGE"
            || token
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-'));

    if !valid_token {
        return Ok(None);
    }

    if value.trim().is_empty() {
        return if is_known_footer(token) {
            Err(format!("footer `{token}` must have a value"))
        } else {
            Ok(None)
        };
    }

    Ok(Some(token))
}

fn is_known_footer(token: &str) -> bool {
    matches!(
        token,
        "BREAKING CHANGE" | "BREAKING-CHANGE" | "Issue" | "Co-authored-by"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_allowed_type_without_scope() {
        for commit_type in ALLOWED_TYPES {
            let title = format!("{commit_type}: describe resulting behavior");
            assert_eq!(validate_message(&title, None), Ok(()), "{title}");
        }
    }

    #[test]
    fn accepts_every_canonical_scope() {
        for scope in CANONICAL_SCOPES {
            let title = format!("fix({scope}): describe resulting behavior");
            assert_eq!(validate_message(&title, None), Ok(()), "{title}");
        }
    }

    #[test]
    fn accepts_test_scope_exceptions() {
        for title in [
            "test: cover repository behavior",
            "test(core): fuzz foundational parsers",
            "test(extra): exercise integration infrastructure",
        ] {
            assert_eq!(validate_message(title, None), Ok(()), "{title}");
        }
    }

    #[test]
    fn rejects_invalid_and_intersecting_scopes() {
        for title in [
            "fix(extra): invalid non-test exception",
            "test(pdu): component scopes are forbidden for tests",
            "fix(core,extra): intersecting scopes",
            "fix(core/extra): intersecting scopes",
            "fix(protocol): non-canonical scope",
            "fix((core)): nested scope",
        ] {
            assert!(validate_message(title, None).is_err(), "{title}");
        }
    }

    #[test]
    fn validates_title_syntax() {
        for title in [
            "bug(core): unsupported type",
            "fix(core) missing separator",
            "fix(): empty scope",
            "fix(core: malformed scope",
            "fix: ",
            "fix: leading or trailing whitespace ",
            "fix: first line\nsecond line",
        ] {
            assert!(validate_message(title, None).is_err(), "{title}");
        }
    }

    #[test]
    fn accepts_breaking_changes() {
        for (title, body) in [
            ("feat(core)!: replace parser contract", None),
            (
                "feat(core): replace parser contract",
                Some("Explain the new behavior.\n\nBREAKING CHANGE: callers must pass a context"),
            ),
            (
                "feat!: replace parser contract",
                Some("BREAKING-CHANGE: callers must pass a context"),
            ),
        ] {
            assert_eq!(validate_message(title, body), Ok(()), "{title}");
        }
    }

    #[test]
    fn rejects_breaking_refactors() {
        for (title, body) in [
            ("refactor!: replace parser contract", None),
            ("refactor(core)!: replace parser contract", None),
            (
                "refactor(core): replace parser contract",
                Some("BREAKING CHANGE: callers must pass a context"),
            ),
        ] {
            assert!(validate_message(title, body).is_err(), "{title}");
        }
    }

    #[test]
    fn validates_body_and_footer_formatting() {
        for body in [
            "Explain the motivation.\n\nIssue: PROJECT-123",
            "Explain the motivation.\n\nIssue #123",
            "Explain the motivation.\n\nIssue: PROJECT-123\nCo-authored-by: A User <user@example.com>",
            "Motivation: preserve existing behavior.\nThe new check remains narrowly scoped.",
            "Motivation: preserve existing behavior.\n\nThe new check remains narrowly scoped.",
            "BREAKING CHANGE was avoided by preserving the existing API.",
            "Explain the motivation.\r\n\r\nIssue: PROJECT-123",
            "Explain the motivation.\n\n\nIssue: PROJECT-123",
            "Issue: PROJECT-123",
        ] {
            assert_eq!(
                validate_message("fix(core): preserve behavior", Some(body)),
                Ok(()),
                "{body}"
            );
        }

        for body in [
            "Explain the motivation.\nIssue: PROJECT-123",
            "Issue: PROJECT-123\nMore body text.",
            "BREAKING CHANGE:",
            "BREAKING CHANGE: ",
            "Issue: ",
            "Co-authored-by: ",
            "- [ ] Run tests",
        ] {
            assert!(
                validate_message("fix(core): preserve behavior", Some(body)).is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn accepts_empty_and_null_bodies() {
        assert_eq!(validate_message("ci: validate metadata", None), Ok(()));
        assert_eq!(validate_message("ci: validate metadata", Some("")), Ok(()));
        assert_eq!(validate_message("ci: validate metadata", Some(" \n\t")), Ok(()));
    }

    #[test]
    fn extracts_null_and_hostile_metadata_as_data() {
        let null_body: JsonValue = r#"{"pull_request":{"title":"ci(pr-automation): validate metadata","body":null}}"#
            .parse()
            .expect("valid fixture");
        assert_eq!(
            pull_request_message(&null_body).expect("valid event"),
            ("ci(pr-automation): validate metadata", None)
        );

        let hostile: JsonValue = r#"{"pull_request":{"title":"ci(pr-automation): preserve `$()`, quotes, and ;","body":"Why: retain `$(touch nope)`; && | > < \"quotes\".\n\nIssue: #123"}}"#
            .parse()
            .expect("valid fixture");
        let (title, body) = pull_request_message(&hostile).expect("valid event");
        assert_eq!(validate_message(title, body), Ok(()));
    }

    #[test]
    fn rejects_malformed_event_payloads() {
        for event in [
            "{}",
            r#"{"pull_request":null}"#,
            r#"{"pull_request":{"title":null,"body":null}}"#,
            r#"{"pull_request":{"title":"ci: valid","body":[]}}"#,
        ] {
            let event = event.parse().expect("valid JSON fixture");
            assert!(pull_request_message(&event).is_err(), "{event:?}");
        }
    }
}

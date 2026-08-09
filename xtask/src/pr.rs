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
    let paragraphs = body.split("\n\n").filter(|paragraph| !paragraph.is_empty());
    let paragraph_count = paragraphs.clone().count();

    for (paragraph_index, paragraph) in paragraphs.enumerate() {
        let final_paragraph = paragraph_index + 1 == paragraph_count;
        let mut footer_lines = 0;
        let mut known_footer = false;
        let mut content_lines = 0;
        let mut footer_started = false;

        for line in paragraph.lines() {
            if line.is_empty() {
                continue;
            }

            if let Some(token) = malformed_footer(line) {
                return Err(format!("footer `{token}` must use `: <value>` or ` #<value>`"));
            }

            if let Some(token) = footer_token(line)? {
                let known = is_known_footer(token);
                if known || footer_started || saw_footer || final_paragraph {
                    footer_lines += 1;
                    known_footer |= known;
                    footer_started = true;
                    breaking |= matches!(token, "BREAKING CHANGE" | "BREAKING-CHANGE");
                } else {
                    content_lines += 1;
                }
            } else if footer_started {
                continue;
            } else {
                content_lines += 1;
            }
        }

        if known_footer && content_lines > 0 {
            return Err("commit body and footers must be separated by a blank line".into());
        }

        if footer_lines > 0 {
            saw_footer = true;
        } else if saw_footer && content_lines > 0 {
            return Err("commit body content must not follow a footer block".into());
        }
    }

    Ok(breaking)
}

fn is_checklist_item(line: &str) -> bool {
    let line = line.trim_start();
    let item = ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
        .or_else(|| {
            let (marker, item) = line.split_once(". ").or_else(|| line.split_once(") "))?;
            (!marker.is_empty() && marker.chars().all(|character| character.is_ascii_digit())).then_some(item)
        });

    item.is_some_and(|item| {
        ["[ ]", "[x]", "[X]"].iter().any(|checkbox| {
            item.strip_prefix(checkbox)
                .is_some_and(|rest| rest.is_empty() || rest.as_bytes()[0].is_ascii_whitespace())
        })
    })
}

fn malformed_footer(line: &str) -> Option<&'static str> {
    const KNOWN_FOOTERS: &[&str] = &["BREAKING CHANGE", "BREAKING-CHANGE", "Issue", "Co-authored-by"];

    KNOWN_FOOTERS.iter().copied().find(|token| {
        line.strip_prefix(token).is_some_and(|suffix| {
            if suffix.is_empty() && matches!(*token, "BREAKING CHANGE" | "BREAKING-CHANGE") {
                return true;
            }

            let trimmed = suffix.trim_start();
            let malformed_colon = trimmed.starts_with(':') && !suffix.starts_with(": ");
            let malformed_hash = trimmed.starts_with('#') && !suffix.starts_with(" #");
            let forbidden_breaking_hash =
                matches!(*token, "BREAKING CHANGE" | "BREAKING-CHANGE") && suffix.starts_with(" #");

            malformed_colon || malformed_hash || forbidden_breaking_hash
        })
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

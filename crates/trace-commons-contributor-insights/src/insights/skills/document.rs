use super::{MAX_SKILL_BYTES, MAX_SKILL_DESCRIPTION_BYTES, MAX_SKILL_NAME_BYTES};

/// Parse the common scalar subset of SKILL.md YAML frontmatter. Quoted scalars,
/// folded/literal blocks, CRLF and wrapped descriptions are supported. Ambiguous
/// YAML structures are rejected rather than displayed as invented metadata.
pub fn parse_skill_document(bytes: &[u8]) -> Option<(String, String)> {
    if bytes.len() as u64 > MAX_SKILL_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes)
        .ok()?
        .trim_start_matches('\u{feff}');
    let lines: Vec<_> = text.lines().collect();
    if lines.first()?.trim() != "---" {
        return None;
    }
    let end = lines.iter().skip(1).position(|l| *l == "---")? + 1;
    let mut name = None;
    let mut description = None;
    let mut index = 1;
    while index < end {
        let line = lines[index];
        index += 1;
        if line.starts_with(char::is_whitespace) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once(':') else {
            continue;
        };
        if key != "name" && key != "description" {
            continue;
        }
        let mut raw = raw.trim().to_string();
        let block = matches!(raw.as_str(), ">" | ">-" | ">+" | "|" | "|-" | "|+");
        let mut continuation = Vec::new();
        while index < end
            && (lines[index].starts_with(char::is_whitespace) || lines[index].is_empty())
        {
            continuation.push(lines[index]);
            index += 1;
        }
        let value = if block {
            let indent = continuation
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.len() - l.trim_start().len())
                .min()
                .unwrap_or(0);
            let content: Vec<_> = continuation
                .iter()
                .map(|l| l.get(indent..).unwrap_or(""))
                .collect();
            if raw.starts_with('>') {
                content.join(" ")
            } else {
                content.join("\n")
            }
        } else {
            if !continuation.is_empty() {
                raw.push(' ');
                raw.push_str(
                    &continuation
                        .iter()
                        .map(|s| s.trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            scalar(&raw)?
        };
        let value = value.trim().to_string();
        if value.is_empty()
            || value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return None;
        }
        let metadata_limit = if key == "name" {
            MAX_SKILL_NAME_BYTES
        } else {
            MAX_SKILL_DESCRIPTION_BYTES
        };
        if value.len() > metadata_limit {
            return None;
        }
        let slot = if key == "name" {
            &mut name
        } else {
            &mut description
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    Some((name?, description?))
}

fn scalar(value: &str) -> Option<String> {
    let value = if value.starts_with(['\'', '\"']) {
        let quote = value.as_bytes()[0];
        let bytes = value.as_bytes();
        let mut index = 1;
        let mut end = None;
        while index < bytes.len() {
            if quote == b'"' && bytes[index] == b'\\' {
                index += 2;
                continue;
            }
            if bytes[index] == quote {
                if quote == b'\'' && bytes.get(index + 1) == Some(&quote) {
                    index += 2;
                    continue;
                }
                end = Some(index + 1);
                break;
            }
            index += 1;
        }
        let end = end?;
        let trailing = value[end..].trim();
        if !trailing.is_empty() && !trailing.starts_with('#') {
            return None;
        }
        &value[..end]
    } else {
        value
    };

    if value.starts_with('"') {
        // JSON strings are a strict useful subset of YAML double-quoted scalars.
        serde_json::from_str(value).ok()
    } else if value.starts_with('\'') {
        let inner = value.strip_prefix('\'')?.strip_suffix('\'')?;
        let mut chars = inner.chars().peekable();
        let mut out = String::new();
        while let Some(c) = chars.next() {
            if c == '\'' && chars.next() != Some('\'') {
                return None;
            }
            out.push(c);
        }
        Some(out)
    } else if value.starts_with(['[', '{', '&', '*', '!', '|', '>']) {
        None
    } else {
        Some(value.split(" #").next()?.trim().to_string())
    }
}

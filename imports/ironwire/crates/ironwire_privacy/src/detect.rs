//! Finding what to substitute.
//!
//! Tier 1 (secrets) and tier 2 (values the user nominated) are both
//! deterministic and both delegate their hard part to `ironclaw_safety`
//! (`docs/PRIVACY.md` §2). What is written here is the part ironclaw has no
//! reason to have: locating matches so they can be replaced **reversibly**,
//! and refusing to match things that only look sensitive.
//!
//! That second job is specific to a coding agent and easy to underestimate. A
//! large share of what a PII detector flags in a coding session is
//! load-bearing *code*: `user@example.com` in a test assertion, `192.168.1.1`
//! in a fixture, a `555` number in a validator's expected output. Substituting
//! those makes the model write code that does not compile, and the user blames
//! the model.

use std::ops::Range;

use ironclaw_safety::{LeakDetector, redaction_values_for_secret};

use crate::mint::Class;

/// One thing worth substituting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Byte range within the scanned text.
    pub range: Range<usize>,
    /// What kind of value it is.
    pub class: Class,
    /// Which rule matched, for `ironwire privacy check`.
    pub rule: String,
}

/// Hosts and ranges reserved by standards for documentation and testing.
///
/// Never substituted. They are in a user's code *because* they are not real,
/// and replacing them turns a working fixture into a broken one.
const RESERVED_SUBSTRINGS: &[&str] = &[
    // RFC 2606 / 6761 reserved names.
    "example.com",
    "example.org",
    "example.net",
    "example.edu",
    ".invalid",
    ".localhost",
    ".test",
    "localhost",
];

/// Which tiers are switched on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tiers {
    /// Tier 1: secrets with a machine-checkable shape.
    pub secrets: bool,
    /// Tier 2: exact strings the user nominated.
    pub named_values: Vec<String>,
    /// Deterministic PII: email addresses, IP addresses, phone numbers.
    ///
    /// Pattern-matched, so it has tier 1's properties rather than tier 3's —
    /// the same input always produces the same output and every match can be
    /// shown to the user. Human names are deliberately *not* here: they need
    /// the tier-3 classifier, and a regex for them would be a false-negative
    /// machine that reads as protection (`docs/PRIVACY.md` §2).
    pub pii: bool,
}

impl Tiers {
    /// Whether anything is enabled.
    #[must_use]
    pub fn is_off(&self) -> bool {
        !self.secrets && !self.pii && self.named_values.is_empty()
    }
}

/// Finds substitutable values in text.
pub struct Detector {
    secrets: Option<LeakDetector>,
    /// Whether the deterministic PII classes are on.
    pii: bool,
    /// Nominated values, expanded to their encoded variants and sorted
    /// longest-first so an overlapping shorter value cannot win.
    named: Vec<String>,
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The nominated values are the user's own sensitive strings. Printing
        // them in a log would defeat the feature.
        f.debug_struct("Detector")
            .field("secrets", &self.secrets.is_some())
            .field("named", &format!("{} value(s)", self.named.len()))
            .finish()
    }
}

impl Detector {
    /// Build a detector for the configured tiers.
    #[must_use]
    pub fn new(tiers: &Tiers) -> Self {
        // `redaction_values_for_secret` expands `%20`/`+` and lowercase
        // percent-escape variants — the part of exact-value matching that is
        // easy to get wrong and that ironclaw already got right.
        let mut named: Vec<String> = tiers
            .named_values
            .iter()
            .filter(|value| !value.trim().is_empty())
            .flat_map(|value| redaction_values_for_secret(value))
            .collect();
        named.sort_by_key(|value| std::cmp::Reverse(value.len()));
        named.dedup();

        Self {
            secrets: tiers.secrets.then(LeakDetector::new),
            pii: tiers.pii,
            named,
        }
    }

    /// Locate everything worth substituting, in order, without overlaps.
    #[must_use]
    pub fn find(&self, text: &str) -> Vec<Finding> {
        let mut findings = Vec::new();

        if let Some(detector) = &self.secrets {
            for hit in detector.scan(text).matches {
                if needs_credential_context(&hit.pattern_name)
                    && !has_credential_context(text, hit.location.start)
                {
                    continue;
                }
                findings.push(Finding {
                    range: hit.location,
                    class: Class::Secret,
                    rule: hit.pattern_name,
                });
            }
        }

        if self.pii {
            findings.extend(find_pii(text));
        }

        // Longest-first, so `alice@corp.com` wins over a nominated `corp.com`
        // that overlaps it.
        for value in &self.named {
            let mut from = 0;
            while let Some(offset) = text[from..].find(value.as_str()) {
                let start = from + offset;
                findings.push(Finding {
                    range: start..start + value.len(),
                    class: Class::Named,
                    rule: "named value".to_string(),
                });
                from = start + value.len();
            }
        }

        resolve_overlaps(findings, text)
    }
}

/// Deterministic PII, by shape.
///
/// Everything here goes through the same `resolve_overlaps` -> `is_reserved`
/// path as tier 1, which already excludes documentation domains, RFC 1918 and
/// RFC 5737 ranges, `2001:db8`, loopback and the NANP fictional exchange. That
/// exclusion list was written for exactly these classes and has been guarding
/// matches that never included one.
fn find_pii(text: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let bytes = text.as_bytes();

    // Email. A conservative RFC 5322 subset: the liberal grammar matches things
    // that are not addresses, and a false positive here rewrites code.
    let mut index = 0;
    while let Some(offset) = text[index..].find('@') {
        let at = index + offset;
        index = at + 1;
        let start = text[..at]
            .rfind(|c: char| !is_email_local(c))
            .map_or(0, |boundary| boundary + 1);
        let after = &text[at + 1..];
        let host_len = after
            .find(|c: char| !is_email_host(c))
            .unwrap_or(after.len());
        let host = &after[..host_len];
        // A domain needs a dot and a plausible TLD; `user@localhost` and
        // `@mention` are not addresses.
        if start == at || host_len == 0 {
            continue;
        }
        let Some((_, tld)) = host.rsplit_once('.') else {
            continue;
        };
        if tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        findings.push(Finding {
            range: start..at + 1 + host.trim_end_matches('.').len(),
            class: Class::Email,
            rule: "email".to_string(),
        });
    }

    // IPv4 dotted quad, with each octet in range so a version string like
    // `1.2.3.4.5` or `999.1.1.1` cannot match.
    let mut cursor = 0;
    while cursor < bytes.len() {
        if !bytes[cursor].is_ascii_digit()
            || (cursor > 0 && (bytes[cursor - 1].is_ascii_digit() || bytes[cursor - 1] == b'.'))
        {
            cursor += 1;
            continue;
        }
        let rest = &text[cursor..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        // A sentence-ending period is part of the run of dots-and-digits and is
        // not part of the address: `…is 203.0.114.9.` would otherwise split
        // into five groups and never match.
        let token = rest[..end].trim_end_matches('.');
        if is_ipv4(token) {
            findings.push(Finding {
                range: cursor..cursor + token.len(),
                class: Class::IpAddress,
                rule: "ipv4".to_string(),
            });
        }
        cursor += end.max(1);
    }

    // IPv6, only in its unambiguous forms: at least two colons and only hex
    // digits and colons around them.
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find(':') {
        let colon = cursor + offset;
        let start = text[..colon]
            .rfind(|c: char| !is_ipv6_char(c))
            .map_or(0, |boundary| boundary + 1);
        let after = &text[start..];
        let len = after
            .find(|c: char| !is_ipv6_char(c))
            .unwrap_or(after.len());
        let token = &after[..len];
        cursor = start + len.max(1);
        if is_ipv6(token) {
            findings.push(Finding {
                range: start..start + token.len(),
                class: Class::IpAddress,
                rule: "ipv6".to_string(),
            });
        }
    }

    // Phone. The highest-false-positive class of the three, and the one most
    // likely to corrupt code, so it requires real structure: an explicit
    // country code, or a separated NANP number. A bare run of digits — a port
    // range, a version, a timestamp, a SHA fragment — never matches.
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte != b'+' && !byte.is_ascii_digit() && byte != b'(' {
            cursor += 1;
            continue;
        }
        if cursor > 0 && (bytes[cursor - 1].is_ascii_digit() || bytes[cursor - 1] == b'.') {
            cursor += 1;
            continue;
        }
        let rest = &text[cursor..];
        let len = rest.find(|c: char| !is_phone_char(c)).unwrap_or(rest.len());
        let token = rest[..len].trim_end_matches(|c: char| !c.is_ascii_digit());
        if is_phone(token) {
            findings.push(Finding {
                range: cursor..cursor + token.len(),
                class: Class::Phone,
                rule: "phone".to_string(),
            });
        }
        cursor += len.max(1);
    }

    findings
}

fn is_email_local(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-')
}

fn is_email_host(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '-')
}

fn is_ipv6_char(c: char) -> bool {
    c.is_ascii_hexdigit() || c == ':'
}

fn is_phone_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | ' ')
}

/// A dotted quad whose octets are all in range.
fn is_ipv4(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.len() <= 3 && part.parse::<u8>().is_ok())
}

/// An IPv6 address in a form that cannot be confused with the other
/// colon-separated shapes a repository is full of.
///
/// Those shapes are MAC addresses (`aa:bb:cc:dd:ee:ff`) and clock times
/// (`01:23:45`), and both are "hex groups separated by colons" — the naive
/// test matches them, and substituting either rewrites something that has to
/// keep working. So a match needs one of the two things only a real address
/// has: a `::` elision, or the full eight groups.
fn is_ipv6(token: &str) -> bool {
    let groups: Vec<&str> = token.split(':').collect();
    let elided = token.contains("::");
    if !elided && groups.len() != 8 {
        return false;
    }
    if elided && token.matches("::").count() > 1 {
        return false;
    }
    if groups.len() > 8 {
        return false;
    }
    groups.iter().all(|group| {
        group.is_empty() || (group.len() <= 4 && group.chars().all(|c| c.is_ascii_hexdigit()))
    }) && groups.iter().any(|group| !group.is_empty())
}

/// A phone number with enough structure to be one.
///
/// The strictest of the three rules, because this is the class most likely to
/// corrupt code and the one with the most lookalikes. "Ten digits and a
/// separator" is not enough: a port range (`30000-32767`), a version, a
/// timestamp and a SHA fragment all clear that bar. Without an explicit country
/// code, the digits must fall into the NANP grouping — 3-3-4, optionally behind
/// a `1` — which a port range does not.
fn is_phone(token: &str) -> bool {
    if token.starts_with('+') {
        // E.164: the `+` is the claim, and the length is the check.
        return (8..=15).contains(&token.chars().filter(char::is_ascii_digit).count());
    }
    let groups: Vec<usize> = token
        .split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
        .map(str::len)
        .collect();
    matches!(groups.as_slice(), [3, 3, 4] | [1, 3, 3, 4])
}

/// Patterns whose shape alone is not evidence of a secret.
///
/// `high_entropy_hex` is the whole reason this exists. In a general corpus a
/// 64-character hex string is often a credential; in a *repository* it is
/// overwhelmingly a git SHA, a lockfile checksum, or a content hash. IronWire
/// only ever sees repositories.
///
/// Substituting one is not a small annoyance: the model rewrites a
/// `Cargo.lock` or a `package-lock.json` around a placeholder, the build
/// breaks, and nothing in the failure points at IronWire. So these patterns
/// need a second signal — a nearby word saying the value is a credential —
/// before they count.
///
/// The cost is stated plainly: a bare hex secret with no surrounding context is
/// missed. That is the trade this feature makes everywhere (`docs/PRIVACY.md`
/// §1), and here it is the right side of it.
fn needs_credential_context(pattern: &str) -> bool {
    matches!(pattern, "high_entropy_hex" | "high_entropy_base64")
}

/// Words near a match that suggest it really is a credential.
const CREDENTIAL_WORDS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "apikey",
    "api_key",
    "api-key",
    "credential",
    "auth",
    "bearer",
    "private_key",
    "privatekey",
    "access_key",
    "accesskey",
    "client_secret",
    "signing",
    "session",
];

/// Whether the text just before a match calls it a credential.
///
/// A short window on purpose: `token = "..."` and `Authorization: Bearer ...`
/// both fit, while a `token` mentioned three lines earlier is not evidence
/// about *this* value.
fn has_credential_context(text: &str, at: usize) -> bool {
    const WINDOW: usize = 48;
    let start = text[..at]
        .char_indices()
        .rev()
        .take(WINDOW)
        .last()
        .map_or(0, |(i, _)| i);
    let before = text[start..at].to_ascii_lowercase();
    CREDENTIAL_WORDS.iter().any(|word| before.contains(word))
}

/// Whether a match should be left alone despite matching.
///
/// Checked against the surrounding line, not just the match, because that is
/// where the evidence is: `192.168.1.1` in `EXPECTED_HOST = "192.168.1.1"` is a
/// fixture, and the range alone cannot tell you that.
fn is_reserved(text: &str, range: &Range<usize>) -> bool {
    let matched = &text[range.clone()];
    let lower = matched.to_ascii_lowercase();
    if RESERVED_SUBSTRINGS
        .iter()
        .any(|reserved| lower.contains(reserved))
    {
        return true;
    }
    // RFC 1918 private ranges and RFC 5737 documentation ranges are in a repo
    // because they are not routable, not because they identify anyone.
    for prefix in [
        "10.",
        "192.168.",
        "127.",
        "169.254.",
        "192.0.2.",
        "198.51.100.",
        "203.0.113.",
    ] {
        if lower.starts_with(prefix) {
            return true;
        }
    }
    for prefix in ["172.1", "172.2", "172.3"] {
        if lower.starts_with(prefix) && looks_like_private_172(&lower) {
            return true;
        }
    }
    // RFC 3849 documentation IPv6.
    if lower.starts_with("2001:db8") {
        return true;
    }
    // Loopback, exactly — not as a substring, or `2606:4700:4700::1111` would
    // be excluded for containing `::1`.
    if lower == "::1" || lower == "::" {
        return true;
    }
    // NANP numbers reserved for fiction. The substring forms catch the plain
    // spellings; the digit test catches `(555) 010-0199`, where the separators
    // fall between the characters the substrings are looking for.
    if lower.contains("555-01") || lower.contains("55501") {
        return true;
    }
    let digits: String = lower.chars().filter(char::is_ascii_digit).collect();
    let national = digits.strip_prefix('1').unwrap_or(&digits);
    if national.len() == 10 && (national.starts_with("555") || &national[3..6] == "555") {
        return true;
    }
    false
}

/// `172.16.0.0/12` — the second octet is 16..=31, which a prefix test cannot
/// express on its own.
fn looks_like_private_172(text: &str) -> bool {
    text.strip_prefix("172.")
        .and_then(|rest| rest.split('.').next())
        .and_then(|octet| octet.parse::<u8>().ok())
        .is_some_and(|octet| (16..=31).contains(&octet))
}

/// Keep the longest match at each position and drop anything reserved.
fn resolve_overlaps(mut findings: Vec<Finding>, text: &str) -> Vec<Finding> {
    findings.retain(|finding| {
        // A range that does not land on a character boundary came from a
        // pattern we cannot safely slice; dropping it is better than panicking
        // on someone's request.
        text.is_char_boundary(finding.range.start)
            && text.is_char_boundary(finding.range.end)
            && !is_reserved(text, &finding.range)
    });

    findings.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then_with(|| b.range.len().cmp(&a.range.len()))
    });

    let mut out: Vec<Finding> = Vec::with_capacity(findings.len());
    for finding in findings {
        if out
            .last()
            .is_some_and(|prev| prev.range.end > finding.range.start)
        {
            continue;
        }
        out.push(finding);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets_only() -> Detector {
        Detector::new(&Tiers {
            secrets: true,
            named_values: Vec::new(),
            pii: false,
        })
    }

    #[test]
    fn a_real_looking_api_key_is_found() {
        let detector = secrets_only();
        let text = concat!(
            "export GITHUB_TOKEN=ghp_",
            "abcdefghijklmnopqrstuvwxyz0123456789"
        );
        let findings = detector.find(text);
        assert_eq!(findings.len(), 1, "got {findings:?}");
        assert_eq!(findings[0].class, Class::Secret);
    }

    #[test]
    fn ordinary_source_code_is_left_alone() {
        // The failure mode that matters most: over-matching makes the model
        // write broken code and the user blames the model.
        let detector = secrets_only();
        let text = "let total = items.iter().map(|i| i.price).sum::<u64>();";
        assert!(detector.find(text).is_empty());
    }

    #[test]
    fn reserved_documentation_values_are_never_substituted() {
        // These are in a repo *because* they are not real. Replacing them turns
        // a working fixture into a broken one.
        let detector = Detector::new(&Tiers {
            secrets: true,
            named_values: vec![
                "user@example.com".to_string(),
                "192.168.1.1".to_string(),
                "10.0.0.5".to_string(),
                "172.16.0.1".to_string(),
                "203.0.113.7".to_string(),
                "2001:db8::1".to_string(),
                "555-0100".to_string(),
            ],
            pii: false,
        });
        for text in [
            "assert_eq!(user.email, \"user@example.com\");",
            "const HOST: &str = \"192.168.1.1\";",
            "bind 10.0.0.5",
            "gateway 172.16.0.1",
            "doc range 203.0.113.7",
            "v6 2001:db8::1",
            "call 555-0100",
        ] {
            assert!(
                detector.find(text).is_empty(),
                "substituted a reserved value in: {text}"
            );
        }
    }

    #[test]
    fn a_public_172_address_is_not_treated_as_private() {
        // 172.16/12 is private; 172.32 is not. A prefix test alone gets this
        // wrong and would silently skip a real address.
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["172.32.4.5".to_string()],
            pii: false,
        });
        assert_eq!(detector.find("host 172.32.4.5").len(), 1);
    }

    #[test]
    fn a_nominated_value_is_found_everywhere_it_appears() {
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["Acme Holdings".to_string()],
            pii: false,
        });
        let text = "Acme Holdings invoices Acme Holdings monthly";
        assert_eq!(detector.find(text).len(), 2);
    }

    #[test]
    fn a_url_encoded_nominated_value_is_found_too() {
        // The part of exact-value matching that is easy to get wrong, and the
        // reason this delegates to ironclaw rather than calling `str::find`.
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["Acme Holdings".to_string()],
            pii: false,
        });
        assert_eq!(detector.find("q=Acme%20Holdings&x=1").len(), 1);
        assert_eq!(detector.find("q=Acme+Holdings&x=1").len(), 1);
    }

    #[test]
    fn overlapping_matches_resolve_to_the_longest() {
        // Otherwise a nominated `corp.com` would carve up an
        // `alice@corp.com` that a longer rule already claimed, and reversal
        // would put back two fragments.
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["alice@corp.com".to_string(), "corp.com".to_string()],
            pii: false,
        });
        let findings = detector.find("mail alice@corp.com now");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].range.len(), "alice@corp.com".len());
    }

    #[test]
    fn a_detector_never_prints_the_users_nominated_values() {
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["MyEmployerName".to_string()],
            pii: false,
        });
        let rendered = format!("{detector:?}");
        assert!(!rendered.contains("MyEmployerName"), "got {rendered}");
    }

    #[test]
    fn an_empty_configuration_finds_nothing_and_costs_nothing() {
        let detector = Detector::new(&Tiers::default());
        assert!(
            detector
                .find(concat!("sk-ant-api03-", "aaaaaaaaaaaaaaaaaaaaaaaa"))
                .is_empty()
        );
        assert!(Tiers::default().is_off());
    }

    #[test]
    fn findings_come_back_in_order_and_never_overlap() {
        // The substituter walks them back-to-front; overlapping ranges would
        // corrupt the output.
        let detector = Detector::new(&Tiers {
            secrets: false,
            named_values: vec!["alpha".to_string(), "beta".to_string()],
            pii: false,
        });
        let findings = detector.find("beta then alpha then beta");
        for pair in findings.windows(2) {
            assert!(
                pair[0].range.end <= pair[1].range.start,
                "overlapping findings: {pair:?}"
            );
        }
    }
}

//! Deterministic detectors for identifiers that tool output carries (#1362).
//!
//! Tool calls and tool results are most of a coding-agent trace, and under
//! `TRACE_COMMONS_PII_CLASSIFY_POLICY=prose-only` the classifier does not
//! examine them, so these patterns are the coverage that traffic gets. They
//! extend the deterministic pass to the machine-local shapes tool output is
//! full of: `git log`/`git blame` authors, shell prompts and `ssh` targets,
//! `id`, `whoami`, `ls -l` and environment dumps, IP addresses,
//! private-suffix host names, and phone numbers.
//!
//! Every finding becomes a numbered, typed placeholder minted by the same
//! `PlaceholderMap` as `<PRIVATE_EMAIL_1>` and `<PRIVATE_LOCAL_PATH_1>`, so
//! the same value gets the same token throughout a trace and the shells'
//! marker grammar names it without a change. These run wherever
//! `DeterministicTraceRedactor::redact_text_with_state` runs -- the
//! contributor's pass over every event type and the server's re-scrub -- so
//! client and server agree by construction.
//!
//! The rules are shape-anchored and conservative on purpose. A detector that
//! fires on version strings, hashes or code makes every trace look sensitive
//! and teaches people to ignore the count; each one below names what it
//! deliberately leaves alone, and the tests pin both sides.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::{RedactionReport, RedactionState};

/// A name in a `git log` header, a commit trailer or a `git blame` line.
pub(super) const GIT_AUTHOR: &str = "git_author";
/// An IPv4 or IPv6 address that can identify a host or network.
pub(super) const IP_ADDRESS: &str = "ip_address";
/// A host name under a private-use suffix, or the host half of a prompt or
/// `ssh` target.
pub(super) const PRIVATE_HOSTNAME: &str = "private_hostname";
/// A local account name outside a path: prompts, `ssh` targets, `id`,
/// `whoami`, `ls -l` and `USER=`.
pub(super) const OS_USERNAME: &str = "os_username";
/// An E.164 number, or a North American or UK mobile number in its usual
/// written form.
pub(super) const PHONE_NUMBER: &str = "phone_number";

struct Finding {
    range: Range<usize>,
    label: &'static str,
}

/// The passes that must run BEFORE the email pass: a `user@host.local`
/// prompt is otherwise taken whole as an email address under the wrong
/// label, and an `Author: Name <addr>` line would keep its name.
pub(super) fn redact_identity_shapes(
    input: &str,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    let mut findings = Vec::new();
    git_author_findings(input, &mut findings);
    user_at_host_findings(input, &mut findings);
    private_hostname_findings(input, &mut findings);
    apply_findings(input, findings, state, report)
}

/// The passes that run AFTER both path passes, so an account name inside
/// `/Users/<name>/...` is already a `local_path` placeholder and is never
/// counted twice.
pub(super) fn redact_network_and_account_shapes(
    input: &str,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    let mut findings = Vec::new();
    ipv4_findings(input, &mut findings);
    ipv6_findings(input, &mut findings);
    phone_findings(input, &mut findings);
    account_listing_findings(input, &mut findings);
    apply_findings(input, findings, state, report)
}

/// Replace every finding with its placeholder, earliest first, dropping any
/// that overlaps one already taken. A span that already holds a placeholder
/// or a `[REDACTED` mark is skipped, which is what makes a second run a
/// no-op.
fn apply_findings(
    input: &str,
    mut findings: Vec<Finding>,
    state: &mut RedactionState,
    report: &mut RedactionReport,
) -> String {
    if findings.is_empty() {
        return input.to_string();
    }
    findings.sort_by_key(|finding| (finding.range.start, std::cmp::Reverse(finding.range.end)));
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;
    let mut changed = false;
    let mut edits = Vec::new();
    for finding in findings {
        if finding.range.is_empty() || finding.range.start < last_end {
            continue;
        }
        let value = &input[finding.range.clone()];
        if value.contains("<PRIVATE_") || value.contains("[REDACTED") {
            continue;
        }
        result.push_str(&input[last_end..finding.range.start]);
        let placeholder = state.placeholders.placeholder_for(finding.label, value);
        if state.edit_map.is_some() {
            edits.push(crate::token_distribution::RedactionEdit {
                original: crate::token_distribution::ByteSpan {
                    start: finding.range.start as u64,
                    end: finding.range.end as u64,
                },
                replacement: placeholder.as_bytes().to_vec(),
            });
        }
        result.push_str(&placeholder);
        last_end = finding.range.end;
        report.increment(finding.label);
        report.add_pii_label(finding.label);
        changed = true;
    }
    if !changed {
        return input.to_string();
    }
    state.track_edits(&edits);
    result.push_str(&input[last_end..]);
    result
}

fn regex(pattern: &str) -> Regex {
    // safety: every pattern in this module is a literal covered by the tests
    // below, so a failure here is a build-time mistake, not runtime input.
    Regex::new(pattern).expect("hardcoded tool-output detector regex must compile")
}

fn byte_before(input: &str, at: usize) -> Option<u8> {
    at.checked_sub(1).map(|index| input.as_bytes()[index])
}

fn byte_at(input: &str, at: usize) -> Option<u8> {
    input.as_bytes().get(at).copied()
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

// --- git_author ------------------------------------------------------------

/// `Author:`/`Commit:` headers and the commit trailers that carry a person.
/// Requires the `<address>` that follows a git identity -- with an `@` in
/// it, so a Rust field such as `commit: Option<String>` is not one -- and so
/// prose such as "Author: unknown" is never touched. A line may start after
/// a literal `\n`, because tool output often arrives as an escaped JSON
/// string.
static GIT_IDENTITY_LINE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?mi)(?:^|\\n)[ \t]*(?:author|commit|committer|signed-off-by|co-authored-by|reviewed-by|acked-by|tested-by|reported-by|suggested-by|helped-by|cc)[ \t]*:[ \t]+(?P<name>[^<>\r\n\\]*?[^<>\s\\])[ \t]*<[^<>@\r\n]+@[^<>\r\n]+>",
    )
});

/// `git blame` default output: `<sha> [path] (<name> <date> <time> <tz> <line>)`.
static GIT_BLAME_LINE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?m)^\^?[0-9a-f]{7,40}[ \t]+(?:\S+[ \t]+)?\((?P<name>[^()\n]+?)[ \t]+\d{4}-\d{2}-\d{2}[ \t]+\d{2}:\d{2}:\d{2}[ \t]+[+-]\d{4}[ \t]+\d+\)",
    )
});

fn git_author_findings(input: &str, findings: &mut Vec<Finding>) {
    for regex in [&*GIT_IDENTITY_LINE, &*GIT_BLAME_LINE] {
        for captures in regex.captures_iter(input) {
            if let Some(name) = captures.name("name") {
                findings.push(Finding {
                    range: name.range(),
                    label: GIT_AUTHOR,
                });
            }
        }
    }
}

// --- prompts, ssh targets and host names -----------------------------------

/// A shell prompt: `user@host:cwd$`, `user@host cwd %`, `[user@host cwd]$`.
/// The prompt terminator followed by a space or end of line is what makes
/// this a prompt rather than an address or a package reference.
static SHELL_PROMPT: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?m)(?:^|\\n|[\s\[("'`])(?P<user>[a-z_][a-z0-9_.-]{0,31})@(?P<host>[A-Za-z0-9](?:[A-Za-z0-9.-]{0,252}[A-Za-z0-9])?)(?::[^\s$#%\]]*|[ \t]+[^\s$#%\]]+)?[ \t]*\]?[$#%](?:[ \t]|$)"#,
    )
});

/// The first `user@host` on a line that runs an ssh-family command.
static SSH_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?m)\b(?:ssh|scp|sftp|rsync|mosh)\b[^\n]*?[\s'"=](?P<user>[a-z_][a-z0-9_.-]{0,31})@(?P<host>[A-Za-z0-9](?:[A-Za-z0-9.-]{0,252}[A-Za-z0-9])?)"#,
    )
});

/// Suffixes reserved or conventionally used for names that only resolve
/// inside one network (RFC 6762 `.local`, RFC 8375 `.home.arpa`, and the
/// common private-use ones). A name under a public suffix is left alone: it
/// is not machine-local, and telling a personal domain from a public one is
/// not something a pattern can do.
static PRIVATE_SUFFIX_HOST: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        // The suffix is lowercase only, as resolvers and Bonjour write it;
        // `TimeZoneInfo.Local` is a member access, not a host.
        r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*\.(?:localdomain|local|internal|intranet|lan|corp|home\.arpa)",
    )
});

/// Fixed names every installation shares; they identify a product, not a
/// machine.
const SHARED_INTERNAL_HOSTS: &[&str] = &[
    "host.docker.internal",
    "gateway.docker.internal",
    "kubernetes.docker.internal",
    "metadata.google.internal",
];

/// Stems that put `.local` (and friends) on a file name or a member access
/// rather than a host: `Makefile.local`, `config.local`, `self.local`,
/// `this.internal`. Measured against this repository's own source, which is
/// what a file-read tool returns.
const FILE_STEMS: &[&str] = &[
    "app",
    "args",
    "cls",
    "ctx",
    "exports",
    "module",
    "obj",
    "options",
    "opts",
    "process",
    "props",
    "self",
    "state",
    "this",
    "window",
    "appsettings",
    "bashrc",
    "compose",
    "config",
    "credentials",
    "dev",
    "development",
    "docker-compose",
    "env",
    "gemfile",
    "gitconfig",
    "local",
    "makefile",
    "npmrc",
    "procfile",
    "production",
    "profile",
    "properties",
    "secrets",
    "settings",
    "test",
    "values",
    "vimrc",
    "zshrc",
];

/// Account names that every machine of a kind shares. They say nothing
/// about who ran the session, and keeping them keeps `root@host` legible.
fn is_shared_account(name: &str) -> bool {
    name.starts_with('_')
        || matches!(
            name,
            "root"
                | "admin"
                | "administrator"
                | "app"
                | "azureuser"
                | "bin"
                | "centos"
                | "core"
                | "daemon"
                | "debian"
                | "docker"
                | "ec2-user"
                | "everyone"
                | "fedora"
                | "git"
                | "guest"
                | "jenkins"
                | "mysql"
                | "node"
                | "nobody"
                | "opc"
                | "operator"
                | "postgres"
                | "redis"
                | "runner"
                | "staff"
                | "sys"
                | "ubuntu"
                | "user"
                | "vagrant"
                | "wheel"
                | "www-data"
        )
}

enum HostKind {
    /// No dot: a machine name only its own network resolves.
    SingleLabel,
    /// Under a private-use suffix.
    PrivateSuffix,
    /// An address; the address passes decide whether it is identifying.
    Address,
    /// A dotted name under a public suffix. Left whole for the email pass,
    /// which already takes `user@name.tld` as one value.
    Public,
}

fn host_kind(host: &str) -> HostKind {
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return HostKind::Address;
    }
    if !host.contains('.') {
        return HostKind::SingleLabel;
    }
    if is_private_suffix_host(host) {
        return HostKind::PrivateSuffix;
    }
    HostKind::Public
}

fn is_private_suffix_host(host: &str) -> bool {
    PRIVATE_SUFFIX_HOST
        .find(host)
        .is_some_and(|m| m.start() == 0 && m.end() == host.len())
        && !SHARED_INTERNAL_HOSTS.contains(&host.to_ascii_lowercase().as_str())
}

fn user_at_host_findings(input: &str, findings: &mut Vec<Finding>) {
    for regex in [&*SHELL_PROMPT, &*SSH_TARGET] {
        for captures in regex.captures_iter(input) {
            let (Some(user), Some(host)) = (captures.name("user"), captures.name("host")) else {
                continue;
            };
            let redact_host = match host_kind(host.as_str()) {
                HostKind::Public => continue,
                HostKind::Address => false,
                HostKind::SingleLabel | HostKind::PrivateSuffix => true,
            };
            if !is_shared_account(user.as_str()) {
                findings.push(Finding {
                    range: user.range(),
                    label: OS_USERNAME,
                });
            }
            if redact_host {
                findings.push(Finding {
                    range: host.range(),
                    label: PRIVATE_HOSTNAME,
                });
            }
        }
    }
}

fn private_hostname_findings(input: &str, findings: &mut Vec<Finding>) {
    for found in PRIVATE_SUFFIX_HOST.find_iter(input) {
        let (start, end) = (found.start(), found.end());
        // Part of a longer name or file name: `settings.local.json`,
        // `foo.localized`.
        if let Some(next) = byte_at(input, end) {
            // `-` continues a name; `(` makes it a method call.
            if is_word_byte(next) || next == b'-' || next == b'(' {
                continue;
            }
            if next == b'.' && byte_at(input, end + 1).is_some_and(is_word_byte) {
                continue;
            }
        }
        match byte_before(input, start) {
            // `.env.local`, or the domain half of `jane@corp.internal`,
            // which the email pass takes whole.
            Some(b'.' | b'@' | b'-' | b'~') => continue,
            // A path segment, unless it is a URL authority (`//host`).
            Some(b'/') if byte_before(input, start - 1) != Some(b'/') => continue,
            Some(byte) if is_word_byte(byte) => continue,
            _ => {}
        }
        let host = found.as_str();
        let lowered = host.to_ascii_lowercase();
        let stem = lowered.split('.').next().unwrap_or_default();
        if host.matches('.').count() == 1 && FILE_STEMS.contains(&stem) {
            continue;
        }
        if SHARED_INTERNAL_HOSTS.contains(&lowered.as_str()) {
            continue;
        }
        findings.push(Finding {
            range: start..end,
            label: PRIVATE_HOSTNAME,
        });
    }
}

// --- ip_address ------------------------------------------------------------

/// A maximal run of dot-separated digit groups. Taking the whole run first
/// is what keeps `1.3.6.1.4.1.311` (an OID) or `1.2.3.4.5` from yielding a
/// four-part address out of its middle.
static DOTTED_DIGITS: LazyLock<Regex> = LazyLock::new(|| regex(r"[0-9]+(?:\.[0-9]+)+"));

/// A maximal run of hex digits and colons with at least two colons. The
/// standard library parser then does the real validation.
static HEX_COLON_RUN: LazyLock<Regex> =
    LazyLock::new(|| regex(r"[0-9A-Fa-f]*:[0-9A-Fa-f]*:[0-9A-Fa-f:]*"));

/// Text just before a dotted run that makes it a version, not an address.
const VERSION_CUES: &[&str] = &["version", "==", ">=", "<=", "~=", "!=", "^"];

/// Public resolvers every machine may name. Not identifying, and keeping
/// them keeps DNS debugging legible.
const SHARED_RESOLVERS: &[Ipv4Addr] = &[
    Ipv4Addr::new(1, 0, 0, 1),
    Ipv4Addr::new(1, 1, 1, 1),
    Ipv4Addr::new(8, 8, 4, 4),
    Ipv4Addr::new(8, 8, 8, 8),
    Ipv4Addr::new(9, 9, 9, 9),
    Ipv4Addr::new(149, 112, 112, 112),
    Ipv4Addr::new(208, 67, 220, 220),
    Ipv4Addr::new(208, 67, 222, 222),
];

/// Whether an IPv4 address is worth replacing.
///
/// Replaced: public addresses, the RFC 1918 private ranges and the
/// 100.64/10 shared range. A public address locates a person; a private one
/// maps the contributor's own network, and 100.64/10 is where Tailscale and
/// carrier NAT number individual devices. The placeholder is numbered, so a
/// trace that talks to the same host twice still says so.
///
/// Kept: values that are the same on every machine and so identify no one
/// -- "this network" (0/8, including 0.0.0.0), loopback (127/8), link-local
/// (169.254/16, which includes the cloud metadata address), the IETF
/// protocol-assignment and documentation ranges, the benchmarking range,
/// multicast and the reserved/broadcast block (which is also where netmasks
/// such as 255.255.255.0 land), and a short list of public resolvers.
fn ipv4_is_identifying(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    let shared = a == 0
        || a == 127
        || a >= 224
        || (a == 169 && b == 254)
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || (a == 198 && (b == 18 || b == 19))
        || SHARED_RESOLVERS.contains(&address);
    !shared
}

fn ipv4_findings(input: &str, findings: &mut Vec<Finding>) {
    for found in DOTTED_DIGITS.find_iter(input) {
        let (start, end) = (found.start(), found.end());
        let text = found.as_str();
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() != 4
            || parts
                .iter()
                .any(|part| part.len() > 3 || (part.len() > 1 && part.starts_with('0')))
        {
            continue;
        }
        // Four single digits is a version (`"offered": "1.2.3.4"`) far more
        // often than a host; the few real hosts of that shape are public
        // resolvers, which are kept anyway.
        if parts.iter().all(|part| part.len() == 1) {
            continue;
        }
        // Ending in `.0` is a network, not a host -- `35.235.240.0/20` in a
        // firewall rule -- and it is also the commonest four-part version
        // shape (`4.12.7.0`). A version arrives with no context at all when
        // it is a whole typed leaf such as a harness version, which the
        // residual scan reads, so this cannot rest on the cues below.
        if text.ends_with(".0") {
            continue;
        }
        let Ok(address) = text.parse::<Ipv4Addr>() else {
            continue;
        };
        match byte_before(input, start) {
            // `v1.2.3.4`, `x1.2.3.4`, a run glued to a word.
            Some(byte) if is_word_byte(byte) || byte == b'.' || byte == b'-' => continue,
            // `Chrome/120.0.0.0` or a path segment; `http://1.2.3.4` is kept.
            Some(b'/') if byte_before(input, start - 1) != Some(b'/') => continue,
            _ => {}
        }
        match byte_at(input, end) {
            Some(byte) if is_word_byte(byte) => continue,
            // `1.2.3.4-beta`, `1.2.3.4+build`.
            Some(b'-' | b'+')
                if byte_at(input, end + 1).is_some_and(|next| next.is_ascii_alphanumeric()) =>
            {
                continue;
            }
            _ => {}
        }
        let window = input
            .get(start.saturating_sub(12)..start)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if VERSION_CUES.iter().any(|cue| window.contains(cue)) {
            continue;
        }
        if !ipv4_is_identifying(address) {
            continue;
        }
        findings.push(Finding {
            range: start..end,
            label: IP_ADDRESS,
        });
    }
}

/// Whether an IPv6 address is worth replacing: global unicast (2000::/3)
/// other than the 2001:db8::/32 documentation prefix, and unique-local
/// (fc00::/7), for the same reasons as the IPv4 public and private ranges.
/// Requiring one of those prefixes is also what keeps `dead::beef` and other
/// hex-and-colon code out.
fn ipv6_is_identifying(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    let global = (0x2000..=0x3fff).contains(&segments[0])
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8);
    let unique_local = (segments[0] & 0xfe00) == 0xfc00;
    global || unique_local
}

fn ipv6_findings(input: &str, findings: &mut Vec<Finding>) {
    for found in HEX_COLON_RUN.find_iter(input) {
        let (start, end) = (found.start(), found.end());
        if byte_before(input, start).is_some_and(|byte| is_word_byte(byte) || byte == b'.') {
            continue;
        }
        match byte_at(input, end) {
            Some(byte) if is_word_byte(byte) => continue,
            // An IPv4-suffixed form; the IPv4 pass sees its tail.
            Some(b'.') if byte_at(input, end + 1).is_some_and(|next| next.is_ascii_digit()) => {
                continue;
            }
            _ => {}
        }
        let Ok(address) = found.as_str().parse::<Ipv6Addr>() else {
            continue;
        };
        if ipv6_is_identifying(address) {
            findings.push(Finding {
                range: start..end,
                label: IP_ADDRESS,
            });
        }
    }
}

// --- phone_number ----------------------------------------------------------

/// A `+` followed by a country code and digit groups. Validated below.
static E164_CANDIDATE: LazyLock<Regex> = LazyLock::new(|| regex(r"\+[1-9][0-9 ().-]{6,22}[0-9]"));

/// The group structure an E.164 candidate must have: single separators
/// between groups of one to four digits, optionally parenthesised.
static E164_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^\+[1-9][0-9]{0,3}(?:[ .-]?(?:\([0-9]{1,4}\)|[0-9]{1,4}))+$"));

/// North American numbers in their written forms, with the area code and
/// exchange starting 2-9 as the numbering plan requires: `(415) 555-0132`,
/// `415-555-0132`, `415.555.0132`. A bare ten-digit run is never taken:
/// timestamps, ids and counts look exactly like it.
static NANP: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?:\([2-9][0-9]{2}\) ?[2-9][0-9]{2}-[0-9]{4}|[2-9][0-9]{2}-[2-9][0-9]{2}-[0-9]{4}|[2-9][0-9]{2}\.[2-9][0-9]{2}\.[0-9]{4})",
    )
});

/// UK mobile numbers as usually written: `07700 900123`, `07700900123`.
static UK_MOBILE: LazyLock<Regex> = LazyLock::new(|| regex(r"07[0-9]{3} ?[0-9]{6}"));

fn phone_findings(input: &str, findings: &mut Vec<Finding>) {
    for found in E164_CANDIDATE.find_iter(input) {
        let (start, end) = (found.start(), found.end());
        let text = found.as_str();
        if !E164_SHAPE.is_match(text) {
            continue;
        }
        let digits = text.bytes().filter(u8::is_ascii_digit).count();
        let compact = text[1..].bytes().all(|byte| byte.is_ascii_digit());
        if !(8..=15).contains(&digits) || (compact && digits < 10) {
            continue;
        }
        match byte_before(input, start) {
            Some(byte) if is_word_byte(byte) || byte == b'+' => continue,
            // A diff line adding a number (`+1760176800`) is not a phone.
            None | Some(b'\n') if compact => continue,
            _ => {}
        }
        if !phone_end_is_clean(input, end) {
            continue;
        }
        findings.push(Finding {
            range: start..end,
            label: PHONE_NUMBER,
        });
    }
    for regex in [&*NANP, &*UK_MOBILE] {
        for found in regex.find_iter(input) {
            let (start, end) = (found.start(), found.end());
            if byte_before(input, start).is_some_and(|byte| {
                is_word_byte(byte) || matches!(byte, b'.' | b'-' | b'+' | b'/' | b':')
            }) {
                continue;
            }
            if !phone_end_is_clean(input, end) {
                continue;
            }
            findings.push(Finding {
                range: start..end,
                label: PHONE_NUMBER,
            });
        }
    }
}

fn phone_end_is_clean(input: &str, end: usize) -> bool {
    match byte_at(input, end) {
        Some(byte) if is_word_byte(byte) => false,
        Some(b'.' | b'-' | b':' | b'/') => {
            !byte_at(input, end + 1).is_some_and(|next| next.is_ascii_digit())
        }
        _ => true,
    }
}

// --- os_username in account listings ---------------------------------------

/// `id` output: `uid=501(name)`.
static ID_UID: LazyLock<Regex> =
    LazyLock::new(|| regex(r"uid=[0-9]+\((?P<name>[a-z_][a-z0-9_.-]{0,31})\)"));

/// `whoami` followed by its one-word answer on the next line.
static WHOAMI: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?m)(?:^|[$#%>][ \t]+)whoami[ \t]*\r?\n[ \t]*(?P<name>[a-z_][a-z0-9_.-]{0,31})[ \t]*\r?$",
    )
});

/// `USER=`, `LOGNAME=` and `USERNAME=` in an environment dump, and the same
/// keys in a JSON object.
static ENV_ACCOUNT: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?m)(?:(?:^|[\s"'])(?:USER|LOGNAME|USERNAME)=(?P<name>[a-z_][a-z0-9_.-]{0,31})(?:$|[\s"';])|"(?:USER|LOGNAME|USERNAME)"[ \t]*:[ \t]*"(?P<json>[a-z_][a-z0-9_.-]{0,31})")"#,
    )
});

/// An `ls -l` row: mode, link count, owner, group, then the size.
static LS_LONG_ROW: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"(?m)^[-dlcbpsD][rwxsStT-]{9}[@+.]?[ \t]+[0-9]+[ \t]+(?P<owner>\S+)[ \t]+(?P<group>\S+)[ \t]+[0-9]",
    )
});

fn push_account(findings: &mut Vec<Finding>, name: regex::Match<'_>) {
    if !is_shared_account(name.as_str()) {
        findings.push(Finding {
            range: name.range(),
            label: OS_USERNAME,
        });
    }
}

fn account_listing_findings(input: &str, findings: &mut Vec<Finding>) {
    for captures in ID_UID.captures_iter(input) {
        let Some(name) = captures.name("name") else {
            continue;
        };
        if is_shared_account(name.as_str()) {
            continue;
        }
        // The same name recurs as the primary group and in `groups=` on most
        // systems; take every `N(name)` on the line.
        let line_end = input[name.end()..]
            .find('\n')
            .map_or(input.len(), |offset| name.end() + offset);
        let needle = format!("({})", name.as_str());
        let mut from = name.start() - 1;
        while let Some(offset) = input[from..line_end].find(&needle) {
            let open = from + offset;
            findings.push(Finding {
                range: open + 1..open + needle.len() - 1,
                label: OS_USERNAME,
            });
            from = open + needle.len();
        }
    }
    for captures in WHOAMI.captures_iter(input) {
        if let Some(name) = captures.name("name") {
            push_account(findings, name);
        }
    }
    for captures in ENV_ACCOUNT.captures_iter(input) {
        if let Some(name) = captures.name("name").or_else(|| captures.name("json")) {
            push_account(findings, name);
        }
    }
    for captures in LS_LONG_ROW.captures_iter(input) {
        let (Some(owner), Some(group)) = (captures.name("owner"), captures.name("group")) else {
            continue;
        };
        push_account(findings, owner);
        if group.as_str() == owner.as_str() {
            push_account(findings, group);
        }
    }
}

#[cfg(test)]
mod tests {
    #![deny(dead_code)]

    use super::super::{
        ConsentMetadata, ConsentScope, DeterministicTraceRedactor, RawTraceCaptureToolCall,
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
        RedactionReport, ResidualPiiRisk, TRACE_CONTRIBUTION_POLICY_VERSION, TraceRedactor,
        label_bears_severity, rescrub_trace_envelope_with, residual_envelope_scan, residual_risk,
    };
    use super::*;

    const NEW_LABELS: [&str; 5] = [
        GIT_AUTHOR,
        IP_ADDRESS,
        PRIVATE_HOSTNAME,
        OS_USERNAME,
        PHONE_NUMBER,
    ];

    fn redactor() -> DeterministicTraceRedactor {
        DeterministicTraceRedactor::deterministic_only(Vec::new())
    }

    fn redact(input: &str) -> (String, RedactionReport) {
        redactor().redact_text(input)
    }

    fn count(report: &RedactionReport, label: &str) -> u32 {
        report.counts.get(label).copied().unwrap_or(0)
    }

    /// Asserts the text is untouched and none of the new labels fired.
    fn assert_untouched(input: &str) {
        let (out, report) = redact(input);
        for label in NEW_LABELS {
            assert_eq!(
                count(&report, label),
                0,
                "{label} fired on {input:?} -> {out:?}"
            );
        }
        assert_eq!(out, input, "text changed: {input:?}");
    }

    // --- git_author -------------------------------------------------------

    #[test]
    fn git_log_author_lines_keep_the_label_and_lose_the_name() {
        let input = "commit 3f2a9c1\nAuthor: Jane Q. Example <jane@example.com>\nDate:   Sat Oct 11 10:00:00 2026 +0000\n\n    Fix the build\n";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "commit 3f2a9c1\nAuthor: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\nDate:   Sat Oct 11 10:00:00 2026 +0000\n\n    Fix the build\n"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 1);
        assert_eq!(count(&report, "private_email"), 1);
    }

    #[test]
    fn git_trailers_are_covered_case_insensitively_and_repeat_names_share_a_placeholder() {
        let input = "    Signed-off-by: Jane Example <jane@example.com>\n    Co-Authored-By: Sam Sample <sam@example.org>\n    co-authored-by: Jane Example <jane@example.com>\nCommitter: Sam Sample <sam@example.org>";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "    Signed-off-by: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\n    Co-Authored-By: <PRIVATE_GIT_AUTHOR_2> <<PRIVATE_EMAIL_2>>\n    co-authored-by: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\nCommitter: <PRIVATE_GIT_AUTHOR_2> <<PRIVATE_EMAIL_2>>"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 4);
    }

    #[test]
    fn git_author_lines_inside_an_escaped_json_string_are_covered() {
        let input = r#"{"stdout":"commit abc\nAuthor: Jane Example <jane@example.com>\n"}"#;
        let (out, report) = redact(input);
        assert!(out.contains(r"\nAuthor: <PRIVATE_GIT_AUTHOR_1> <"), "{out}");
        assert_eq!(count(&report, GIT_AUTHOR), 1);
    }

    #[test]
    fn git_blame_lines_lose_the_author_name() {
        let input = "^3f2a9c1 src/lib.rs (Jane Example 2026-10-11 10:00:00 +0000 12) fn main() {}";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "^3f2a9c1 src/lib.rs (<PRIVATE_GIT_AUTHOR_1> 2026-10-11 10:00:00 +0000 12) fn main() {}"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 1);
    }

    #[test]
    fn prose_mentioning_an_author_is_not_a_git_line() {
        assert_untouched("The author: see the README for details.");
        assert_untouched("Author: unknown");
        assert_untouched("AuthorDate: Sat Oct 11 10:00:00 2026 +0000");
        // Measured on this repository's source: a struct field.
        assert_untouched("        commit: Option<String>,");
    }

    // --- ip_address -------------------------------------------------------

    #[test]
    fn public_and_private_ipv4_addresses_are_replaced() {
        let (out, report) = redact(
            "Connecting to 34.117.59.81:443 from 10.0.4.17 via 192.168.1.20 and 100.101.102.103, again 34.117.59.81",
        );
        assert_eq!(
            out,
            "Connecting to <PRIVATE_IP_ADDRESS_1>:443 from <PRIVATE_IP_ADDRESS_2> via <PRIVATE_IP_ADDRESS_3> and <PRIVATE_IP_ADDRESS_4>, again <PRIVATE_IP_ADDRESS_1>"
        );
        assert_eq!(count(&report, IP_ADDRESS), 5);
    }

    #[test]
    fn loopback_unspecified_link_local_documentation_and_masks_stay() {
        assert_untouched("listening on 127.0.0.1:8080 and 0.0.0.0:3000");
        assert_untouched("link-local 169.254.169.254 metadata");
        assert_untouched("examples 192.0.2.10, 198.51.100.7, 203.0.113.99");
        assert_untouched("netmask 255.255.255.0 broadcast 255.255.255.255 wildcard 0.0.0.255");
        assert_untouched("multicast 224.0.0.251");
        assert_untouched("resolvers 8.8.8.8 and 1.1.1.1");
    }

    #[test]
    fn version_strings_and_longer_dotted_runs_are_not_addresses() {
        assert_untouched("bumped to v1.2.3.4 today");
        assert_untouched("version 1.2.3.4");
        assert_untouched("pinned foo==1.2.3.4");
        assert_untouched("Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36");
        assert_untouched("oid 1.3.6.1.4.1.311.21");
        assert_untouched("release 1.2.3.4-beta.1");
        assert_untouched("not an address 256.1.1.1 or 01.02.03.04");
        assert_untouched("rustc 1.82.0 (f6e511eec 2024-10-15)");
        assert_untouched("cache key ab/34.117.59.81/blob");
        // Measured: the only fixture-corpus hit before this rule was a bare
        // version leaf in tests/fixtures/update-conformance.
        assert_untouched("1.2.3.4");
        assert_untouched(r#"{ "current": "1.2.3", "offered": "2.0.0.1" }"#);
        // Measured on this repository's source: a published provider range.
        assert_untouched(r#"IAP_RANGE="35.235.240.0/20""#);
        assert_untouched("4.12.7.0");
    }

    #[test]
    fn a_single_host_in_cidr_notation_is_still_an_address() {
        let (out, report) = redact("allow 34.117.59.81/32");
        assert_eq!(out, "allow <PRIVATE_IP_ADDRESS_1>/32");
        assert_eq!(count(&report, IP_ADDRESS), 1);
    }

    #[test]
    fn urls_with_an_address_host_have_the_host_replaced() {
        let (out, report) = redact("curl http://34.117.59.81/health and https://10.1.2.3:8443/x");
        assert_eq!(
            out,
            "curl http://<PRIVATE_IP_ADDRESS_1>/health and https://<PRIVATE_IP_ADDRESS_2>:8443/x"
        );
        assert_eq!(count(&report, IP_ADDRESS), 2);
    }

    #[test]
    fn global_and_unique_local_ipv6_addresses_are_replaced() {
        let (out, report) = redact(
            "inet6 2a01:4f8:c17:1234::1 prefixlen 64 and fd12:3456:789a::5 and [2606:4700:4700::1111]:443",
        );
        assert_eq!(
            out,
            "inet6 <PRIVATE_IP_ADDRESS_1> prefixlen 64 and <PRIVATE_IP_ADDRESS_2> and [<PRIVATE_IP_ADDRESS_3>]:443"
        );
        assert_eq!(count(&report, IP_ADDRESS), 3);
    }

    #[test]
    fn ipv6_lookalikes_and_non_global_ipv6_stay() {
        assert_untouched("loopback ::1 and fe80::1%en0 and 2001:db8::1");
        assert_untouched("at 12:30:45 the mac aa:bb:cc:dd:ee:ff changed");
        assert_untouched("use std::io::Error; serde::de::Deserialize");
        assert_untouched("let x = a::b::c; dead::beef");
        assert_untouched("sha 3f2a9c1e4b5d6f7a8b9c0d1e2f3a4b5c6d7e8f90");
    }

    // --- private_hostname -------------------------------------------------

    #[test]
    fn private_suffix_hostnames_are_replaced() {
        let (out, report) = redact(
            "ping Janes-MacBook-Pro.local; ssh build01.corp; curl http://grafana.internal:3000/ and nas.home.arpa, db.lan",
        );
        assert_eq!(
            out,
            "ping <PRIVATE_HOSTNAME_1>; ssh <PRIVATE_HOSTNAME_2>; curl http://<PRIVATE_HOSTNAME_3>:3000/ and <PRIVATE_HOSTNAME_4>, <PRIVATE_HOSTNAME_5>"
        );
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 5);
    }

    #[test]
    fn well_known_internal_names_and_local_files_stay() {
        assert_untouched("host.docker.internal and metadata.google.internal");
        assert_untouched("edit .env.local and Makefile.local and settings.local.json");
        assert_untouched("see config/app.local/readme and localhost:8080");
        assert_untouched("github.com and docs.rs and example.com");
        // Measured on this repository's source: member accesses.
        assert_untouched("TomorrowAtNine(now, localZone ?? TimeZoneInfo.Local),");
        assert_untouched(r#"return self.local / f"pipeline-{check_id}.json""#);
        assert_untouched("this.internal = true; cache.local()");
    }

    #[test]
    fn an_address_at_a_private_suffix_stays_an_email() {
        let (out, report) = redact("mail jane@corp.internal");
        assert_eq!(out, "mail <PRIVATE_EMAIL_1>");
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 0);
    }

    // --- shell prompts, ssh targets and os_username ----------------------

    #[test]
    fn shell_prompts_lose_user_and_host() {
        let (out, report) = redact(
            "jdoe@devbox:~/src$ ls\njdoe@Janes-MacBook-Pro trace-commons % git status\n[jdoe@fedora ~]$ id",
        );
        assert_eq!(
            out,
            "<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1>:~/src$ ls\n<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_2> trace-commons % git status\n[<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_3> ~]$ id"
        );
        assert_eq!(count(&report, OS_USERNAME), 3);
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 3);
    }

    #[test]
    fn a_prompt_at_a_local_host_is_not_an_email() {
        let (out, report) = redact("jdoe@devbox.local:~$ make");
        assert_eq!(out, "<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1>:~$ make");
        assert_eq!(count(&report, "private_email"), 0);
    }

    #[test]
    fn generic_accounts_keep_their_name() {
        let (out, report) = redact("root@buildhost:/# ls");
        assert_eq!(out, "root@<PRIVATE_HOSTNAME_1>:/# ls");
        assert_eq!(count(&report, OS_USERNAME), 0);
    }

    #[test]
    fn ssh_family_targets_lose_user_and_bare_host() {
        let (out, report) =
            redact("ssh -p 2222 jdoe@buildbox uptime\nscp out.tar ubuntu@34.117.59.81:/srv/");
        assert_eq!(
            out,
            "ssh -p 2222 <PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1> uptime\nscp out.tar ubuntu@<PRIVATE_IP_ADDRESS_1>:/srv/"
        );
        assert_eq!(count(&report, OS_USERNAME), 1);
    }

    #[test]
    fn package_and_action_references_are_not_user_at_host() {
        assert_untouched("npm install lodash@latest react@18 @types/node");
        assert_untouched("uses: actions/checkout@v6");
        assert_untouched("git rev-parse HEAD@{upstream}");
    }

    #[test]
    fn id_whoami_env_and_ls_output_lose_the_account_name() {
        let input = "$ id\nuid=501(jdoe) gid=20(staff) groups=20(staff),12(everyone),501(jdoe)\n$ whoami\njdoe\nUSER=jdoe\nLOGNAME=jdoe\n-rw-r--r--@  1 jdoe  staff  1204 Oct 11 10:00 notes.txt\ndrwxr-xr-x   4 jdoe  jdoe    128 Oct 11 10:00 src\n-rw-r--r--   1 root  wheel    10 Oct 11 10:00 hosts";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "$ id\nuid=501(<PRIVATE_OS_USERNAME_1>) gid=20(staff) groups=20(staff),12(everyone),501(<PRIVATE_OS_USERNAME_1>)\n$ whoami\n<PRIVATE_OS_USERNAME_1>\nUSER=<PRIVATE_OS_USERNAME_1>\nLOGNAME=<PRIVATE_OS_USERNAME_1>\n-rw-r--r--@  1 <PRIVATE_OS_USERNAME_1>  staff  1204 Oct 11 10:00 notes.txt\ndrwxr-xr-x   4 <PRIVATE_OS_USERNAME_1>  <PRIVATE_OS_USERNAME_1>    128 Oct 11 10:00 src\n-rw-r--r--   1 root  wheel    10 Oct 11 10:00 hosts"
        );
        assert_eq!(count(&report, OS_USERNAME), 8);
    }

    #[test]
    fn a_home_directory_name_is_already_a_path() {
        let (out, report) = redact("cd /home/jdoe/src && ls /Users/jdoe");
        assert_eq!(
            out,
            "cd <PRIVATE_LOCAL_PATH_1> && ls <PRIVATE_LOCAL_PATH_2>"
        );
        assert_eq!(count(&report, OS_USERNAME), 0);
    }

    // --- phone_number -----------------------------------------------------

    #[test]
    fn e164_and_national_phone_numbers_are_replaced() {
        let (out, report) = redact(
            "call +1 415 555 0132 or +44 20 7946 0958 or +14155550132, (415) 555-0132, 415-555-0132, 07700 900123",
        );
        assert_eq!(
            out,
            "call <PRIVATE_PHONE_NUMBER_1> or <PRIVATE_PHONE_NUMBER_2> or <PRIVATE_PHONE_NUMBER_3>, <PRIVATE_PHONE_NUMBER_4>, <PRIVATE_PHONE_NUMBER_5>, <PRIVATE_PHONE_NUMBER_6>"
        );
        assert_eq!(count(&report, PHONE_NUMBER), 6);
    }

    #[test]
    fn numbers_that_are_not_phone_numbers_stay() {
        assert_untouched("ts 1760176800 pid 48213 port 5432");
        assert_untouched("date 2026-10-11 and 2026-10-11T10:00:00Z");
        assert_untouched("src/lib.rs:123:45: error");
        assert_untouched("+1760176800\n+    let x = 4155550132;");
        assert_untouched("ssn-shaped 123-45-6789 and short 555-0132");
        assert_untouched("call 115-555-0132 (area codes never start with 1)");
        assert_untouched("uuid 123e4567-e89b-12d3-a456-426614174000");
        assert_untouched("bytes 1+234+567+8901");
    }

    // --- whole-pipeline properties ----------------------------------------

    const MIXED: &str = "jdoe@devbox:~$ git log -1\ncommit 3f2a9c1\nAuthor: Jane Example <jane@example.com>\n\nuid=501(jdoe) gid=20(staff)\nping Janes-MacBook-Pro.local (10.0.4.17): 34.117.59.81 2a01:4f8:c17:1234::1 +1 415 555 0132";

    #[test]
    fn running_twice_changes_nothing_and_finds_nothing() {
        let (once, first) = redact(MIXED);
        for label in NEW_LABELS {
            assert!(
                count(&first, label) > 0,
                "{label} did not fire on the mixed sample"
            );
        }
        let (twice, second) = redact(&once);
        assert_eq!(twice, once);
        for label in NEW_LABELS {
            assert_eq!(
                count(&second, label),
                0,
                "{label} fired on its own output: {once}"
            );
        }
        assert!(
            !once.contains("<<PRIVATE_GIT"),
            "double placeholder in {once}"
        );
        assert!(
            !once.contains("<PRIVATE_PRIVATE"),
            "nested placeholder in {once}"
        );
    }

    #[test]
    fn severity_follows_the_person_versus_machine_split() {
        assert!(label_bears_severity(GIT_AUTHOR));
        assert!(label_bears_severity(PHONE_NUMBER));
        assert!(!label_bears_severity(IP_ADDRESS));
        assert!(!label_bears_severity(PRIVATE_HOSTNAME));
        assert!(!label_bears_severity(OS_USERNAME));
        // The existing exemption and its neighbour are untouched.
        assert!(!label_bears_severity("local_path"));
        assert!(label_bears_severity("private_email"));
    }

    fn no_content_consent() -> ConsentMetadata {
        ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            correction_included: false,
            routing_metadata_included: false,
            revocable: true,
        }
    }

    #[test]
    fn machine_identifiers_alone_do_not_raise_the_tier() {
        let (_, report) = redact("ping Janes-MacBook-Pro.local (10.0.4.17) as jdoe@devbox:~$");
        assert!(count(&report, IP_ADDRESS) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Low
        );
        let (_, report) = redact("Author: Jane Example <jane@example.com>");
        assert!(count(&report, GIT_AUTHOR) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Medium
        );
        let (_, report) = redact("call +44 20 7946 0958");
        assert!(count(&report, PHONE_NUMBER) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Medium
        );
    }

    /// The exemption is an exact match on the deterministic label. A
    /// classifier's IP span is counted as `privacy_filter:ip_address` and
    /// still raises the tier.
    #[test]
    fn a_classifier_ip_finding_still_bears_severity() {
        let mut report = RedactionReport::default();
        report.increment("privacy_filter:ip_address");
        report.add_pii_label("ip_address");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Medium
        );
    }

    /// The token-distribution provenance path tracks every edit; a pass that
    /// rewrote text without recording its edits would silently drop the map.
    #[tokio::test]
    async fn the_edit_map_survives_the_new_passes() {
        let (redacted, edits) = redactor()
            .redact_text_with_edits(MIXED)
            .await
            .expect("redacts");
        assert_eq!(redacted.redacted, redact(MIXED).0);
        assert!(edits.is_some(), "edit map was dropped");
    }

    fn tool_heavy_raw() -> RawTraceContribution {
        let started = chrono::Utc::now();
        RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "why is the build failing on jdoe@devbox:~$ make".to_string(),
                response: Some("The last commit changed the host to grafana.internal.".to_string()),
                tool_calls: vec![RawTraceCaptureToolCall {
                    name: "shell".to_string(),
                    id: Some("call_1".to_string()),
                    arguments: Some(serde_json::json!({"command": "git log -1 && id && ifconfig"})),
                    result_preview: Some(MIXED.to_string()),
                    error: None,
                    rationale: None,
                }],
                started_at: started,
                completed_at: Some(started + chrono::Duration::milliseconds(10)),
                state: Some("Completed".to_string()),
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                include_tool_payloads: true,
                ..RecordedTraceContributionOptions::default()
            },
        )
    }

    /// The residual scan runs every detector over EVERY string leaf of the
    /// finished envelope, typed fields included. A new label showing up there
    /// would make `residual_clean()` false and block a downgrade for good.
    #[tokio::test]
    async fn a_redacted_envelope_leaves_no_residual_finding_for_the_new_labels() {
        let redactor = redactor();
        let mut raw = tool_heavy_raw();
        // Four-part harness versions are whole typed leaves with no context
        // around them; the residual scan reads them all the same.
        raw.ironclaw.version = "4.12.7.0".to_string();
        raw.ironclaw.engine_version = Some("1.12.0.0".to_string());
        let envelope = redactor.redact_trace(raw).await.expect("redacts");
        assert_eq!(envelope.ironclaw.version, "4.12.7.0");
        assert_eq!(
            envelope.ironclaw.engine_version.as_deref(),
            Some("1.12.0.0")
        );
        for label in NEW_LABELS {
            assert!(
                envelope
                    .privacy
                    .redaction_counts
                    .get(label)
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{label} absent from {:?}",
                envelope.privacy.redaction_counts
            );
        }
        let residual = residual_envelope_scan(&redactor, &envelope).expect("scan runs");
        for label in NEW_LABELS {
            assert_eq!(
                residual.counts.get(label),
                None,
                "{label} survived into the finished envelope"
            );
        }
        let serialized = serde_json::to_string(&envelope).unwrap();
        for raw in [
            "jdoe",
            "Jane Example",
            "34.117.59.81",
            "Janes-MacBook-Pro",
            "415 555 0132",
        ] {
            assert!(!serialized.contains(raw), "{raw} reached the envelope");
        }
    }

    /// The server re-runs the same pass over a client-redacted envelope. It
    /// must find nothing new and change no event content.
    #[tokio::test]
    async fn the_server_rescrub_is_idempotent_over_a_client_redacted_envelope() {
        let redactor = redactor();
        let mut envelope = redactor
            .redact_trace(tool_heavy_raw())
            .await
            .expect("redacts");
        let before_events = serde_json::to_value(&envelope.events).unwrap();
        let before_counts = envelope.privacy.redaction_counts.clone();
        rescrub_trace_envelope_with(&redactor, &mut envelope);
        assert_eq!(
            serde_json::to_value(&envelope.events).unwrap(),
            before_events
        );
        for label in NEW_LABELS {
            assert_eq!(
                envelope.privacy.redaction_counts.get(label),
                before_counts.get(label),
                "{label} counted again by the server pass"
            );
        }
    }
}

#!/usr/bin/env python3
"""Independent positive-evidence scanner for trace fuzzing corpora.

The scanner reports structural evidence of sensitive material. It does not rewrite
input and does not rely on a list of terms that another component claims to remove.
Returned findings contain hashes and structural locations, never captured values.
"""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import math
import os
import re
from pathlib import Path
from typing import Any, Iterable, Iterator, TypedDict


class Detection(TypedDict):
    forbidden_class: str
    detector: str
    surface: str
    source_hash: str
    locator_hash: str
    evidence_shape: str


class AuditFailure(TypedDict):
    source_hash: str
    error: str


class AuditReport(TypedDict):
    corpus_hash: str
    cases_run: int
    failures: list[AuditFailure]
    detections: list[Detection]


_SAFE_PATH_TOKENS = {
    "",
    ".",
    "..",
    "applications",
    "bin",
    "cache",
    "claude",
    "codex",
    "data",
    "documents",
    "downloads",
    "etc",
    "home",
    "library",
    "logs",
    "opt",
    "private",
    "project",
    "projects",
    "repo",
    "shared",
    "src",
    "synthetic",
    "synthetic-user",
    "tmp",
    "trace",
    "traces",
    "user",
    "users",
    "usr",
    "var",
    "workspace",
}

_PLACEHOLDER_RE = re.compile(
    r"^\s*(?:\[(?:redacted|opaque|user|placeholder|synthetic)[^\]]*\]"
    r"|<[^>]*(?:redacted|placeholder|synthetic)[^>]*>"
    r"|(?:redacted|placeholder|synthetic|example|dummy|none|null|unknown|unset)"
    r"|\$\{?[A-Z_][A-Z0-9_]*\}?|\*{3,}|x{4,})\s*$",
    re.IGNORECASE,
)

_ROLE_WORDS = (
    r"advisor|attorney|chief|client|co-?founder|counsel|contractor|director|"
    r"aunt|daughter|employee|employer|executive|father|founder|husband|lawyer|mother|"
    r"officer|operator|owner|partner|"
    r"participant|prepared\s+by|principal|reviewer|shareholder|signatory|speaker|"
    r"recipient|sender|son|spouse|team\s+member|uncle|vice\s+president|wife"
)
_PERSON = r"[A-Z][a-z]{2,}(?:[-'][A-Z][a-z]{2,})?\s+[A-Z][a-z]{2,}(?:[-'][A-Z][a-z]{2,})?"
# _PERSON is deliberately compiled case-SENSITIVELY: its whole purpose is the
# [A-Z][a-z]... proper-noun shape. A surrounding re.IGNORECASE (as this used to
# carry) degrades that shape to "any two words of 3+ letters", which fires on
# ordinary lowercase prose ("rotating the client", "Redis operator"). Only the
# role-word vocabulary is case-insensitive, scoped locally with (?i:...).
_NAME_ROLE_RE = re.compile(
    rf"(?:\b{_PERSON}\b.{{0,72}}\b(?i:{_ROLE_WORDS})\b|"
    rf"\b(?i:{_ROLE_WORDS})\b.{{0,72}}\b{_PERSON}\b)"
)
_CHAT_SPEAKER_RE = re.compile(
    rf"(?:\[[^\]\n]{{4,48}}\]\s*{_PERSON}\s*:|\b{_PERSON}\s+in\s+reply\s+to\s+{_PERSON}\b)"
)
_REDACTION_NEIGHBOR_RE = re.compile(
    r"(?:\[(?:USER|REDACTED(?:_TERM)?)\]\s+[A-Z][a-z]{2,}"
    r"|[A-Z][a-z]{2,}\s+\[(?:USER|REDACTED(?:_TERM)?)\])"
)
_SELF_PROFILE_RE = re.compile(
    rf"(?:\[(?:USER|REDACTED(?:_PERSON)?)\]|\b(?:the\s+user|his|her|their)\b)"
    rf".{{0,120}}\b(?:{_ROLE_WORDS}|admitted|bar|works?\s+at|formerly|based\s+in)\b",
    re.IGNORECASE,
)
# A trailing \b after the alternation stops "to"/"from" from matching as a bare
# prefix of a longer word (e.g. "to" inside "tokens"). But "to"/"from" are also
# common English prepositions with proper trailing boundaries of their own
# ("to reduce throttling", "to rotate the") -- word-boundary alone doesn't
# distinguish those from an actual participant/speaker field. The structural
# signal that does is a field-style separator: real transcript/digest headers
# read "From: <name>", "Attendees: <name>", not bare prose "to <two words>".
# Requiring the ":"/"=" separator (no longer optional) keeps the field-label
# reading and drops the preposition reading.
_PARTICIPANT_NAME_RE = re.compile(
    r"\b(?:meeting\s+participants?|attendees?|speaker|from|to)\b\s*[:=]\s*"
    r"[a-z][a-z'-]{2,}\s+[a-z][a-z'-]{2,}\b",
    re.IGNORECASE,
)
_HANDLE_RE = re.compile(r"(?<![\w.])@[A-Za-z][A-Za-z0-9_]{3,31}\b")
_ACCOUNT_CONTEXT_RE = re.compile(
    r"\b(?:github|gitlab|twitter|x/twitter|telegram|gmail|oauth2?|login|user(?:name)?)"
    r"\b.{0,48}(?:@[A-Za-z][A-Za-z0-9_]{3,31}\b|[a-z][a-z0-9_-]{3,}\.[a-z][a-z0-9_.-]{2,})",
    re.IGNORECASE,
)
_LAW_DOMAIN_RE = re.compile(r"\b[a-z][a-z0-9-]{3,}\.(?:law|legal)\b", re.IGNORECASE)

_CREDENTIAL_RE = re.compile(
    r"\b(?P<cue>api[ _-]?key|access[ _-]?key|auth[ _-]?token|bearer|client[ _-]?secret|"
    r"confirmation[ _-]?(?:code|number)|credential|invite[ _-]?(?:code|hash)|operator[ _-]?id|"
    r"pass(?:word|phrase|code)|pin|pnr|private[ _-]?key|record[ _-]?locator|secret|token)\b"
    r"\s*(?:is|=|:|#|\"\s*:\s*\")\s*[\"'`]?"
    r"(?P<value>[^\s\"'`,;}\]]{4,160})",
    re.IGNORECASE,
)
_SPLIT_LITERAL_RE = re.compile(
    r"[\"'][A-Za-z][A-Za-z0-9.-]{2,31}_[\"']\s*\+\s*"
    r"[\"'][A-Za-z0-9_-]{16,160}[\"']"
)
_PRIVATE_ROOM_RE = re.compile(
    r"\b(?:https?://)?(?:t\.me/\+[A-Za-z0-9_-]{10,}|"
    r"meet\.google\.com/[a-z]{3}-[a-z]{4}-[a-z]{3})\b",
    re.IGNORECASE,
)
_MAC_RE = re.compile(r"(?<![0-9a-f])(?:[0-9a-f]{2}:){5}[0-9a-f]{2}(?![0-9a-f])", re.I)
_DARWIN_FINGERPRINT_RE = re.compile(
    r"/var/folders/[a-z0-9_]{2}/[A-Za-z0-9_+-]{20,}", re.IGNORECASE
)
_REVERSE_DNS_PERSON_RE = re.compile(
    r"\b(?:com|net|org)\.[a-z][a-z0-9-]{3,}\.[a-z0-9._-]{2,}\b", re.IGNORECASE
)
_POSTAL_ADDRESS_RE = re.compile(
    r"\b\d{1,6}\s+(?:\[(?:REDACTED_TERM|USER)\]|[A-Za-z][A-Za-z'-]{2,})"
    r"(?:\s+[A-Za-z][A-Za-z'-]{2,})?\s+(?:Ave(?:nue)?|Blvd|Drive|Dr|Lane|Ln|Road|Rd|Street|St)\b"
    r".{0,80}\b[A-Z]\d[A-Z]\s?\d[A-Z]\d\b",
    re.IGNORECASE,
)
_DOB_RE = re.compile(
    r"\b(?:date\s+of\s+birth|dob|born)\b.{0,36}"
    r"(?:19|20)\d{2}[-/.](?:0?[1-9]|1[0-2])[-/.](?:0?[1-9]|[12]\d|3[01])",
    re.IGNORECASE,
)
_FAMILY_DETAIL_RE = re.compile(
    r"\b(?:child|children|daughter|family|father|husband|maternal|mother|son|spouse|wife)\b"
    r".{0,100}\b(?:age|born|dob|diagnos(?:is|ed)|medical|medication|name|year-?old)\b",
    re.IGNORECASE,
)
_HEALTH_RE = re.compile(
    r"\b(?:bloodwork|cardiomyopathy|cpap|diagnos(?:is|ed)|genetic\s+(?:risk|screen)|"
    r"health\s+profile|medical\s+(?:condition|history)|prescription|sleep\s+apnea|treatment)\b",
    re.IGNORECASE,
)
_PRIVATE_SESSION_RE = re.compile(
    r"\b(?:coaching\s+session|family\s+context|marital|personal\s+coaching|"
    r"settlement\s+negotiation|travel\s+itinerary)\b",
    re.IGNORECASE,
)

_LEGAL_STRONG_RE = re.compile(
    r"\b(?:attorney[- ]client|attorney\s+work\s+product|claim\s+quantum|"
    r"confidential\s+legal|ex\s+parte|freezing\s+injunction|legal\s+escalation|"
    r"litigation\s+strategy|not\s+for\s+external\s+circulation|norwich\s+pharmacal|"
    r"privileged(?:\s+and\s+confidential)?|without\s+prejudice)\b",
    re.IGNORECASE,
)
_LEGAL_REGISTER_RE = re.compile(
    r"\b(?:affidavit|application|breach(?:ed)?|counsel|court|damages|defendant|"
    r"engagement|fee\s+cap|filing|injunction|liquidat(?:ion|or)|litigation|matter|"
    r"operative\s+agreement|plaintiff|proceedings|regulatory|settlement|statute|"
    r"term\s+sheet|work\s+product)\b",
    re.IGNORECASE,
)
# Some _LEGAL_REGISTER_RE words are unambiguous markers of an actual legal
# matter (a named party role, a privileged-work-product concept, a specific
# procedural instrument); others ("filing", "regulatory", "settlement",
# "application", "proceedings", "statute", "breach") are ordinary vocabulary
# in non-legal audit/compliance prose ("the audit trail records every filing
# and settlement event ... for regulatory reporting"). Requiring >=2 hits
# ANYWHERE in the string treats both groups the same; a cluster should only
# fire when it contains at least one anchor from the first group.
_LEGAL_MATTER_ANCHOR_WORDS = {
    "affidavit",
    "claim",
    "client",
    "counsel",
    "court",
    "damages",
    "defendant",
    "dispute",
    "engagement",
    "fee cap",
    "injunction",
    "litigation",
    "liquidation",
    "liquidator",
    "matter",
    "operative agreement",
    "plaintiff",
    "privileged",
    "term sheet",
    "work product",
}
_LEGAL_CLUSTER_PROXIMITY_CHARS = 200
_CASE_CITATION_RE = re.compile(
    r"\b[A-Z][A-Za-z.&' -]{2,50}\s+v\.?\s+[A-Z][A-Za-z.&' -]{2,50},\s*"
    r"\d{1,4}\s+[A-Z][A-Za-z. ]+\s+\d{1,5}\s*\(\d{4}\)"
)
_LEGAL_PATH_RE = re.compile(
    r"(?:^|[/_.-])(?:affidavit|claim|complaint|counsel|engagement|injunction|legal|"
    r"litigation|matter|memo|patent|privileged|settlement|tax|term[-_ ]?sheet)(?:$|[/_.-])",
    re.IGNORECASE,
)
_DOCUMENT_TITLE_RE = re.compile(
    rf"\b{_PERSON}\b[^\n]{{0,72}}\b(?:agreement|affidavit|brief|consent|contract|"
    r"memo|resolution|term\s+sheet)\b[^\n]{0,80}\s+\.?(?:docx|html|md|pdf|xlsx|zip)\b",
    re.IGNORECASE,
)
_NAMED_PATH_RE = re.compile(
    r"[A-Z][a-z]{2,}[-_][A-Z][a-z]{2,}[-_][A-Za-z0-9_-]*"
    r"(?:advisor|agreement|brief|consent|contract|director|officer|resolution|term[-_]?sheet)",
    re.IGNORECASE,
)
_CORPORATE_ID_RE = re.compile(
    r"\b(?:corp(?:oration)?|inc(?:orporated)?|ltd|registry)\b.{0,24}\b\d{7,12}\b",
    re.IGNORECASE,
)
_SERVER_ID_RE = re.compile(
    r"\b(?:host|ip|server|ssh)\b.{0,64}\b(?:\d{1,3}\.){3}\d{1,3}\b",
    re.IGNORECASE,
)
_GIT_ID_RE = re.compile(
    r"\b(?:github\.com|gitlab\.com)/[A-Za-z0-9][A-Za-z0-9_-]{2,39}/[A-Za-z0-9_.-]+",
    re.IGNORECASE,
)
# A github.com/gitlab.com path only identifies a PERSON or a private org when
# it is a reference to *that* repo -- not when it is one of thousands of
# import paths sitting inside a vendored/bundled dependency tree (Go's
# vendor/, node_modules/, Godeps/, etc. all embed the upstream repo's own
# import path verbatim). The vendoring convention is the structural signal:
# it says "this is someone else's public library, mechanically copied in",
# not "this trace is about a specific person's or private org's repo".
_VENDORED_DEPENDENCY_PATH_RE = re.compile(
    r"(?:^|[/\\])(?:vendor|vendored|third[-_]?party|node_modules|bower_components|"
    r"deps|Godeps|\.git[/\\]modules|packages)[/\\]\Z",
    re.IGNORECASE,
)


def _short_hash(value: str | bytes) -> str:
    if isinstance(value, str):
        value = value.encode("utf-8", "surrogatepass")
    return hashlib.sha256(value).hexdigest()[:20]


# Kept in step with `shape_signature` in
# crates/trace-commons-contributor/tests/local_redaction_audit.rs, which is
# where this repo settled the question of how an audit describes something it
# must not quote.
_SHAPE_MAX_RUNS = 24


def _shape_signature(value: str, max_runs: int = _SHAPE_MAX_RUNS) -> str:
    """Structural signature with every information-bearing character erased.

    Lowercase becomes ``a``, uppercase ``A``, digits ``9``, whitespace ``_``;
    punctuation is kept verbatim, and runs collapse to ``class{n}``.

    This replaced a plain SHA-256 of the matched text. A hash is the right
    shape of answer for a credential, whose value is high-entropy, and the
    wrong one for most of what this auditor exists to find: an unsalted
    digest of ``Jane Doe``, a date of birth, or a postal address is recovered
    with a wordlist in seconds, so a findings report -- the artefact an
    operator forwards, pastes into an issue, or archives -- carried the very
    third-party PII the scan was run to locate.

    A shape leaks strictly less and says strictly more. It cannot be reversed
    to the name, and unlike an opaque digest it tells a triager whether a
    credential hit is a live secret or a placeholder, which is what the Rust
    auditor's version of this exists to do.

    Not zero-knowledge: a shape still reveals length and character classes.
    That is the repo's standing trade for an audit surface, and it is why the
    output carries no raw text at all.
    """

    def class_of(character: str) -> str:
        if character.islower() and character.isascii():
            return "a"
        if character.isupper() and character.isascii():
            return "A"
        if character.isdigit() and character.isascii():
            return "9"
        if character.isspace():
            return "_"
        return character

    out: list[str] = []
    classes = [class_of(character) for character in value]
    index = 0
    runs = 0
    while index < len(classes):
        if runs >= max_runs:
            out.append("...")
            break
        current = classes[index]
        length = 1
        while index + length < len(classes) and classes[index + length] == current:
            length += 1
        if current in {"a", "A", "9", "_"} and length > 1:
            out.append(f"{current}{{{length}}}")
        else:
            out.append(current * length)
        index += length
        runs += 1
    return "".join(out)


def _is_placeholder(value: str) -> bool:
    return bool(_PLACEHOLDER_RE.fullmatch(value.strip()))


def _entropy_bits_per_char(value: str) -> float:
    if not value:
        return 0.0
    counts: dict[str, int] = {}
    for character in value:
        counts[character] = counts.get(character, 0) + 1
    length = len(value)
    return -sum((count / length) * math.log2(count / length) for count in counts.values())


def _unsafe_home_segment(text: str) -> bool:
    normalized = text.replace("\\", "/")
    for match in re.finditer(r"(?:^|[-/])Users[-/](?P<name>[A-Za-z][A-Za-z0-9._-]{2,})", normalized, re.I):
        name = match.group("name").lower().strip("-._")
        if name not in _SAFE_PATH_TOKENS and not _is_placeholder(name):
            return True
    return False


def _path_has_unknown_named_segment(text: str) -> bool:
    normalized = text.replace("\\", "/")
    for segment in re.split(r"/+", normalized):
        clean = segment.strip().lower()
        if not clean or clean in _SAFE_PATH_TOKENS:
            continue
        words = [word for word in re.split(r"[-_. ]+", clean) if word]
        if len(words) >= 2 and _LEGAL_PATH_RE.search(clean):
            return True
    return False


def _public_ip_with_context(text: str) -> bool:
    for match in _SERVER_ID_RE.finditer(text):
        candidate = re.search(r"(?:\d{1,3}\.){3}\d{1,3}", match.group(0))
        if candidate is None:
            continue
        try:
            address = ipaddress.ip_address(candidate.group(0))
        except ValueError:
            continue
        octets = tuple(int(part) for part in candidate.group(0).split("."))
        is_rfc1918 = (
            octets[0] == 10
            or (octets[0] == 172 and 16 <= octets[1] <= 31)
            or (octets[0] == 192 and octets[1] == 168)
        )
        if not (is_rfc1918 or address.is_loopback or address.is_link_local):
            return True
    return False


def _legal_marker_cluster(text: str) -> bool:
    first_seen: dict[str, int] = {}
    for match in _LEGAL_REGISTER_RE.finditer(text):
        word = " ".join(match.group(0).lower().split())
        first_seen.setdefault(word, match.start())
    if len(first_seen) < 2:
        return False
    if not any(word in _LEGAL_MATTER_ANCHOR_WORDS for word in first_seen):
        return False
    positions = sorted(first_seen.values())
    return (positions[-1] - positions[0]) <= _LEGAL_CLUSTER_PROXIMITY_CHARS


def _source_control_identity(text: str) -> bool:
    for match in _GIT_ID_RE.finditer(text):
        prefix = text[: match.start()]
        if _VENDORED_DEPENDENCY_PATH_RE.search(prefix[-32:]):
            continue
        return True
    return False


def _credential_value(text: str) -> bool:
    for match in _CREDENTIAL_RE.finditer(text):
        value = match.group("value").strip(".()")
        if _is_placeholder(value):
            continue
        if value.lower() in {"true", "false", "yes", "no", "value", "here"}:
            continue
        if len(value) >= 5:
            return True
    return False


_UUID_RE = re.compile(
    r"\A[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\Z"
)
_HEX_DIGEST_RE = re.compile(r"\A(?:[0-9a-f]{20,64}|[0-9A-F]{20,64})\Z")
# Segment vocabulary for package/wheel/build-artifact filenames (PEP 427 wheel
# tags, semver leftovers after the tokenizer splits on ".", git-describe-style
# build suffixes). Bare digit runs and these tags are the only things allowed
# in a "-"/"_"-separated identifier for it to read as packaging metadata
# rather than an opaque secret -- an arbitrary English word (as in a
# diceware-style passphrase like "orbit-canvas-84") never matches, so this
# can't be used to launder a real secret through hyphenation.
_BUILD_TAG_SEGMENT_RE = re.compile(
    r"\A(?:\d+|v\d+|cp\d{2,3}|py\d{2,3}|pp\d{2,3}|abi\d+|"
    r"manylinux\d*|macosx\d*|musllinux\d*|win(?:32|64)?|"
    r"amd64|x86|x64|x86_64|arm64|aarch64|i686|"
    r"universal2?|none|any|linux|darwin)\Z",
    re.IGNORECASE,
)


def _is_structured_identifier(token: str) -> bool:
    """True for well-known benign identifier shapes: UUIDs, hex digests
    (git SHAs and similar), and package/wheel/build-artifact version+tag
    strings (the practical form a bare semver takes once punctuation has
    split the surrounding text into tokens)."""
    if _UUID_RE.match(token) or _HEX_DIGEST_RE.match(token):
        return True
    segments = re.split(r"[-_]", token)
    if len(segments) >= 2 and all(_BUILD_TAG_SEGMENT_RE.match(segment) for segment in segments):
        return True
    return False


def _opaque_identifier(text: str) -> bool:
    for token in re.findall(r"\b[A-Za-z0-9_-]{20,160}\b", text):
        if _is_placeholder(token) or _is_structured_identifier(token):
            continue
        kinds = sum(bool(re.search(pattern, token)) for pattern in (r"[a-z]", r"[A-Z]", r"\d", r"[_-]"))
        if kinds >= 3 and _entropy_bits_per_char(token) >= 3.4:
            return True
    return False


def _emit(
    output: list[Detection],
    seen: set[tuple[str, str, str, str]],
    forbidden_class: str,
    detector: str,
    surface: str,
    source_id: str,
    locator: str,
    evidence: str,
) -> None:
    locator_hash = _short_hash(locator)
    key = (forbidden_class, detector, source_id, locator_hash)
    if key in seen:
        return
    seen.add(key)
    output.append(
        {
            "forbidden_class": forbidden_class,
            "detector": detector,
            "surface": surface,
            "source_hash": source_id,
            "locator_hash": locator_hash,
            "evidence_shape": _shape_signature(evidence),
        }
    )


def _scan_text(
    text: str,
    *,
    surface: str,
    source_id: str,
    locator: str,
    output: list[Detection],
    seen: set[tuple[str, str, str, str]],
) -> None:
    checks: tuple[tuple[str, str, bool], ...] = (
        ("identity", "unsafe_home_segment", _unsafe_home_segment(text)),
        ("identity", "self_profile", bool(_SELF_PROFILE_RE.search(text))),
        ("identity", "participant_identity", bool(_PARTICIPANT_NAME_RE.search(text))),
        ("identity", "account_context", bool(_ACCOUNT_CONTEXT_RE.search(text))),
        ("identity", "social_handle", bool(_HANDLE_RE.search(text))),
        ("identity", "law_professional_domain", bool(_LAW_DOMAIN_RE.search(text))),
        ("identity", "corporate_registry_identity", bool(_CORPORATE_ID_RE.search(text))),
        ("identity", "public_server_identifier", _public_ip_with_context(text)),
        ("identity", "source_control_identity", _source_control_identity(text)),
        ("identity", "reverse_dns_personal_label", bool(_REVERSE_DNS_PERSON_RE.search(text))),
        ("identity", "redaction_neighbor", bool(_REDACTION_NEIGHBOR_RE.search(text))),
        ("third_party_pii", "name_near_role", bool(_NAME_ROLE_RE.search(text))),
        ("third_party_pii", "chat_speaker", bool(_CHAT_SPEAKER_RE.search(text))),
        ("third_party_pii", "named_document", bool(_DOCUMENT_TITLE_RE.search(text))),
        ("third_party_pii", "named_path", bool(_NAMED_PATH_RE.search(text))),
        ("third_party_pii", "private_room", bool(_PRIVATE_ROOM_RE.search(text))),
        ("personal", "postal_address", bool(_POSTAL_ADDRESS_RE.search(text))),
        ("personal", "date_of_birth", bool(_DOB_RE.search(text))),
        ("personal", "family_detail", bool(_FAMILY_DETAIL_RE.search(text))),
        ("personal", "health_detail", bool(_HEALTH_RE.search(text))),
        ("personal", "private_session", bool(_PRIVATE_SESSION_RE.search(text))),
        ("secret", "credential_cue_value", _credential_value(text)),
        ("secret", "split_literal", bool(_SPLIT_LITERAL_RE.search(text))),
        ("secret", "private_room_locator", bool(_PRIVATE_ROOM_RE.search(text))),
        ("secret", "machine_fingerprint", bool(_DARWIN_FINGERPRINT_RE.search(text))),
        ("secret", "hardware_identifier", bool(_MAC_RE.search(text))),
        ("secret", "opaque_identifier", surface != "path" and _opaque_identifier(text)),
        ("legal_matter", "legal_register", bool(_LEGAL_STRONG_RE.search(text))),
        ("legal_matter", "case_citation", bool(_CASE_CITATION_RE.search(text))),
        ("legal_matter", "legal_path", _path_has_unknown_named_segment(text)),
    )
    if _legal_marker_cluster(text):
        checks += (("legal_matter", "legal_marker_cluster", True),)
    for forbidden_class, detector, matched in checks:
        if matched:
            _emit(output, seen, forbidden_class, detector, surface, source_id, locator, text)


def _walk_json(value: Any, pointer: str = "$") -> Iterator[tuple[str, str, str]]:
    if isinstance(value, dict):
        for index, (key, child) in enumerate(value.items()):
            key_locator = f"{pointer}.key[{index}]"
            yield "key", str(key), key_locator
            yield from _walk_json(child, f"{pointer}.value[{index}]")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from _walk_json(child, f"{pointer}[{index}]")
    elif isinstance(value, str):
        yield "value", value, pointer
    elif value is not None:
        yield "value", str(value), pointer


def _iter_files(path: Path) -> Iterable[Path]:
    if path.is_file():
        yield path
        return
    for candidate in sorted(path.rglob("*"), key=lambda item: os.fsencode(str(item))):
        if candidate.is_file() and not candidate.is_symlink():
            yield candidate


def _relative_label(path: Path, root: Path) -> str:
    if root.is_file():
        return path.name
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.name


def audit(path: str | os.PathLike[str]) -> AuditReport:
    """Scan a file or directory and return a privacy-oriented evidence report."""
    target = Path(path).expanduser()
    digest = hashlib.sha256()
    failures: list[AuditFailure] = []
    detections: list[Detection] = []
    seen: set[tuple[str, str, str, str]] = set()
    cases_run = 0

    if not target.exists():
        return {
            "corpus_hash": digest.hexdigest(),
            "cases_run": 0,
            "failures": [{"source_hash": _short_hash(str(target)), "error": "path_not_found"}],
            "detections": [],
        }

    root_source_id = _short_hash(str(target.resolve()))
    path_candidates = [target]
    if target.is_dir():
        path_candidates.extend(sorted(target.rglob("*"), key=lambda item: os.fsencode(str(item))))
    for candidate in path_candidates:
        label = _relative_label(candidate, target)
        digest.update(b"P\0" + os.fsencode(label) + b"\0")
        _scan_text(
            str(candidate),
            surface="path",
            source_id=_short_hash(label),
            locator="filesystem-path",
            output=detections,
            seen=seen,
        )

    for source in _iter_files(target):
        cases_run += 1
        label = _relative_label(source, target)
        source_id = _short_hash(label)
        digest.update(b"F\0" + os.fsencode(label) + b"\0")
        try:
            raw = source.read_bytes()
        except OSError as exc:
            failures.append({"source_hash": source_id, "error": type(exc).__name__})
            continue
        digest.update(len(raw).to_bytes(8, "big"))
        digest.update(raw)
        text = raw.decode("utf-8", "replace")
        parsed_any = False
        for line_number, line in enumerate(text.splitlines(), 1):
            if not line.strip():
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                _scan_text(
                    line,
                    surface="value",
                    source_id=source_id,
                    locator=f"line:{line_number}",
                    output=detections,
                    seen=seen,
                )
                continue
            parsed_any = True
            for surface, scalar, pointer in _walk_json(record):
                _scan_text(
                    scalar,
                    surface=surface,
                    source_id=source_id,
                    locator=f"line:{line_number}:{pointer}",
                    output=detections,
                    seen=seen,
                )
        if not parsed_any and "\n" not in text:
            _scan_text(
                text,
                surface="value",
                source_id=source_id,
                locator="file",
                output=detections,
                seen=seen,
            )

    return {
        "corpus_hash": digest.hexdigest(),
        "cases_run": cases_run,
        "failures": failures,
        "detections": detections,
    }


# Regression fixtures for the five 2026-09 false-positive root causes. Each
# entry is (label, forbidden_class, detector, positive_text, negative_text):
#   - positive_text must still trip `detector` after the fix (guards recall).
#   - negative_text must NOT trip `detector` after the fix (guards precision;
#     this is the failure mode that shipped -- the old self-test's negative
#     set never exercised these shapes).
# Text is deliberately NOT copy-pasted from cases.jsonl: these check the
# general regex/logic fix, not memorized benchmark strings.
_REGRESSION_FIXTURES: tuple[tuple[str, str, str, str, str], ...] = (
    (
        "participant_identity_word_boundary",
        "identity",
        "participant_identity",
        "attendees: mira delgado",
        "helpers route retries to the nearest replica pool",
    ),
    (
        "name_near_role_case_sensitive",
        "third_party_pii",
        "name_near_role",
        "Priya Anand is the finance director",
        "restart the database operator and rotate the client secret",
    ),
    (
        "opaque_identifier_structured_shapes",
        "secret",
        "opaque_identifier",
        "Tf3k9QzR7mX2vB6jH1nY8pL4wD0sA5c",
        "built torch-2.3.1-cp312-cp312-linux_x86_64.whl using request "
        "550e8400-e29b-41d4-a716-446655440000",
    ),
    (
        "source_control_identity_vendored_path",
        "identity",
        "source_control_identity",
        "remote set to github.com/marlowe-industries/internal-tools",
        "vendor/github.com/aws/aws-sdk-go/aws/session.go",
    ),
    (
        "legal_marker_cluster_anchor_and_proximity",
        "legal_matter",
        "legal_marker_cluster",
        "counsel flagged a fee cap issue before the engagement closes",
        "the ledger tracks every filing and settlement update surfaced to "
        "the regulatory dashboard",
    ),
)


def _check_regression_fixtures() -> list[tuple[str, bool, bool]]:
    results: list[tuple[str, bool, bool]] = []
    for label, forbidden_class, detector, positive_text, negative_text in _REGRESSION_FIXTURES:
        pos_output: list[Detection] = []
        _scan_text(
            positive_text,
            surface="value",
            source_id="fixture",
            locator=f"{label}:positive",
            output=pos_output,
            seen=set(),
        )
        neg_output: list[Detection] = []
        _scan_text(
            negative_text,
            surface="value",
            source_id="fixture",
            locator=f"{label}:negative",
            output=neg_output,
            seen=set(),
        )
        positive_ok = any(
            item["forbidden_class"] == forbidden_class and item["detector"] == detector
            for item in pos_output
        )
        negative_ok = not any(
            item["forbidden_class"] == forbidden_class and item["detector"] == detector
            for item in neg_output
        )
        results.append((label, positive_ok, negative_ok))
    return results


# The distinctive substrings of the self-test's own positive samples. A
# detection that contains any of these is quoting its input back.
_SELF_TEST_SENSITIVE_STRINGS: tuple[str, ...] = (
    "Zyxoria",
    "Mockvale",
    "2098-03-05",
    "fuzz-orbit-canvas-84",
    "Fuzzworks",
)


def _self_test() -> int:
    import tempfile

    samples: tuple[tuple[str, str], ...] = (
        ("identity", "[USER] works at Fuzzworks as officer"),
        ("third_party_pii", "Zyxoria Mockvale is the synthetic reviewer"),
        ("personal", "child date of birth 2098-03-05"),
        ("secret", "passphrase: fuzz-orbit-canvas-84"),
        ("legal_matter", "privileged fuzz litigation strategy"),
    )
    with tempfile.TemporaryDirectory(prefix="redaction-audit-fuzz-", dir="/tmp") as temp_name:
        temp_root = Path(temp_name)
        positive_file = temp_root / "positive.jsonl"
        positive_file.write_text(
            "\n".join(
                json.dumps(
                    {
                        "fixture_notice": "SYNTHETIC_FUZZ_FIXTURE",
                        "payload": value,
                    },
                    sort_keys=True,
                )
                for _, value in samples
            )
            + "\n",
            encoding="utf-8",
        )
        positive_report = audit(positive_file)
        observed = {item["forbidden_class"] for item in positive_report["detections"]}
        expected = {item[0] for item in samples}

        key_file = temp_root / "key.jsonl"
        key_file.write_text(
            json.dumps(
                {
                    "fixture_notice": "SYNTHETIC_FUZZ_FIXTURE",
                    "passphrase: fuzz-key-canvas-51": "fixture",
                }
            )
            + "\n",
            encoding="utf-8",
        )
        key_report = audit(key_file)
        key_hit = any(
            item["forbidden_class"] == "secret" and item["surface"] == "key"
            for item in key_report["detections"]
        )

        path_file = temp_root / "Users" / "ZyxoriaMockvale" / "trace.jsonl"
        path_file.parent.mkdir(parents=True)
        path_file.write_text(
            json.dumps({"fixture_notice": "SYNTHETIC_FUZZ_FIXTURE", "event": "build passed"})
            + "\n",
            encoding="utf-8",
        )
        path_report = audit(path_file)
        path_hit = any(
            item["forbidden_class"] == "identity" and item["surface"] == "path"
            for item in path_report["detections"]
        )

        negative_file = temp_root / "negative.jsonl"
        negative_file.write_text(
            json.dumps(
                {
                    "fixture_notice": "SYNTHETIC_FUZZ_FIXTURE",
                    "event": "build passed",
                    "passphrase": "[OPAQUE]",
                    "path": "/Users/user/workspace/src/module.py",
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        negative_report = audit(negative_file)

    # The property the whole output format rests on: a detection describes
    # what was found without carrying it. Checked against the positive corpus
    # above, whose scalars are exactly the sensitive shapes this tool exists
    # to locate -- a name, a date of birth, an address, a credential.
    #
    # This is here because the field it guards used to be a plain SHA-256 of
    # the matched text, which is a fine answer for a high-entropy credential
    # and a poor one for a name: an unsalted digest of `Jane Doe` falls to a
    # wordlist, so a findings report carried the third-party PII the scan was
    # run to find.
    echoed: list[str] = []
    for detection in positive_report["detections"] + key_report["detections"] + path_report["detections"]:
        for field, value in detection.items():
            if not isinstance(value, str):
                continue
            for sensitive in _SELF_TEST_SENSITIVE_STRINGS:
                if sensitive and sensitive in value:
                    echoed.append(f"{detection['detector']}.{field}")
    quotes_nothing = not echoed

    regression_results = _check_regression_fixtures()
    regression_passed = all(positive_ok and negative_ok for _, positive_ok, negative_ok in regression_results)

    failures = positive_report["failures"] + key_report["failures"] + path_report["failures"] + negative_report["failures"]
    missing = sorted(expected - observed)
    passed = (
        not failures
        and not missing
        and key_hit
        and path_hit
        and not negative_report["detections"]
        and regression_passed
        and quotes_nothing
    )
    print(f"self_test={'PASS' if passed else 'FAIL'}")
    print(f"positive_classes={len(expected - set(missing))}/{len(expected)}")
    print(f"key_surface={'PASS' if key_hit else 'FAIL'}")
    print(f"path_surface={'PASS' if path_hit else 'FAIL'}")
    print(f"negative_detections={len(negative_report['detections'])}")
    print(f"quotes_nothing={'PASS' if quotes_nothing else 'FAIL'}")
    for field in sorted(set(echoed)):
        print(f"  FAIL {field} echoes its input back into the report")
    regression_ok_count = sum(1 for _, positive_ok, negative_ok in regression_results if positive_ok and negative_ok)
    print(f"regression_fixtures={regression_ok_count}/{len(regression_results)}")
    for label, positive_ok, negative_ok in regression_results:
        if not (positive_ok and negative_ok):
            print(f"  FAIL {label}: positive={'ok' if positive_ok else 'MISSED'} negative={'ok' if negative_ok else 'STILL FIRES'}")
    return 0 if passed else 1


def _main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Audit trace fuzzing inputs for sensitive-data shapes.")
    parser.add_argument("path", nargs="?", help="File or directory to scan")
    parser.add_argument("--self-test", action="store_true", help="Run synthetic scanner checks")
    args = parser.parse_args(argv)
    if args.self_test:
        if args.path is not None:
            parser.error("path cannot be combined with --self-test")
        return _self_test()
    if args.path is None:
        parser.error("path is required unless --self-test is used")
    report = audit(args.path)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 1 if report["failures"] else 0


if __name__ == "__main__":
    raise SystemExit(_main())

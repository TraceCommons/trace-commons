#!/usr/bin/env python3
"""Build reviewed localization resources from canonical, fixed Rust FFI tables.

Python standard library only. Never call this exporter on a daemon payload.
"""

import argparse
import copy
import ctypes
import hashlib
import json
from pathlib import Path
import plistlib
import re
import sys


ROOT = Path(__file__).resolve().parents[2]
CATALOG = ROOT / "localization/catalog.json"
RESOURCES = ROOT / "macos/Sources/TCLocalization/Resources"
GENERATED = ROOT / "localization/generated"

# Audited no-argument, owned-string exports ONLY. Never discover/call ABI symbols
# dynamically: many neighboring tc_* APIs return identifiers or user content.
REGISTRY = {
    "routing_copy": "tc_routing_copy",
    "consent_copy": "tc_consent_copy",
    "attach_copy": "tc_attach_copy",
    "private_inference_copy": "tc_private_inference_copy",
    "public_run_copy": "tc_public_run_copy",
    "skill_learning_copy": "tc_skill_learning_copy",
    "route_disclosure_unreadable_copy": "tc_route_disclosure_unreadable_copy",
    "certificate_detail_copy": "tc_certificate_detail_copy",
    "session_notification_copy": "tc_session_notification_copy",
    "source_settings_copy": "tc_source_settings_copy",
    "witness_copy": "tc_witness_copy",
    "onboarding_copy": "tc_onboarding_copy",
    "first_run_copy": "tc_first_run_copy_json",
    "contribution_mode_copy": "tc_contribution_mode_copy_json",
    "missions_disclosure_copy": "tc_missions_disclosure_copy_json",
    "monitor_traces_copy": "tc_monitor_traces_copy_json",
    "monitor_screens_copy": "tc_monitor_screens_copy_json",
    "shell_words_copy": "tc_shell_words_copy_json",
    "inference_connection_copy": "tc_inference_connection_copy_json",
    "payout_link_copy": "tc_payout_link_copy_json",
    "referral_copy": "tc_referral_copy_json",
    "account_devices_copy": "tc_account_devices_copy_json",
    "account_change_copy": "tc_account_change_copy_json",
    "set_up_again_copy": "tc_set_up_again_copy_json",
    "sent_traces_training_copy": "tc_sent_traces_training_copy_json",
    "privacy_scan_copy": "tc_privacy_scan_copy_json",
    "contributor_disclosure_copy": "tc_contributor_disclosure_copy_json",
    "managed_copy": "tc_managed_copy_json",
    "insights_copy": "tc_insights_copy_json",
    "nudge_copy": "tc_nudge_copy_json",
    "compute_copy": "tc_compute_copy_json",
    "legacy_migration_offer": "tc_legacy_migration_offer_json",
}

# Lists have been inspected in their owning Rust tables. New arrays fail closed
# until explicitly audited. Identifiers inside these static choices are excluded.
STATIC_LISTS = {
    "first_run_copy.rules.weekdays", "first_run_copy.rules.months",
    "contribution_mode_copy.choices",
    "public_run_copy.task_outcome_choices", "public_run_copy.feedback_choices",
    "public_run_copy.evidence_kind_choices",
    "public_run_copy.contribution_status_choices",
    "public_run_copy.history_status_labels", "public_run_copy.credit_outcome_labels",
    "public_run_copy.permitted_use_choices", "public_run_copy.reuse_permissions",
    "shell_words_copy.public_profile.published_lines",
    "shell_words_copy.public_profile.never_lines",
}
EXCLUDED = {
    "routing_copy.folder_note": "Machine-resolved folder; rendered by the keyed routing_folder_note presentation",
    "account_devices_copy.code_command": "Executable command, not prose",
    "first_run_copy.join.invite_placeholder": "Example invitation URL, not prose",
    "first_run_copy.uses.credit_scope": "Consent scope identifier used for permission decisions, not prose",
    "referral_copy.field_placeholder": "Referral identifier format, not prose",
    "witness_copy.url_placeholder": "Example URL, not prose",
    "witness_copy.signing_address_placeholder": "Example address syntax, not prose",
    "witness_copy.measurements_placeholder": "Measurement identifiers and syntax, not prose",
    "shell_words_copy.withdrawal.not_found_labels": "Wire identifiers, not prose",
    "skill_learning_copy.contract_summary_format": "Legacy positional format; macOS uses the keyed named-parameter skill presentation",
    "skill_learning_copy.model_budget_format": "Legacy positional format; macOS uses the keyed named-parameter skill presentation",
    "skill_learning_copy.review_budget_format": "Legacy positional format; macOS uses the keyed named-parameter skill presentation",
}

SEEDS = {
    "app.language.title": "Language",
    "app.language.system": "System default",
    "app.language.restart": "Language changes take effect after restarting Near.ai.",
    "app.window.first_run": "First run",
    "app.permission.apple_events": "Near.ai opens Private AI sessions in Terminal.",
    "privacy.residual.secrets": {
        "one": "{count} secret is still in this trace",
        "other": "{count} secrets are still in this trace",
    },
    "privacy.residual.unchecked": "This trace could not be checked for leftover secrets, so treat it as holding one.",
}

# Translator context is deliberately authored by surface, not inferred from text.
CONTEXTS = {
    "routing_copy": "Tools settings: describes only the observed first hop; do not strengthen privacy claims.",
    "consent_copy": "Consent decision: preserve exact scope, destination and consequences before the action.",
    "attach_copy": "Attached watcher: this window cannot stop the other watcher or read full trace text.",
    "private_inference_copy": "Private AI offer and settings: preserve limits of observed routing and attestation.",
    "public_run_copy": "Trace detail and public publication editor: distinguish publishing from contributing and withdrawing.",
    "skill_learning_copy": "Correction-derived skills editor: labels describe saved skill configuration, not user content.",
    "route_disclosure_unreadable_copy": "Unreadable disclosure: no permission may be inferred from missing information.",
    "certificate_detail_copy": "Certificate detail: verification at review is not a promise about future requests.",
    "session_notification_copy": "Trace review notification: the user still decides whether to send.",
    "source_settings_copy": "Source settings: distinguish reading local traces from permission to contribute them.",
    "witness_copy": "Witness and wallet settings: preserve verification limits and disclosure scope.",
    "onboarding_copy": "Onboarding: preserve watch-only versus enrolled contribution behavior.",
    "first_run_copy": "First-run screens: preserve permission scope, named placeholders, and action versus heading roles.",
    "contribution_mode_copy": "Folder contribution modes: Automatic sends finished traces without asking again.",
    "missions_disclosure_copy": "Mission permission disclosure: do not widen data-use consent.",
    "monitor_traces_copy": "Trace monitor: preserve pending, sent and review states; templates receive user values later.",
    "monitor_screens_copy": "Monitor navigation and settings: distinguish current state from available actions.",
    "shell_words_copy": "Shared shell screens: withdrawal and public profile consequences must remain explicit.",
    "inference_connection_copy": "Inference connection: authentication alone grants no contribution permission.",
    "payout_link_copy": "Payout organization link: preserve unlinking consequences and organization placeholder.",
    "referral_copy": "Referrals and credit split: preserve code, date and amount placeholders without claims of guaranteed credit.",
    "account_devices_copy": "Linked devices: preserve account linking and unlinking consequences; commands are not translatable.",
    "account_change_copy": "Account login change: preserve confirmation and account ownership boundaries.",
    "set_up_again_copy": "Destructive setup reset confirmation: preserve what is reset and what remains.",
    "sent_traces_training_copy": "Training consent for previously sent traces: preserve scope and the separate review decision.",
    "privacy_scan_copy": "Privacy scan: uncertainty is not a guarantee that a trace contains no secrets.",
    "contributor_disclosure_copy": "Shared disclosure bundle: preserve permissions, uncertainty and consequences; never translate wire keys.",
    "managed_copy": "Managed sessions: configuration labels and actions, never session bodies or commands.",
    "insights_copy": "Local Insights: observations and independent user reports are not verified task outcomes.",
    "nudge_copy": "Trace offer: preserve voluntary review and dismissal as distinct actions.",
    "compute_copy": "Compute settings: compute permission is independent of traces; preserve no-pay and shutdown limitations.",
    "legacy_migration_offer": "Account migration offer: preserve voluntary dismissal, identity migration and sign-in scope.",
    "app": "Application language preference and first-run window; language selection requires restart.",
    "privacy": "Residual-secret warning: an unavailable scan must never be described as zero secrets.",
}
SENSITIVE = {
    "routing_copy", "source_settings_copy", "consent_copy", "private_inference_copy", "public_run_copy",
    "route_disclosure_unreadable_copy", "certificate_detail_copy",
    "session_notification_copy", "witness_copy", "onboarding_copy", "first_run_copy",
    "contribution_mode_copy", "missions_disclosure_copy", "shell_words_copy",
    "inference_connection_copy", "payout_link_copy", "account_devices_copy",
    "account_change_copy", "set_up_again_copy", "sent_traces_training_copy",
    "privacy_scan_copy", "contributor_disclosure_copy", "privacy",
    "compute_copy", "legacy_migration_offer",
}
KEY = re.compile(r"[a-z][a-z0-9_]*(?:\.(?:[A-Za-z_][A-Za-z0-9_-]*|[0-9]+))+")
LOCALE = re.compile(r"[a-z]{2,3}(?:-[A-Z][a-z]{3})?(?:-(?:[A-Z]{2}|[0-9]{3}))?")
NAME = re.compile(r"[a-z][a-z0-9_]*")
PLACEHOLDER = re.compile(r"\{([a-z][a-z0-9_]*)\}")
CATEGORIES = {"zero", "one", "two", "few", "many", "other"}
# Protected snippets are not changed by pseudo or translations. Keep executable
# snippets in backticks, URLs, paths, brands, and these known command spellings.
PROTECTED = re.compile(
    r"`[^`]+`|https?://[^\s<>]+|(?<!\w)(?:~?/|\./)[\w./-]+|--[a-z][a-z-]*|"
    r"\b(?:tracecommons\.ai(?:/[\w./-]+)?|daemon-settings\.json|"
    r"NEAR AI|Near\.ai|IronWire|Claude Code|Gemini CLI|Codex|Cline|OpenCode|Holonear|Terminal|"
    r"ironwire(?: [a-z-]+)+|trace-commons-contributor(?: [a-z-]+)+|"
    r"trace-commons(?: [a-z-]+)+)\b"
)


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def source_hash(english):
    """Strings hash exact UTF-8; plurals hash canonical compact sorted JSON."""
    source = english if isinstance(english, str) else json.dumps(
        english, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(source.encode("utf-8")).hexdigest()


def parameters(text):
    if not isinstance(text, str):
        raise ValueError("message text must be a string")
    names = PLACEHOLDER.findall(text)
    if "{" in PLACEHOLDER.sub("", text) or "}" in PLACEHOLDER.sub("", text):
        raise ValueError("invalid named placeholder")
    if any(ord(c) < 32 and c not in "\n\r\t" for c in text):
        raise ValueError("control character in message")
    return set(names)


def forms(value, plural):
    if plural:
        if not isinstance(value, dict) or "other" not in value or not set(value) <= CATEGORIES:
            raise ValueError("plural requires other and only zero/one/two/few/many/other")
        return list(value.values())
    if not isinstance(value, str):
        raise ValueError("nonplural value must be string")
    return [value]


def excluded_reason(path):
    for prefix, reason in EXCLUDED.items():
        if path == prefix or path.startswith(prefix + "."):
            return reason
    if path.startswith("public_run_copy.") and path.rsplit(".", 1)[-1] in {"value", "permission"}:
        return "Wire choice identifier, not prose"
    if re.fullmatch(r"contribution_mode_copy\.choices\.[0-9]+\.mode", path):
        return "Wire contribution mode, not prose"
    if re.fullmatch(r"(?:source_settings_copy|contributor_disclosure_copy\.source_settings)\.tools\.[^.]+\.key", path):
        return "Session source identifier, not prose"
    if path.rsplit(".", 1)[-1].endswith(("_glyph", "_tone")):
        return "Presentation symbol or tone identifier, not prose"
    return None


def flatten(value, path, messages, excluded):
    reason = excluded_reason(path)
    if reason:
        excluded[path] = reason
    elif isinstance(value, dict):
        for field, child in sorted(value.items()):
            flatten(child, path + "." + field, messages, excluded)
    elif isinstance(value, list):
        if path not in STATIC_LISTS:
            raise ValueError(f"Unaudited list at {path}")
        for index, child in enumerate(value):
            flatten(child, f"{path}.{index}", messages, excluded)
    elif isinstance(value, str):
        if not KEY.fullmatch(path):
            raise ValueError(f"Invalid export key: {path}")
        # Entire command/URL/path values require explicit field exclusions.
        if re.match(r"^(?:https?://|~/|/|\./)", value):
            raise ValueError(f"Non-prose value needs explicit exclusion: {path}")
        messages[path] = value
    else:
        excluded[path] = "Non-string metadata, not prose"


def snapshot(library):
    lib = ctypes.CDLL(str(Path(library).resolve()))
    free = lib.tc_string_free
    free.argtypes = [ctypes.c_void_p]
    free.restype = None
    messages, excluded = {}, {}
    for namespace, symbol in REGISTRY.items():
        function = getattr(lib, symbol)
        function.argtypes = []
        function.restype = ctypes.c_void_p  # c_char_p loses the owned pointer
        pointer = function()
        if not pointer:
            raise ValueError(f"NULL copy table: {symbol}")
        try:
            table = json.loads(ctypes.string_at(pointer).decode("utf-8"))
        finally:
            free(pointer)
        if not isinstance(table, dict) or not table:
            raise ValueError(f"Empty or invalid copy table: {symbol}")
        flatten(table, namespace, messages, excluded)
    function = lib.tc_message_templates_json
    function.argtypes = []
    function.restype = ctypes.c_void_p
    pointer = function()
    if not pointer:
        raise ValueError("NULL presentation templates")
    try:
        templates = json.loads(ctypes.string_at(pointer).decode("utf-8"))
    finally:
        free(pointer)
    def collect(node):
        if isinstance(node, dict):
            if "key" in node and "english" in node:
                key, english = node["key"], node["english"]
                if key in messages and messages[key] != english:
                    raise ValueError(f"Presentation key has inconsistent sources: {key}")
                messages[key] = english
            for child in node.values():
                collect(child)
        elif isinstance(node, list):
            for child in node:
                collect(child)
    collect(templates)
    return {"version": 1, "registry": REGISTRY, "messages": messages, "excluded": excluded}


def source_messages(exported, authored=None, plurals=None):
    live = {**exported["messages"], **SEEDS}
    for key, definition in (authored or {}).items():
        if key in live:
            raise ValueError(f"Authored key shadows canonical English: {key}")
        live[key] = definition["english"]
    for key, definition in (plurals or {}).items():
        if key in live:
            raise ValueError(f"Plural key shadows canonical English: {key}")
        parameter = definition.get("countParameter")
        live[key] = {form: exported["messages"][definition[form]] for form in ("one", "other")}
        if parameter:
            live[key] = {form: text.replace("{" + parameter + "}", "{count}") for form, text in live[key].items()}
    return live


def import_messages(catalog, exported, authored=None, plurals=None):
    result = copy.deepcopy(catalog)
    live = source_messages(exported, authored, plurals)
    # Retired table fields must no longer be eligible for runtime substitution.
    for key in list(result["messages"]):
        if (key.split(".")[0] in REGISTRY or key.startswith(("message.", "plural."))) and key not in live:
            del result["messages"][key]
    for key, english in live.items():
        namespace = key.split(".")[0]
        names = set().union(*(parameters(t) for t in forms(english, isinstance(english, dict))))
        if isinstance(english, dict):
            names.add("count")
        if key not in result["messages"]:
            result["messages"][key] = {
                "context": f"{CONTEXTS.get(namespace, 'Core-selected presentation: preserve scope, uncertainty and consequences.')} Field: {key}.",
                "review": "sensitive" if (key in exported["messages"] and namespace not in REGISTRY) or key == "app.permission.apple_events" or namespace in SENSITIVE or re.search(
                    r"consent|disclos|priva|secret|withdraw|delet|destruct|clear|remove|forget", key
                ) else "standard",
                "translations": {},
            }
        if authored and key in authored:
            for field in ("context", "review"):
                result["messages"][key][field] = authored[key][field]
        result["messages"][key]["english"] = english
        result["messages"][key]["parameters"] = sorted(names)
    return result


def report(catalog):
    totals = {}
    for locale in sorted(set(catalog["locales"]) - {"en"}):
        counts = {"missing": 0, "draft": 0, "outdated": 0}
        for message in catalog["messages"].values():
            translation = message.get("translations", {}).get(locale)
            if translation is None:
                counts["missing"] += 1
            else:
                counts["draft"] += translation.get("state") != "reviewed"
                counts["outdated"] += translation.get("sourceHash") != source_hash(message["english"])
        totals[locale] = counts
    return totals


def validate(catalog, gate_enabled=True):
    if catalog.get("version") != 1 or catalog.get("sourceLanguage") != "en":
        raise ValueError("Expected version 1 and sourceLanguage en")
    locales = catalog["locales"]
    if locales.get("en") != {"name": "English", "enabled": True}:
        raise ValueError("English must always be enabled")
    for locale, info in locales.items():
        if not LOCALE.fullmatch(locale) or locale in {"en-XA", "ar-XB"}:
            raise ValueError(f"Invalid production locale: {locale}")
        if not isinstance(info.get("name"), str) or not info["name"].strip() or type(info.get("enabled")) is not bool:
            raise ValueError(f"Invalid locale metadata: {locale}")
    if not isinstance(catalog["messages"], dict) or not catalog["messages"]:
        raise ValueError("Catalog messages must be a nonempty object")
    for key, message in catalog["messages"].items():
        try:
            if not KEY.fullmatch(key):
                raise ValueError("invalid key")
            if not isinstance(message["context"], str) or not message["context"].strip():
                raise ValueError("translator context required")
            if message["review"] not in {"standard", "sensitive"}:
                raise ValueError("invalid review classification")
            names = message["parameters"]
            if not isinstance(names, list) or any(not isinstance(n, str) or not NAME.fullmatch(n) for n in names) or len(names) != len(set(names)):
                raise ValueError("invalid parameter names")
            english = message["english"]
            plural = isinstance(english, dict)
            texts = forms(english, plural)
            if plural and (set(english) != {"one", "other"} or "count" not in names):
                raise ValueError("English plural needs one, other and numeric count")
            for text in texts:
                found = parameters(text)
                if found != set(names) and not (plural and found == set(names) - {"count"}):
                    raise ValueError("English placeholders differ from parameters")
                if plural and text.count("{count}") > 1:
                    raise ValueError("plural requires at most one count per form")
            translations = message["translations"]
            if not isinstance(translations, dict):
                raise ValueError("translations must be an object")
            for locale, translation in translations.items():
                if locale not in locales or locale == "en":
                    raise ValueError(f"unregistered translation locale: {locale}")
                if translation["state"] not in {"draft", "reviewed"} or not re.fullmatch(r"[0-9a-f]{64}", translation["sourceHash"]):
                    raise ValueError("invalid translation review state or source hash")
                current = translation["sourceHash"] == source_hash(english)
                translated_plural = isinstance(translation["value"], dict)
                if current and translated_plural != plural:
                    raise ValueError("translation plural shape differs from source")
                # A preserved stale translation may describe an OLD parameter
                # contract. Still validate its syntax, but never ship it or
                # require it to match the new contract before human review.
                for text in forms(translation["value"], translated_plural):
                    found = parameters(text)
                    if current and ((all(source == "" for source in texts) and text != "")
                                    or (any(source != "" for source in texts) and not text.strip())):
                        raise ValueError("translation must preserve suppressed copy and cannot erase visible copy")
                    if current and found != set(names) and not (plural and found == set(names) - {"count"}):
                        raise ValueError("translation placeholders differ from parameters")
                    if translated_plural and text.count("{count}") > 1:
                        raise ValueError("plural requires at most one count per form")
                    protected = set().union(*(PROTECTED.findall(source) for source in texts)) if current else set()
                    for token in protected:
                        if token not in text:
                            raise ValueError(f"protected token changed: {token}")
        except (ValueError, TypeError, KeyError) as error:
            raise ValueError(f"{key}: {error}") from error
    for locale, counts in report(catalog).items():
        if gate_enabled and locales[locale]["enabled"] and any(counts.values()):
            raise ValueError(f"Enabled locale {locale} is incomplete/unreviewed/outdated: {counts}")


def check_english(catalog, exported, authored=None, plurals=None):
    expected = source_messages(exported, authored, plurals)
    actual = {key: message["english"] for key, message in catalog["messages"].items()
              if key.split(".")[0] in REGISTRY or key in expected or key.startswith(("message.", "plural."))}
    if expected != actual:
        keys = sorted(key for key in expected.keys() | actual.keys() if expected.get(key) != actual.get(key))
        raise ValueError("English differs from live FFI/required seeds; run --import: " + ", ".join(keys))


def strings_quote(text):
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t") + '"'


def plural_template(text):
    return text.replace("%", "%%").replace("{count}", "%lld")


def native_files(messages, locale):
    strings = ["/* Generated by scripts/localization/catalog.py; do not edit. */"]
    plurals = {}
    for key, message in sorted(messages.items()):
        value = message["english"] if locale == "en" else message["translations"][locale]["value"]
        if isinstance(value, dict):
            plurals[key] = {
                "NSStringLocalizedFormatKey": "%#@count@",
                "count": {
                    "NSStringFormatSpecTypeKey": "NSStringPluralRuleType",
                    "NSStringFormatValueTypeKey": "lld",
                    **{category: plural_template(text) for category, text in value.items()},
                },
            }
        else:
            strings.append(f"{strings_quote(key)} = {strings_quote(value)};")
    outputs = {
        f"{locale}.lproj/Localizable.strings": ("\n".join(strings) + "\n").encode("utf-8"),
        f"{locale}.lproj/Localizable.stringsdict": plistlib.dumps(plurals, sort_keys=True),
    }
    if "app.permission.apple_events" in messages:
        message = messages["app.permission.apple_events"]
        value = message["english"] if locale == "en" else message["translations"][locale]["value"]
        outputs[f"{locale}.lproj/InfoPlist.strings"] = (
            '/* Generated; copy into the main app bundle for OS permission UI. */\n'
            f'"NSAppleEventsUsageDescription" = {strings_quote(value)};\n'
        ).encode("utf-8")
    return outputs


def xcstrings(catalog):
    entries = {}
    for key, message in sorted(catalog["messages"].items()):
        localizations = {}
        values = {"en": (message["english"], "translated")}
        for locale, translation in message["translations"].items():
            state = "translated" if translation["state"] == "reviewed" and translation["sourceHash"] == source_hash(message["english"]) else "needs_review"
            values[locale] = (translation["value"], state)
        for locale, (value, state) in values.items():
            if isinstance(value, dict):
                localizations[locale] = {"variations": {"plural": {
                    category: {"stringUnit": {"state": state, "value": plural_template(text)}}
                    for category, text in value.items()
                }}}
            else:
                localizations[locale] = {"stringUnit": {"state": state, "value": value}}
        entries[key] = {"comment": f"[{message['review']}] {message['context']}",
                        "extractionState": "manual", "localizations": localizations}
    return {"sourceLanguage": "en", "strings": entries, "version": "1.0"}


def production_files(catalog, resources=RESOURCES, generated=GENERATED):
    validate(catalog)
    enabled = {locale: info for locale, info in catalog["locales"].items() if info["enabled"]}
    metadata = copy.deepcopy(catalog)
    metadata["locales"] = enabled
    for message in metadata["messages"].values():
        message["translations"] = {locale: translation for locale, translation in message["translations"].items() if locale in enabled}
    outputs = {resources / "Catalog.json": json_bytes(metadata)}
    for locale in sorted(enabled):
        outputs.update({resources / name: data for name, data in native_files(catalog["messages"], locale).items()})
    outputs[generated / "Localizable.xcstrings"] = json_bytes(xcstrings(catalog))
    outputs[generated / "locales.plist"] = plistlib.dumps(sorted(enabled))
    return outputs


ACCENTS = str.maketrans("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ", "áƀçďéƒĝĥíĵķľɱñóþɋŕšţúṽŵẋýžÁƁÇĎÉƑĜĤÍĴĶĽṀÑÓÞɊŔŠŢÚṼŴẊÝŽ")


def pseudo_text(text, rtl=False):
    # Split only fixed prose. Runtime substitution must happen AFTER lookup;
    # never pass a user's completed sentence or trace through this function.
    token = re.compile(f"({PLACEHOLDER.pattern}|{PROTECTED.pattern})")
    pieces, start = [], 0
    for match in token.finditer(text):
        prose = text[start:match.start()]
        pieces.append(prose if rtl else prose.translate(ACCENTS))
        pieces.append(match.group())
        start = match.end()
    prose = text[start:]
    pieces.append(prose if rtl else prose.translate(ACCENTS))
    if not text:
        return text
    result = "".join(pieces)
    return "\u2067[" + result + "]\u2069" if rtl else "[" + result + " ~" + "~" * (len(text) // 3) + "]"


def pseudo_files(catalog, output):
    # This standalone DEVELOPMENT bundle is never passed to production_files.
    # Synthetic reviewed states make the schema complete for debug selection;
    # they are not assertions of human/native-speaker review or shipping status.
    metadata = copy.deepcopy(catalog)
    metadata["locales"] = {"en": {"name": "English", "enabled": True}}
    for message in metadata["messages"].values():
        message["translations"] = {}
        message["context"] = "[DEVELOPMENT SYNTHETIC — NOT HUMAN REVIEWED] " + message["context"]
    outputs = {output / name: data for name, data in native_files(metadata["messages"], "en").items()}
    for locale, rtl in (("en-XA", False), ("ar-XB", True)):
        metadata["locales"][locale] = {
            "name": "Development RTL pseudo" if rtl else "Development expanded pseudo", "enabled": True}
        for message in metadata["messages"].values():
            english = message["english"]
            value = {category: pseudo_text(text, rtl) for category, text in english.items()} if isinstance(english, dict) else pseudo_text(english, rtl)
            message["translations"][locale] = {"value": value, "state": "reviewed", "sourceHash": source_hash(english)}
        outputs.update({output / name: data for name, data in native_files(metadata["messages"], locale).items()})
    outputs[output / "Catalog.json"] = json_bytes(metadata)
    return outputs


def write_or_check(outputs, check=False, resource_root=None):
    stale = []
    if resource_root and resource_root.exists():
        # Only these names are generator-owned. Never remove another resource.
        existing = (set(resource_root.glob("*.lproj/Localizable.strings"))
                    | set(resource_root.glob("*.lproj/Localizable.stringsdict"))
                    | set(resource_root.glob("*.lproj/InfoPlist.strings")))
        for path in sorted(existing - outputs.keys()):
            if check:
                stale.append(str(path))
            else:
                path.unlink()
    for path, data in outputs.items():
        if check:
            if not path.is_file() or path.read_bytes() != data:
                stale.append(str(path))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    if stale:
        raise ValueError("Generated outputs differ: " + ", ".join(stale))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog", type=Path, default=CATALOG)
    parser.add_argument("--library", type=Path, help="Trusted, freshly built contributor FFI shared library")
    parser.add_argument("--import", dest="import_live", action="store_true", help="Mirror live English; preserve context/translations")
    parser.add_argument("--snapshot", type=Path, help="Write live export and field exclusion report here")
    parser.add_argument("--check", action="store_true", help="Require live English and byte-identical generated outputs")
    parser.add_argument("--report", action="store_true", help="Print missing/draft/outdated totals (may overlap); do not generate")
    parser.add_argument("--pseudo", type=Path, metavar="DIRECTORY", help="Development-only pseudo bundles, never production output")
    parser.add_argument("--resources", type=Path, default=RESOURCES)
    parser.add_argument("--generated", type=Path, default=GENERATED)
    args = parser.parse_args(argv)
    if (args.import_live or args.snapshot or args.check) and not args.library:
        parser.error("--import, --snapshot and --check require --library (live FFI verification)")
    if args.check and (args.import_live or args.report):
        parser.error("--check cannot import or report")
    if args.pseudo:
        output = args.pseudo.resolve()
        production = RESOURCES.resolve()
        requested = args.resources.resolve()
        if output == production or production in output.parents or output in production.parents or output == requested or requested in output.parents or output in requested.parents:
            parser.error("pseudo output must be separate from production resources")
    try:
        catalog = json.loads(args.catalog.read_text("utf-8")) if args.catalog.exists() else {
            "version": 1, "sourceLanguage": "en", "locales": {"en": {"name": "English", "enabled": True}}, "messages": {}}
        authored_path = ROOT / "localization/macos-messages.json"
        plurals_path = ROOT / "localization/plurals.json"
        authored = json.loads(authored_path.read_text("utf-8")) if authored_path.exists() else {}
        plurals = json.loads(plurals_path.read_text("utf-8")) if plurals_path.exists() else {}
        exported = snapshot(args.library) if args.library else None
        if args.import_live:
            catalog = import_messages(catalog, exported, authored, plurals)
        validate(catalog, gate_enabled=not args.report)
        if exported:
            check_english(catalog, exported, authored, plurals)
            print(f"Live FFI: {len(REGISTRY)} tables, {len(exported['messages'])} prose fields; {len(exported['excluded'])} excluded fields")
            for path, reason in sorted(exported["excluded"].items()):
                print(f"  excluded {path}: {reason}")
        if args.report:
            print(json.dumps(report(catalog), indent=2, sort_keys=True))
            return 0
        if args.pseudo:
            write_or_check(pseudo_files(catalog, args.pseudo), args.check, args.pseudo)
            return 0
        outputs = production_files(catalog, args.resources, args.generated)
        if args.import_live:
            outputs[args.catalog] = json_bytes(catalog)
        if args.snapshot:
            outputs[args.snapshot] = json_bytes(exported)
        write_or_check(outputs, args.check, args.resources)
        print(f"{'Checked' if args.check else 'Generated'} {len(catalog['messages'])} messages; enabled: " + ", ".join(locale for locale, info in catalog["locales"].items() if info["enabled"]))
        return 0
    except (ValueError, KeyError, TypeError, AttributeError, OSError) as error:
        print(f"localization: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

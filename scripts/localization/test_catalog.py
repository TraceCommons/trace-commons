"""Run: python3 -m unittest discover -s scripts/localization -v"""

import copy
import ctypes
import hashlib
import io
import json
from pathlib import Path
import plistlib
import re
import tempfile
import unittest
from unittest.mock import patch

import catalog as tool


def fixture(english="Hello {name}, from {place}"):
    return {
        "version": 1, "sourceLanguage": "en",
        "locales": {"en": {"name": "English", "enabled": True},
                    "fr": {"name": "Français", "enabled": False}},
        "messages": {"app.test.label": {
            "english": english, "context": "Greeting. Field: app.test.label.",
            "parameters": sorted(set().union(*(tool.parameters(t) for t in tool.forms(english, isinstance(english, dict))))),
            "review": "standard", "translations": {},
        }},
    }


def add_translation(catalog, value, state="reviewed", source=None, locale="fr"):
    message = catalog["messages"]["app.test.label"]
    message["translations"][locale] = {
        "value": value, "state": state,
        "sourceHash": tool.source_hash(message["english"]) if source is None else source,
    }


class ValidationTests(unittest.TestCase):
    def test_reordered_named_placeholders(self):
        value = fixture()
        add_translation(value, "De {place}, bonjour {name}")
        value["locales"]["fr"]["enabled"] = True
        tool.validate(value)

    def test_translations_cannot_remove_disclosures_or_restore_suppressed_copy(self):
        for source, translated in (("Consent is required.", ""), ("Consent is required.", "  "),
                                   ("", "Restored notice")):
            value = fixture(source)
            add_translation(value, translated)
            with self.subTest(source=source), self.assertRaisesRegex(ValueError, "suppressed copy"):
                tool.validate(value)
        value = fixture("")
        add_translation(value, "")
        tool.validate(value)

    def test_bad_placeholders(self):
        for text in ("{0}", "{}", "{name", "{Name}", "{name:02}", "{name!r}", "{{name}}"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                tool.parameters(text)
        for text in ("{name}", "{name} {other}", "No parameters"):
            value = fixture()
            add_translation(value, text)
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, "placeholders"):
                tool.validate(value)

    def test_invalid_keys_locales_and_schema(self):
        for key in ("plain", "app..title", "app./path", "app.bad key", "app.title\n"):
            value = fixture()
            value["messages"][key] = value["messages"].pop("app.test.label")
            with self.subTest(key=key), self.assertRaises(ValueError):
                tool.validate(value)
        for locale in ("../fr", "fr_FR", "fr-foo", "en-XA", "ar-XB", "FR", "zh-Hans-CN/other"):
            value = fixture()
            value["locales"][locale] = {"name": "Test", "enabled": False}
            with self.subTest(locale=locale), self.assertRaises(ValueError):
                tool.validate(value)
        for locale in ("zh-Hans", "zh-Hant-TW", "pt-BR", "es-419"):
            value = fixture()
            value["locales"][locale] = {"name": "Test", "enabled": False}
            tool.validate(value)

    def test_review_hash_and_complete_locale_gate(self):
        for state, source in (("draft", None), ("reviewed", "0" * 64)):
            value = fixture()
            add_translation(value, "{place}: {name}", state, source)
            tool.validate(value)  # A nonshipping locale can be in progress.
            value["locales"]["fr"]["enabled"] = True
            with self.assertRaisesRegex(ValueError, "Enabled locale"):
                tool.validate(value)
        value = fixture()
        value["locales"]["fr"]["enabled"] = True
        with self.assertRaisesRegex(ValueError, "Enabled locale"):
            tool.validate(value)

    def test_hash_is_exact_and_plural_order_independent(self):
        self.assertEqual(tool.source_hash("Hello"), hashlib.sha256(b"Hello").hexdigest())
        self.assertNotEqual(tool.source_hash("Hello"), tool.source_hash("Hello "))
        self.assertEqual(tool.source_hash({"one": "one", "other": "other"}),
                         tool.source_hash({"other": "other", "one": "one"}))

    def test_report_counts_overlap(self):
        value = fixture()
        self.assertEqual(tool.report(value)["fr"], {"missing": 1, "draft": 0, "outdated": 0})
        add_translation(value, "{name} {place}", "draft", "0" * 64)
        self.assertEqual(tool.report(value)["fr"], {"missing": 0, "draft": 1, "outdated": 1})

    def test_native_plural_categories_count_and_percent_escaping(self):
        value = fixture({"one": "{count} item, 50% for {name}", "other": "{count} items, 50% for {name}"})
        translated = {category: "{name}: {count} éléments 50% %lld %@" for category in tool.CATEGORIES}
        add_translation(value, translated)
        value["locales"]["fr"]["enabled"] = True
        tool.validate(value)
        data = tool.native_files(value["messages"], "fr")
        entry = plistlib.loads(data["fr.lproj/Localizable.stringsdict"])["app.test.label"]
        self.assertEqual(entry["NSStringLocalizedFormatKey"], "%#@count@")
        count = entry["count"]
        self.assertEqual(count["NSStringFormatSpecTypeKey"], "NSStringPluralRuleType")
        self.assertEqual(count["NSStringFormatValueTypeKey"], "lld")
        for category in tool.CATEGORIES:
            self.assertEqual(count[category], "{name}: %lld éléments 50%% %%lld %%@")
        self.assertNotIn(b"app.test.label", data["fr.lproj/Localizable.strings"])
        xc = tool.xcstrings(value)["strings"]["app.test.label"]["localizations"]["fr"]
        self.assertEqual(xc["variations"]["plural"]["other"]["stringUnit"]["value"], count["other"])

    def test_plural_rejects_missing_other_unknown_forms_and_wrong_count(self):
        for forms in ({"one": "{count}"}, {"other": "{count}", "dual": "{count}"},
                      {"other": "{count} {count}"}):
            value = fixture({"one": "{count} item", "other": "{count} items"})
            add_translation(value, forms)
            with self.subTest(forms=forms), self.assertRaises(ValueError):
                tool.validate(value)
        value = fixture({"one": "{n}", "other": "{n}"})
        with self.assertRaisesRegex(ValueError, "numeric count"):
            tool.validate(value)

    def test_plural_forms_can_omit_count_but_not_other_parameters(self):
        value = fixture({"one": "Once for {name}", "other": "{count} times for {name}"})
        add_translation(value, {"one": "Une fois pour {name}", "other": "{count} fois pour {name}"})
        tool.validate(value)
        value["messages"]["app.test.label"]["translations"]["fr"]["value"]["one"] = "Une fois"
        with self.assertRaisesRegex(ValueError, "placeholders"):
            tool.validate(value)

    def test_source_derived_plural_keeps_canonical_templates(self):
        exported = {"messages": {"table.one": "Once for {name}", "table.many": "{n} times for {name}"}}
        plurals = {"plural.example.times": {"one": "table.one", "other": "table.many", "countParameter": "n"}}
        result = tool.import_messages(fixture(), exported, plurals=plurals)
        message = result["messages"]["plural.example.times"]
        self.assertEqual(message["english"], {"one": "Once for {name}", "other": "{count} times for {name}"})
        self.assertEqual(message["parameters"], ["count", "name"])
        tool.check_english(result, exported, plurals=plurals)
        exported["messages"]["table.one"] = "Changed for {name}"
        with self.assertRaisesRegex(ValueError, "live FFI"):
            tool.check_english(result, exported, plurals=plurals)

    def test_protected_tokens_cannot_be_translated(self):
        value = fixture("Run `trace-commons-contributor login --code {code}` at https://example.com")
        add_translation(value, "Run `autre --code {code}` at https://example.com")
        with self.assertRaisesRegex(ValueError, "protected token"):
            tool.validate(value)


class BoundaryTests(unittest.TestCase):
    def test_macos_literal_localization_keys_exist(self):
        catalog = json.loads(tool.CATALOG.read_text("utf-8"))
        references = []
        for path in (tool.ROOT / "macos/Sources").rglob("*.swift"):
            for key in re.findall(r'Localization\.current\.text\(\s*"([^"\\]+)"(?=\s*[,\)])', path.read_text("utf-8")):
                references.append((path.relative_to(tool.ROOT), key))
        self.assertGreater(len(references), 40, "scan must cover the actual consumers")
        missing = [(str(path), key) for path, key in references if key not in catalog["messages"]]
        self.assertEqual(missing, [], "a typo must not ship a message key as UI text")

    def test_permission_and_source_identifiers_never_enter_translatable_messages(self):
        messages, excluded = {}, {}
        tool.flatten({"uses": {"credit_scope": "model_training", "title": "Data uses"}},
                     "first_run_copy", messages, excluded)
        tool.flatten({"refused_glyph": "⊘", "refused_tone": "refused", "title": "Review"},
                     "witness_copy.admission", messages, excluded)
        self.assertNotIn("witness_copy.admission.refused_glyph", messages)
        self.assertNotIn("witness_copy.admission.refused_tone", messages)
        self.assertEqual(messages["witness_copy.admission.title"], "Review")
        for namespace in ("source_settings_copy", "contributor_disclosure_copy.source_settings"):
            tool.flatten({"tools": {"claude-code": {"key": "claude", "label": "Claude Code"}}},
                         namespace, messages, excluded)
            self.assertNotIn(namespace + ".tools.claude-code.key", messages)
            self.assertIn(namespace + ".tools.claude-code.key", excluded)
            self.assertEqual(messages[namespace + ".tools.claude-code.label"], "Claude Code")
        self.assertNotIn("first_run_copy.uses.credit_scope", messages)
        self.assertIn("first_run_copy.uses.credit_scope", excluded)
        self.assertEqual(messages["first_run_copy.uses.title"], "Data uses")

    def test_registry_covers_fixed_copy_exports_without_arbitrary_apis(self):
        source = tool.ROOT / "crates/trace-commons-contributor-ffi/src"
        symbols = set()
        for name in ("lib.rs", "insights.rs", "nudge.rs", "compute.rs"):
            symbols.update(re.findall(r'^pub extern "C" fn (tc_\w*copy\w*)\(\) -> \*mut c_char',
                                     (source / name).read_text(), re.MULTILINE))
        symbols.add("tc_legacy_migration_offer_json")
        self.assertEqual(set(tool.REGISTRY.values()), symbols)
        for namespace, symbol in tool.REGISTRY.items():
            self.assertEqual(namespace, symbol.removeprefix("tc_").removesuffix("_json"))
        self.assertNotIn("tc_install_guides_json", tool.REGISTRY.values())

    def test_report_cli_can_diagnose_invalid_enabled_locale_without_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "catalog.json"
            value = fixture()
            value["locales"]["fr"]["enabled"] = True
            path.write_bytes(tool.json_bytes(value))
            before = path.read_bytes()
            output = io.StringIO()
            with patch("sys.stdout", output):
                self.assertEqual(tool.main(["--catalog", str(path), "--report"]), 0)
            self.assertEqual(json.loads(output.getvalue())["fr"]["missing"], 1)
            self.assertEqual(path.read_bytes(), before)

    def test_import_preserves_context_and_translation_without_reviewing(self):
        value = fixture("Old")
        value["messages"]["routing_copy.title"] = value["messages"].pop("app.test.label")
        message = value["messages"]["routing_copy.title"]
        message["translations"]["fr"] = {"value": "Ancien", "state": "draft", "sourceHash": tool.source_hash("Old")}
        original = copy.deepcopy(message)
        result = tool.import_messages(value, {"messages": {"routing_copy.title": "New"}})
        updated = result["messages"]["routing_copy.title"]
        self.assertEqual(updated["context"], original["context"])
        self.assertEqual(updated["translations"], original["translations"])
        self.assertEqual(updated["english"], "New")
        self.assertEqual(result["messages"]["privacy.residual.secrets"]["review"], "sensitive")
        self.assertEqual(result["messages"]["app.language.title"]["translations"], {})
        self.assertEqual(value["messages"]["routing_copy.title"], original)

    def test_changed_source_contract_keeps_disabled_stale_translation(self):
        value = fixture("Old {name}")
        add_translation(value, "Ancien {name}")
        message = value["messages"].pop("app.test.label")
        value["messages"]["routing_copy.title"] = message
        result = tool.import_messages(value, {"messages": {"routing_copy.title": "New {tool}"}})
        tool.validate(result)
        updated = result["messages"]["routing_copy.title"]
        self.assertEqual(updated["translations"]["fr"]["value"], "Ancien {name}")
        self.assertEqual(updated["parameters"], ["tool"])
        self.assertEqual(tool.report(result)["fr"]["outdated"], 1)
        updated["translations"]["fr"]["sourceHash"] = tool.source_hash(updated["english"])
        with self.assertRaisesRegex(ValueError, "placeholders"):
            tool.validate(result)

    def test_retired_fields_removed_and_live_english_enforced(self):
        value = tool.import_messages(fixture(), {"messages": {"routing_copy.title": "Words"}})
        tool.check_english(value, {"messages": {"routing_copy.title": "Words"}})
        value["messages"]["routing_copy.title"]["english"] = "Other"
        with self.assertRaisesRegex(ValueError, "live FFI"):
            tool.check_english(value, {"messages": {"routing_copy.title": "Words"}})
        result = tool.import_messages(value, {"messages": {}})
        self.assertNotIn("routing_copy.title", result["messages"])

    def test_static_lists_exclude_wire_values_and_no_arbitrary_lists(self):
        messages, excluded = {}, {}
        tool.flatten({"task_outcome_choices": [{"value": "success", "label": "Done"}]},
                     "public_run_copy", messages, excluded)
        self.assertEqual(messages, {"public_run_copy.task_outcome_choices.0.label": "Done"})
        self.assertIn("public_run_copy.task_outcome_choices.0.value", excluded)
        with self.assertRaisesRegex(ValueError, "Unaudited list"):
            tool.flatten({"user_messages": ["private body"]}, "public_run_copy", {}, {})
        for key in tool.EXCLUDED:
            messages, excluded = {}, {}
            tool.flatten("do not translate", key, messages, excluded)
            self.assertFalse(messages)
            self.assertIn(key, excluded)

    def test_production_never_contains_partial_or_pseudo_locale(self):
        value = fixture()
        add_translation(value, "{place}: {name}", "draft")
        files = tool.production_files(value, Path("resources"), Path("generated"))
        metadata = json.loads(files[Path("resources/Catalog.json")])
        self.assertEqual(set(metadata["locales"]), {"en"})
        self.assertEqual(metadata["messages"]["app.test.label"]["translations"], {})
        self.assertEqual(plistlib.loads(files[Path("generated/locales.plist")]), ["en"])
        self.assertFalse(any("fr.lproj" in str(path) for path in files))
        self.assertIn("fr", json.loads(files[Path("generated/Localizable.xcstrings")])["strings"]["app.test.label"]["localizations"])

    def test_pseudo_preserves_parameters_commands_paths_and_empty_suppression(self):
        text = "Run `trace-commons-contributor login --code {code}` in ~/folder for {name} with NEAR AI"
        for rtl in (False, True):
            localized = tool.pseudo_text(text, rtl)
            self.assertEqual(tool.parameters(localized), {"code", "name"})
            for token in ("`trace-commons-contributor login --code {code}`", "~/folder", "NEAR AI"):
                self.assertIn(token, localized)
            self.assertNotEqual(localized, text)
            self.assertEqual(tool.pseudo_text("", rtl), "")
        with self.assertRaises(SystemExit):
            tool.main(["--pseudo", str(tool.RESOURCES / "pseudo")])

    def test_pseudo_bundle_is_self_contained_and_permission_prompt_is_generated(self):
        value = tool.import_messages(fixture(), {"messages": {}})
        permission = value["messages"]["app.permission.apple_events"]
        self.assertEqual(permission["review"], "sensitive")
        self.assertEqual(permission["english"], "Near.ai opens Private AI sessions in Terminal.")
        files = tool.pseudo_files(value, Path("pseudo"))
        metadata = json.loads(files[Path("pseudo/Catalog.json")])
        self.assertEqual(set(metadata["locales"]), {"en", "en-XA", "ar-XB"})
        self.assertTrue(all(info["enabled"] for info in metadata["locales"].values()))
        for locale in ("en", "en-XA", "ar-XB"):
            self.assertIn(Path(f"pseudo/{locale}.lproj/Localizable.strings"), files)
            self.assertIn(Path(f"pseudo/{locale}.lproj/Localizable.stringsdict"), files)
            info = files[Path(f"pseudo/{locale}.lproj/InfoPlist.strings")].decode()
            expected = permission["english"] if locale == "en" else tool.pseudo_text(permission["english"], locale == "ar-XB")
            self.assertIn(f'"NSAppleEventsUsageDescription" = {tool.strings_quote(expected)};', info)
        self.assertEqual(metadata["messages"]["app.permission.apple_events"]["english"], permission["english"])
        self.assertEqual(set(value["locales"]), {"en", "fr"})
        production = tool.production_files(value, Path("resources"), Path("generated"))
        self.assertIn(Path("resources/en.lproj/InfoPlist.strings"), production)
        self.assertNotIn(Path("resources/fr.lproj/InfoPlist.strings"), production)

    def test_user_content_boundary_template_only(self):
        # This models the runtime contract, not a runtime implementation: only
        # known source equality permits lookup; user values enter afterwards.
        value = fixture("Hello {name}")
        original = copy.deepcopy(value)
        files = tool.pseudo_files(value, Path("pseudo"))
        self.assertEqual(value, original)
        template = tool.pseudo_text("Hello {name}")
        user = "private {place} /secret %lld"
        result = template.replace("{name}", user)
        self.assertIn(user, result)
        self.assertNotIn(user.encode(), b"".join(files.values()))
        for unknown in ("private text", "Hello Alice", "Hello {name} "):
            self.assertNotEqual(unknown, value["messages"]["app.test.label"]["english"])

    def test_ctypes_owned_pointer_freed_even_on_invalid_json(self):
        class Function:
            def __init__(self, result=None):
                self.result = result
                self.calls = []

            def __call__(self, *args):
                self.calls.append(args)
                return self.result

        class Library:
            pass

        lib = Library()
        lib.tc_string_free = Function()
        buffers = []
        for symbol in tool.REGISTRY.values():
            buffer = ctypes.create_string_buffer(b'{"title":"Fixed prose"}')
            buffers.append(buffer)
            setattr(lib, symbol, Function(ctypes.addressof(buffer)))
        templates = ctypes.create_string_buffer(b'[{"key":"message.test.value","english":"Canonical {name}","parameters":{"name":"probe"}}]')
        lib.tc_message_templates_json = Function(ctypes.addressof(templates))
        with patch.object(tool.ctypes, "CDLL", return_value=lib):
            snapshot = tool.snapshot("unused.so")
        self.assertEqual(len(snapshot["messages"]), len(tool.REGISTRY) + 1)
        self.assertEqual(snapshot["messages"]["message.test.value"], "Canonical {name}")
        self.assertEqual(len(lib.tc_string_free.calls), len(tool.REGISTRY) + 1)
        first = getattr(lib, next(iter(tool.REGISTRY.values())))
        self.assertIs(first.restype, ctypes.c_void_p)
        self.assertEqual(first.argtypes, [])
        invalid = ctypes.create_string_buffer(b"invalid json")
        first.result = ctypes.addressof(invalid)
        with patch.object(tool.ctypes, "CDLL", return_value=lib), self.assertRaises(ValueError):
            tool.snapshot("unused.so")
        self.assertEqual(lib.tc_string_free.calls[-1], (ctypes.addressof(invalid),))

    def test_check_is_byte_exact_and_rejects_old_disabled_bundles(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            outputs = tool.production_files(fixture(), root / "resources", root / "generated")
            tool.write_or_check(outputs, resource_root=root / "resources")
            tool.write_or_check(outputs, check=True, resource_root=root / "resources")
            path = root / "resources/fr.lproj/Localizable.strings"
            path.parent.mkdir()
            path.write_text('"key" = "unreviewed";')
            with self.assertRaisesRegex(ValueError, "Generated outputs differ"):
                tool.write_or_check(outputs, check=True, resource_root=root / "resources")
            tool.write_or_check(outputs, resource_root=root / "resources")
            self.assertFalse(path.exists())
            english = root / "resources/en.lproj/Localizable.strings"
            english.write_bytes(english.read_bytes() + b"\n")
            with self.assertRaisesRegex(ValueError, "Generated outputs differ"):
                tool.write_or_check(outputs, check=True, resource_root=root / "resources")

    def test_strings_escape_without_changing_literal_percent(self):
        text = '50% "quoted" \\ path\nnext\tline\r'
        self.assertEqual(tool.strings_quote(text), '"50% \\"quoted\\" \\\\ path\\nnext\\tline\\r"')


if __name__ == "__main__":
    unittest.main()

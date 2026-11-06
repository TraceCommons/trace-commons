#!/usr/bin/env python3
"""Reproduce the source entry-point inventory using only Python's standard library.

The source is read from the pinned Git tree, never the primary working directory.
This is a deliberately bounded lexical scanner, not a Rust/Swift compiler. Unknown
router composition or dispatch syntax is an error. See coverage.md for limits.
Existing audited coverage annotations are retained by ID and source fingerprint;
new entries start as gaps. An owner-model assignment is not evidence of coverage.
"""

import argparse
import collections
import functools
import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
from pathlib import Path

BASELINE = "8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6"
HERE = Path(__file__).resolve().parent.parent
ROOT = HERE.parents[2]
INGEST = "crates/trace-commons-ingest/src/"
MAIN = INGEST + "bin/trace-commons-ingest.rs"
INTERNAL = INGEST + "bin/trace_commons_ingest_internal/"
DAEMON = "crates/trace-commons-contributor/src/daemon/"
CONTRIBUTOR = "crates/trace-commons-contributor/src/"
SERVER = "crates/trace-commons-server/src/"
IPC = DAEMON + "ipc.rs"
MAC = "macos/Sources/TraceCommonsApp/"
GTK = "crates/trace-commons-contributor-gtk/src/"
WIN = "windows/src/TraceCommons.App/"
TAURI = "tauri-desktop/frontend/src/app/"
OWNERS = {
    "client_shell", "client_contribution", "client_services", "client_learning", "client_consent", "client_providers", "platform_clients", "server_receipt",
    "server_pipeline", "server_settlement", "server_lifecycle",
    "server_interfaces", "server_identity", "end_to_end",
}
METHODS = {"get", "post", "put", "patch", "delete", "head", "options", "trace"}
ANNOTATIONS = ("owner_model", "classification", "rationale", "model_actions", "modeled_observables", "model_family")
RAW_STRING = re.compile(r'(?:br|r)(#{0,32})"')
CHARACTER = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z_0-9]*")
PATH_CONSTANT = re.compile(r'\bconst\s+(\w*PATH)\s*:\s*&(?:\x27static\s+)?str\s*=\s*("(?:\\.|[^"\\\n])*")')


class Unsupported(ValueError):
    pass


def git(*args):
    return subprocess.check_output(["git", "-C", str(ROOT), *args])


def read_tree(commit):
    roots = [INGEST, CONTRIBUTOR, SERVER, "crates/trace-commons-protocol/src/", MAC, "macos/Sources/TCShellCore/FirstRun/",
             WIN, GTK, "tauri-desktop/frontend/src/", "tauri-desktop/src-tauri/src/"]
    archive = git("archive", commit, *roots)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
        return {member.name: tar.extractfile(member).read().decode("utf-8")
                for member in tar if member.isfile() and member.name.endswith((".rs", ".swift", ".cs", ".xaml", ".ts", ".tsx"))}


def tokens(text):
    """Return (spelling, byte-like text offset) pairs, excluding comments.

    Handles nested Rust block comments, escaped strings, raw Rust strings and
    character literals. Positions are Python character offsets; line counting
    remains exact for UTF-8 source. Identifiers and punctuation are separate.
    """
    out = []
    i = 0
    while i < len(text):
        if text[i].isspace():
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end
        elif text.startswith("/*", i):
            depth = 1
            i += 2
            while depth and i < len(text):
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise Unsupported("unterminated block comment")
        else:
            raw = RAW_STRING.match(text, i)
            if raw:
                end_marker = '"' + raw[1]
                end = text.find(end_marker, raw.end())
                if end < 0:
                    raise Unsupported("unterminated raw string")
                end += len(end_marker)
            elif text[i] == '"':
                end = i + 1
                while end < len(text):
                    if text[end] == "\\":
                        end += 2
                    elif text[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
                else:
                    raise Unsupported("unterminated string")
            elif text[i] == "'" and (char := CHARACTER.match(text, i)):
                end = char.end()
            elif ident := IDENTIFIER.match(text, i):
                end = ident.end()
            else:
                end = i + 1
            out.append((text[i:end], i))
            i = end
    return out


def close(ts, start):
    matching = {"(": ")", "[": "]", "{": "}"}
    stack = []
    for index in range(start, len(ts)):
        token = ts[index][0]
        if token in matching:
            stack.append(matching[token])
        elif token in matching.values():
            if not stack or stack.pop() != token:
                raise Unsupported(f"unbalanced delimiter near offset {ts[index][1]}")
            if not stack:
                return index
    raise Unsupported("unclosed delimiter")


def arguments(ts, start):
    end = close(ts, start)
    args = []
    mark = i = start + 1
    while i < end:
        if ts[i][0] in ("(", "[", "{"):
            i = close(ts, i) + 1
        elif ts[i][0] == ",":
            args.append(ts[mark:i])
            mark = i = i + 1
        else:
            i += 1
    if mark < end:
        args.append(ts[mark:end])
    return args, end


def test_only_cfg(predicate):
    """Is this cfg false for every non-test build? Other atoms are unknown.

    In particular, all(test, not(windows)) is test-only, but any(test,
    feature="production") is not. Do not silently remove a feature route.
    """
    def expression(index):
        name = predicate[index][0]
        if index + 1 < len(predicate) and predicate[index+1][0] == "(":
            args, end = arguments(predicate, index+1)
            values = [test_only_values(arg) for arg in args]
            if name == "not" and len(values) == 1:
                return {not value for value in values[0]}, end + 1
            if name in {"all", "any"}:
                outcomes = {name == "all"}
                for choices in values:
                    outcomes = {(left and right) if name == "all" else (left or right)
                                for left in outcomes for right in choices}
                return outcomes, end + 1
            raise Unsupported("unsupported cfg predicate: " + name)
        if index + 1 < len(predicate) and predicate[index+1][0] == "=":
            return {True, False}, index + 3
        return ({False} if name == "test" else {True, False}), index + 1

    def test_only_values(part):
        return test_only_cfg(part)[1]

    values, end = expression(0)
    if end != len(predicate):
        raise Unsupported("unparsed cfg predicate")
    return values == {False}, values


def production(text):
    """Remove inline cfg(test) modules, preserving offsets and source lines."""
    ts = tokens(text)
    remove = []
    for i in range(len(ts) - 8):
        if [t[0] for t in ts[i:i+4]] == ["#", "[", "cfg", "("]:
            attr_end = close(ts, i+1)
            predicate = ts[i+4:attr_end-1]
            if not test_only_cfg(predicate)[0]:
                continue
            j = attr_end + 1
            if ts[j][0] == "pub":
                j += 1
                if ts[j][0] == "(":
                    j = close(ts, j) + 1
            if ts[j][0] != "mod":
                continue
            j += 1
            while ts[j][0] != "{":
                if ts[j][0] == ";":
                    break
                j += 1
            end = j if ts[j][0] == ";" else close(ts, j)
            remove.append((ts[i][1], ts[end][1] + 1))
    for start, end in reversed(remove):
        text = text[:start] + re.sub(r"[^\n]", " ", text[start:end]) + text[end:]
    return text


def is_production_path(path):
    return (path.endswith(".rs") and "/tests/" not in path
            and not re.search(r"(?:^|/)(?:tests|[^/]*_tests|[^/]*_test)\.rs$", path))


def owner(identifier, kind):
    name = identifier.lower()
    if kind in ("ipc", "screen", "navigation", "client_worker"):
        if any(term in name for term in ("skill", "missiondraft", "mission_draft", "mission-draft")):
            return "client_learning"
        if any(term in name for term in ("managed", "insight", "inference", "privateai", "private_ai", "skill", "mission", "harness", "credential", "funding", "payout", "referral", "profile", "organization", "balance", "compute")):
            return "client_services"
        if any(term in name for term in ("queue", "traces", "preview", "approve", "history", "project", "consent", "source", "watch", "keep", "dismiss", "receipt", "upload", "policy", "enroll", "arming")):
            return "client_contribution"
        return "client_shell"
    if any(term in name for term in ("withdraw", "revok", "retention", "export", "purge", "tombstone", "invalidat", "rebuild")):
        return "server_lifecycle"
    if any(term in name for term in ("settle", "credit", "payout", "outbox")):
        return "server_settlement"
    if any(term in name for term in ("pipeline", "operator-hold")):
        return "server_pipeline"
    if any(term in name for term in ("submit", "enroll", "upload-claim", "admission", "ingest")):
        return "server_receipt"
    return "server_interfaces"


def row(kind, identifier, path, offset, source, **extra):
    line = source.count("\n", 0, offset) + 1
    return {
        "id": f"{kind}:{identifier}:{path}:{line}", "kind": kind,
        "identifier": identifier, "source": {"path": path, "line": line},
        "inventory_role": {
            "route": "registered HTTP method", "ipc": "dispatched IPC method",
            "navigation": "declared destination or UI entry",
            "screen": "presentation declaration candidate; independent reachability unresolved",
            "boundary": "audited manual HTTP dispatch or external listener boundary",
            "native_entry": "audited native startup, window or event entry point",
            "client_worker": "worker declaration or named periodic pass",
            "server_worker": "worker declaration candidate; runtime enablement unresolved",
        }[kind],
        "source_signature": source_fingerprint(source),
        "owner_model": owner(identifier, kind), "classification": "gap",
        "rationale": "Entry point discovered; no concrete model action and observable-state correspondence has been audited. Owner is a triage assignment, not a coverage claim.",
        "observable_state": extra.pop("observable_state", "Handler result or visible destination; full implementation state is not inferred by this scanner."),
        "model_actions": [], "modeled_observables": [], **extra,
    }


@functools.lru_cache(maxsize=None)
def source_fingerprint(source):
    return hashlib.sha256(source.encode()).hexdigest()


def string_arg(arg, constants=None):
    identifier = "".join(token for token, _ in arg)
    if constants and identifier in constants:
        return constants[identifier]["value"]
    if len(arg) != 1 or not arg[0][0].startswith('"'):
        raise Unsupported("route path must be an ordinary string literal")
    return json.loads(arg[0][0])


def handler_arg(arg):
    text = "".join(t[0] for t in arg)
    if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)*", text):
        raise Unsupported(f"handler must be a direct function path: {text[:100]}")
    return text


def routes(path, source, constants=None):
    text = production(source)
    constants = dict(constants or {})
    for match in re.finditer(r"use\s+(trace_commons_protocol::[A-Za-z_0-9:]+)\s+as\s+(\w+)\s*;", text):
        for name, value in list(constants.items()):
            if name.startswith(match[1] + "::"):
                constants[match[2] + name[len(match[1]):]] = value
    for match in PATH_CONSTANT.finditer(text):
        constants[match[1]] = {"value": json.loads(match[2]), "source": {"path": path, "line": source.count("\n", 0, match.start())+1}}
    ts = tokens(text)
    result = []
    skipped_helper = 0
    for i in range(1, len(ts) - 2):
        if ts[i-1][0] != "." or ts[i+1][0] != "(":
            continue
        method = ts[i][0]
        if method in {"nest", "nest_service", "route_service", "fallback_service", "route_layer_service"}:
            raise Unsupported(f"unsupported router construct {path}:{source.count(chr(10), 0, ts[i][1])+1}: {method}")
        if method != "route":
            continue
        args, _ = arguments(ts, i+1)
        if path == INTERNAL + "account_routes.rs" and [t[0] for t in args[0]] == ["path"]:
            skipped_helper += 1
            continue
        if len(args) != 2:
            raise Unsupported(f"route expects two arguments: {path}:{ts[i][1]}")
        try:
            route_path = string_arg(args[0], constants)
        except Unsupported as error:
            raise Unsupported(f"{path}:{source.count(chr(10), 0, ts[i][1])+1}: {error}") from error
        method_expression = args[1]
        found = []
        for j in range(len(method_expression)-1):
            token = method_expression[j][0]
            if token in {"on", "any", "on_service", "any_service"} and method_expression[j+1][0] == "(":
                raise Unsupported(f"unsupported method router {token} for {route_path}")
            if token in METHODS and method_expression[j+1][0] == "(":
                call_args, _ = arguments(method_expression, j+1)
                if len(call_args) != 1:
                    raise Unsupported(f"method router arity for {route_path}")
                found.append((token.upper(), handler_arg(call_args[0]), method_expression[j][1]))
        if not found:
            raise Unsupported(f"unrecognized method router for {route_path}")
        for verb, handler, _ in found:
            result.append(row("route", f"{verb} {route_path}", path, ts[i][1], source,
                              http_method=verb, path_template=route_path, handler=handler,
                              registration="axum.Router.route", authority="handler-specific; not inferred from path",
                              path_constant=(constants or {}).get("".join(token for token, _ in args[0]))))

    # Follow each AccountRoutes::new() chain, including delegated builder files.
    for i in range(len(ts)-6):
        if [t[0] for t in ts[i:i+6]] != ["AccountRoutes", ":", ":", "new", "(", ")"]:
            continue
        j = i + 6
        while j + 2 < len(ts) and ts[j][0] == "." and ts[j+2][0] == "(":
            method = ts[j+1][0]
            args, end = arguments(ts, j+2)
            if method in METHODS or method == "post_with_body_limit":
                expected = 3 if method == "post_with_body_limit" else 2
                if len(args) != expected:
                    raise Unsupported(f"AccountRoutes arity: {path}:{ts[j+1][1]}")
                try:
                    route_path = string_arg(args[0], constants)
                except Unsupported as error:
                    raise Unsupported(f"{path}:{source.count(chr(10), 0, ts[j+1][1])+1}: {error}") from error
                verb = "POST" if method == "post_with_body_limit" else method.upper()
                result.append(row("route", f"{verb} {route_path}", path, ts[j+1][1], source,
                                  http_method=verb, path_template=route_path,
                                  handler=handler_arg(args[1]), registration="AccountRoutes." + method,
                                  authority="account_auth_middleware; additional handler authority applies",
                                  path_constant=constants.get("".join(token for token, _ in args[0]))))
            elif method not in {"merge", "route_layer_fn"}:
                raise Unsupported(f"unknown AccountRoutes builder method {method} in {path}")
            j = end + 1
    return result, skipped_helper


def dispatch(source):
    ts = tokens(production(source))
    entries = {}
    counts = {}
    for dispatcher in ("handle_request", "handle_request_async"):
        start = next(i for i in range(len(ts)-2) if ts[i][0] == "fn" and ts[i+1][0] == dispatcher)
        start = next(i for i in range(start, len(ts)) if [t[0] for t in ts[i:i+9]] == ["match", "req", ".", "method", ".", "as_str", "(", ")", "{"])
        end = close(ts, start+8)
        j = start + 9
        count = 0
        while j < end:
            if ts[j][0].startswith('"'):
                if [t[0] for t in ts[j+1:j+3]] != ["=", ">"]:
                    raise Unsupported(f"IPC arm is not a single literal: {ts[j][0]}")
                method = json.loads(ts[j][0])
                if not re.fullmatch(r"[a-z_0-9]+", method):
                    raise Unsupported(f"IPC method literal: {method}")
                entry = entries.setdefault(method, row("ipc", method, IPC, ts[j][1], source, dispatchers=[]))
                entry["dispatchers"].append({"name": dispatcher, "line": source.count("\n", 0, ts[j][1])+1})
                count += 1
                j += 3
            elif ts[j][0] == "_":
                j += 1
            elif ts[j][0] in ("(", "[", "{"):
                j = close(ts, j) + 1
            else:
                j += 1
        counts[dispatcher] = count
    # Compare advertised methods and real dispatch, independently of comments.
    start = next(i for i in range(len(ts)-2) if ts[i][0] == "const" and ts[i+1][0] == "METHODS")
    start = next(i for i in range(start, len(ts)) if ts[i][0] == "=")
    start = next(i for i in range(start, len(ts)) if ts[i][0] == "[")
    end = close(ts, start)
    advertised = [json.loads(token) for token, _ in ts[start+1:end] if token.startswith('"')]
    if set(advertised) != set(entries) or len(advertised) != len(set(advertised)):
        raise Unsupported("advertised IPC methods differ from dispatch match arms")
    managed = [entry for name, entry in entries.items() if name.startswith("managed_") and name not in {
        "managed_cli_options_set", "managed_folder_settings_get", "managed_folder_settings_reset"}]
    guard = source.find('if req.method.starts_with("managed_")')
    if guard < 0:
        raise Unsupported("managed async pre-dispatch guard moved; audit delegation")
    for entry in managed:
        entry["delegation"] = {"handler": "super::managed::handle_async", "source": {"path": IPC, "line": source.count("\n", 0, guard)+1}, "guard": "managed_ prefix, except CLI/folder settings, and membership in METHODS"}
    return list(entries.values()), counts


def enum_rows(path, source, enum_name, platform):
    ts = tokens(source)
    start = next(i for i in range(len(ts)-1) if ts[i][0] == "enum" and ts[i+1][0] == enum_name)
    start = next(i for i in range(start, len(ts)) if ts[i][0] == "{")
    end = close(ts, start)
    result = []
    j = start + 1
    while j < end:
        if ts[j][0] == "case":
            result.append(row("navigation", enum_name + "." + ts[j+1][0], path, ts[j+1][1], source, platform=platform))
        elif ts[j][0] in ("(", "[", "{"):
            j = close(ts, j)
        j += 1
    if not result:
        raise Unsupported(f"enum case scan returned nothing: {enum_name}")
    return result


def screens(tree):
    result = []
    for path, names in {
        MAC + "MonitorNavigation.swift": ["HomePage", "TracesPage", "MonitorDestination"],
        MAC + "Views/SettingsSections.swift": ["SettingsSection"],
        "macos/Sources/TCShellCore/FirstRun/FirstRunState.swift": ["FirstRunStep"],
    }.items():
        for name in names:
            result.extend(enum_rows(path, tree[path], name, "macOS native"))
    path = MAC + "TraceCommonsAppMain.swift"
    source = tree[path]
    for match in re.finditer(r'\bWindow\([^\n]+id:\s*WindowID\.(\w+)\)', source):
        result.append(row("navigation", "WindowID." + match[1], path, match.start(), source, platform="macOS native"))
    match = re.search(r"\bMenuBarExtra\s*\{", source)
    if not match:
        raise Unsupported("macOS menu-bar scene moved")
    result.append(row("navigation", "MenuBarExtra", path, match.start(), source, platform="macOS native"))
    # SwiftUI screen-family declarations, including inspectors/dialogs. This
    # intentionally over-includes view components rather than hiding candidates.
    for path, source in sorted(tree.items()):
        if path.startswith(MAC + "Views/") and path.endswith(".swift"):
            for match in re.finditer(r"\bstruct\s+(\w+)\s*(?:<[^\n]+?>)?\s*:\s*(?:[^\n{]*,\s*)?View\b", production(source)):
                result.append(row("screen", match[1], path, match.start(), source,
                                  platform="macOS native", discovery="SwiftUI View declaration candidate; may be a component rather than a separately reachable screen"))
        if path.startswith(WIN) and path.endswith((".cs", ".xaml.cs")):
            for match in re.finditer(r"\bclass\s+(\w+)\s*:\s*(Window|UserControl|ContentDialog)\b", source):
                result.append(row("screen", match[1], path, match.start(), source, platform="Windows native", discovery=match[2] + " declaration"))
    path = WIN + "MainWindow.xaml"
    for match in re.finditer(r'Click="(OnShow\w+)"', tree[path]):
        result.append(row("navigation", match[1], path, match.start(), tree[path], platform="Windows native"))
    path = WIN + "ViewModels/OnboardingViewModel.cs"
    match = re.search(r"enum OnboardingStep\s*\{([^}]+)\}", tree[path])
    for case in re.finditer(r"\b(\w+)\s*,", match[1]):
        result.append(row("navigation", "OnboardingStep." + case[1], path, match.start(1)+case.start(), tree[path], platform="Windows native"))
    path = GTK + "ui/mod.rs"
    source = tree[path]
    for name, pattern in [("BASE_SCREENS", r"const BASE_SCREENS[^=]*=\s*\["), ("MISSION_DRAFTS_SCREEN", r"const MISSION_DRAFTS_SCREEN[^=]*=\s*")]:
        match = re.search(pattern, source)
        if not match:
            raise Unsupported("GTK screen table moved: " + name)
        end = source.index(";", match.end())
        for case in re.finditer(r'\(\s*("[^"]+"|PRIVATE_INFERENCE_SCREEN)\s*,', source[match.end():end]):
            name_value = case[1].strip('"')
            result.append(row("navigation", name_value, path, match.end()+case.start(), source,
                              platform="Linux GTK", conditional="missions_enabled at process startup" if name == "MISSION_DRAFTS_SCREEN" else None))
    path = GTK + "ui/onboarding.rs"
    source = tree[path]
    match = re.search(r"enum Step\s*\{([^}]+)\}", source)
    if not match:
        raise Unsupported("GTK onboarding Step enum moved")
    for case in re.finditer(r"\b(\w+)\s*,", match[1]):
        result.append(row("navigation", "Step." + case[1], path, match.start(1)+case.start(), source, platform="Linux GTK"))
    # GTK screen-family constructors include the dialog/inspector modules.
    for path, source in sorted(tree.items()):
        if path.startswith(GTK + "ui/") and path.endswith(".rs"):
            for match in re.finditer(r"pub(?:\([^)]*\))?\s+struct\s+(\w+(?:View|Dialog|Window))\b", production(source)):
                result.append(row("screen", match[1], path, match.start(), source, platform="Linux GTK", discovery="public UI family declaration candidate"))
    path = TAURI + "routes.ts"
    source = tree[path]
    for table in ("routePaths", "flowPaths"):
        match = re.search(r"const " + table + r"\s*=\s*\{([^}]+)\}", source)
        if not match:
            raise Unsupported("Tauri navigation table moved: " + table)
        for case in re.finditer(r'([\w-]+|"[\w-]+")\s*:\s*"([^"]+)"', match[1]):
            result.append(row("navigation", case[2], path, match.start(1)+case.start(), source,
                              platform="Tauri", discovery=table))
    path = TAURI + "app-routes.tsx"
    source = tree[path]
    for match in re.finditer(r"<(\w+Page)\b", source):
        result.append(row("screen", match[1], path, match.start(), source, platform="Tauri", discovery="rendered route page; repeated render sites retained"))
    path = "tauri-desktop/frontend/src/features/onboarding/flow1.ts"
    source = tree[path]
    match = re.search(r"type OnboardingStep\s*=([^;]+);", source)
    if not match:
        raise Unsupported("Tauri onboarding step union moved")
    for case in re.finditer(r'"([\w_]+)"', match[1]):
        result.append(row("navigation", "OnboardingStep." + case[1], path, match.start(1)+case.start(), source, platform="Tauri"))
    return result


def workers(tree):
    result = []
    for path, source in sorted(tree.items()):
        if not is_production_path(path) or not path.startswith((INGEST, CONTRIBUTOR, SERVER)):
            continue
        text = production(source)
        # Spawner declarations identify configured background worker families.
        # Individual HTTP worker entry points are separately tagged on routes.
        for match in re.finditer(r"\bfn\s+(spawn_\w+)\s*(?:<[^\n]*>)?\s*\(", text):
            kind = "client_worker" if path.startswith(CONTRIBUTOR) else "server_worker"
            result.append(row(kind, match[1], path, match.start(), source,
                              discovery="background spawner declaration; runtime enablement and call graph require separate audit"))
    # Named periodic pass entry points do not follow the spawn_* convention.
    for path, names in {
        DAEMON + "mod.rs": ["run_supervisor", "supervise_passes", "drain_approved", "refresh_estimate_table", "refresh_history", "refresh_community"],
        DAEMON + "watcher.rs": ["tick", "tick_paths"],
        DAEMON + "uploader.rs": ["upload_entry"],
        DAEMON + "managed.rs": ["lease_pass"],
        DAEMON + "token_cleanup.rs": ["pass"],
    }.items():
        source = tree[path]
        text = production(source)
        for name in names:
            matches = list(re.finditer(r"\bfn\s+" + name + r"\s*(?:<[^\n]*>)?\s*\(", text))
            if len(matches) != 1:
                raise Unsupported(f"periodic worker entry moved: {path}: {name}")
            identifier = Path(path).stem + "::" + name
            result.append(row("client_worker", identifier, path, matches[0].start(), source,
                              discovery="audited periodic daemon pass entry point; scheduling/restart occurs in the supervisor"))
    return result


def manual_boundaries(tree):
    """Audited non-Axum dispatch sites: fail if the exact selector disappears.

    These are manually enumerated entry points, not an automatic inference that
    all HTTP listeners use Axum. Each selector is executable source, not a comment.
    """
    result = []
    declarations = [
        (CONTRIBUTOR + "account_auth.rs", "if path != NATIVE_REDIRECT_PATH", "native account authorization callback", "unrestricted method; target path checked after response", "client_shell", "One request on an ephemeral IPv4 loopback listener; the callback parser does not check the HTTP method. A response is sent before path validation. Code exchange remains a separate operation."),
        (DAEMON + "account_onboarding.rs", "if url.path() != LOOPBACK_PATH", "GET /trace-commons/near-onboarding/callback", "GET", "client_shell", "Wallet account-onboarding result callback; result/state and account verification are separate checks."),
        (DAEMON + "nearai_credential/loopback.rs", 'if request.method == "GET" && request.path == CALLBACK_PATH', "GET /trace-commons/near-ai/callback", "GET", "client_services", "Cloud login bounce page; it does not itself persist a credential."),
        (DAEMON + "nearai_credential/loopback.rs", 'if request.method != "POST" || request.path != DEPOSIT_PATH', "POST /trace-commons/near-ai/deposit", "POST", "client_services", "Fragment deposit and state binding; callback success waits for the supplied setup operation."),
        (DAEMON + "nearai_credential/chooser.rs", '"GET" if head.target == format!("{PAGE_PATH}?state={state}")', "GET /trace-commons/near-ai/sign-in?state={state}", "GET", "client_services", "Provider-selection browser page; exact state and empty request body are checked."),
        (DAEMON + "nearai_credential/chooser.rs", '"POST" if head.target == CHOOSE_PATH', "POST /trace-commons/near-ai/choose", "POST", "client_services", "Provider choice dispatch; body/origin/ceremony checks remain implementation detail."),
        (DAEMON + "nearai_credential/near_wallet_loopback.rs", '("GET", target) if target == CALLBACK_PATH', "GET /near-ai/near-wallet/callback", "GET", "client_services", "Wallet page on the listener's authority; challenge ownership and timeout remain separate."),
        (DAEMON + "nearai_credential/near_wallet_loopback.rs", '("POST", target) if target == DEPOSIT_PATH', "POST /near-ai/near-wallet/result", "POST", "client_services", "Wallet result deposit checks origin, bounded body, signed proof and challenge consumption."),
        (DAEMON + "nearai_credential/near_wallet_loopback.rs", '("POST", target) if target == super::chooser::CHOOSE_PATH', "POST chooser choice on wallet listener", "POST", "client_services", "A stale choice from the shared chooser ends or redirects the wallet attempt rather than acting as a fresh result."),
        (DAEMON + "nearai_credential/near_wallet_loopback.rs", '("GET", target) if target.starts_with(super::chooser::PAGE_PATH)', "GET chooser page on wallet listener", "GET", "client_services", "A stale chooser page is handled within the active wallet listener."),
        (DAEMON + "nearai_credential/near_wallet_loopback.rs", '("GET", target) => Route::Asset(', "GET wallet asset allowlist", "GET", "client_services", "Asset paths are constrained by near_wallet_page::asset_path; asset body and executable bundle integrity are not modeled."),
        (DAEMON + "private_inference.rs", 'embed::start_with_options(&home, port, options, |_, _| {}).await', "IronWire embedded HTTP listener", "external router", "client_services", "The daemon owns the embedded listener lifecycle and supplies options/credentials. Individual IronWire HTTP routes live in an external dependency and are outside this repository's source inventory."),
    ]
    for path, selector, identifier, method, model, scope in declarations:
        source = tree[path]
        text = production(source)
        if text.count(selector) != 1:
            raise Unsupported(f"manual HTTP selector changed or is ambiguous: {path}: {selector}")
        entry = row("boundary", identifier, path, text.index(selector), source,
                    registration="manual HTTP dispatch or embedded external listener",
                    http_method=method, observable_state=scope)
        entry["owner_model"] = model
        result.append(entry)
    return result


def native_entries(tree):
    result = []
    declarations = [
        (GTK + "main.rs", "fn local_start(", "GTK local Insights launch selection", "Linux GTK"),
        (GTK + "main.rs", "fn connect_local_startup(", "GTK local Insights activation", "Linux GTK"),
        (GTK + "main.rs", "fn connect_startup(", "GTK contributor activation", "Linux GTK"),
        (GTK + "main.rs", "fn start_or_ask(", "GTK lazy daemon startup or roots prompt", "Linux GTK"),
        (GTK + "ui/mod.rs", "fn wire_quit(", "GTK window-close role-specific quit", "Linux GTK"),
        (WIN + "DaemonHost.cs", "public async Task StartAsync(", "Windows daemon startup", "Windows native"),
        (WIN + "MainWindow.xaml.cs", "private async void OnAppWindowClosing(", "Windows window-close tray decision", "Windows native"),
        (WIN + "MainWindow.xaml.cs", "private async void OnClosed(", "Windows final window disposal", "Windows native"),
        ("tauri-desktop/src-tauri/src/runtime.rs", "fn start_runtime(", "Tauri initial runtime startup", "Tauri"),
        ("tauri-desktop/src-tauri/src/runtime.rs", "fn ensure_daemon_started(", "Tauri ensure daemon or attach", "Tauri"),
        ("tauri-desktop/src-tauri/src/app.rs", "fn supervise_attached_events<", "Tauri attached event supervision", "Tauri"),
        ("tauri-desktop/src-tauri/src/app.rs", "window.on_window_event(move |event|", "Tauri native close interception", "Tauri"),
        ("tauri-desktop/src-tauri/src/commands/platform.rs", "fn quit_app<", "Tauri explicit application quit", "Tauri"),
        (MAC + "AppDelegate.swift", "func applicationShouldTerminate(", "macOS application termination", "macOS native"),
        (MAC + "DaemonStartup.swift", "func start(\n", "macOS pending daemon startup adoption", "macOS native"),
    ]
    for path, selector, identifier, platform in declarations:
        source = tree[path]
        if source.count(selector) != 1:
            raise Unsupported(f"native entry selector changed or is ambiguous: {path}: {selector}")
        entry = row("native_entry", identifier, path, source.index(selector), source, platform=platform,
                    discovery="manual native/event entry-point inventory; exact executable selector checked")
        entry["owner_model"] = "client_shell" if platform == "macOS native" else "platform_clients"
        result.append(entry)
    return result


def reconcile(rows, previous):
    old_rows = {entry["id"]: entry for entry in previous.get("entries", [])}
    for entry in rows:
        old = old_rows.get(entry["id"])
        if old and old.get("source_signature") == entry["source_signature"]:
            for key in ANNOTATIONS:
                if key in old:
                    entry[key] = old[key]
        if entry["owner_model"] not in OWNERS:
            raise Unsupported("unknown model owner: " + entry["id"])
        if entry["classification"] not in {"gap", "coarse-interface", "detailed"}:
            raise Unsupported("unknown classification: " + entry["id"])
        if entry["classification"] != "gap":
            if not entry["model_actions"] or not entry["modeled_observables"]:
                raise Unsupported("covered rows need concrete actions and observables: " + entry["id"])
            model = HERE / "models" / (entry["owner_model"] + ".qnt")
            if not model.exists():
                raise Unsupported("coverage owner model missing: " + str(model))
            source = model.read_text()
            for action in entry["model_actions"]:
                if not re.search(r"\baction\s+" + re.escape(action) + r"\b", source):
                    raise Unsupported("coverage action missing: " + action + " in " + str(model))
            for observable in entry["modeled_observables"]:
                for field in re.findall(r"\bs\.(\w+)", observable):
                    if not re.search(r"\b" + re.escape(field) + r"\s*:", source):
                        raise Unsupported("coverage observable missing: " + field + " in " + str(model))


def document(manifest):
    rows = manifest["entries"]
    out = ["# Current-behavior coverage inventory", "",
           "This is a source-discovery index, not a claim that the entire application is modeled. "
           "Owner assignment only identifies the model that should reconcile an entry. "
           "A row stays a visible gap until its concrete model actions and observable fields are audited. "
           "`coarse-interface` identifies a deliberately abstract interaction; it does not cover handler internals.", "",
           f"Source tree: `{manifest['source_commit']}`. Worktree at discovery: `{manifest['discovery_worktree_commit']}`. "
           "Only files in the pinned Git tree are scanned. The primary checkout and its untracked additions are excluded.", "",
           "Run from the repository root:", "", "```sh",
           "python3 docs/flows/trace-commons/scripts/inventory.py --self-test",
           "python3 docs/flows/trace-commons/scripts/inventory.py",
           "python3 docs/flows/trace-commons/scripts/inventory.py --check", "```", "",
           "The writer retains audited annotations in coverage.json only while the row ID and source-file fingerprint match. "
           "Covered rows require concrete model action names and modeled observables; the command checks that named actions and referenced state fields exist. "
           "Existence of an action does not establish semantic correspondence: human source/model review remains necessary.", "",
           "## Extracted entry points", "", "| Kind | Entries | Detailed | Coarse interface | Gaps |", "| --- | ---: | ---: | ---: | ---: |"]
    counts = collections.Counter(entry["kind"] for entry in rows)
    for kind, count in sorted(counts.items()):
        classes = collections.Counter(entry["classification"] for entry in rows if entry["kind"] == kind)
        out.append(f"| {kind} | {count} | {classes['detailed']} | {classes['coarse-interface']} | {classes['gap']} |")
    out += ["", "| Candidate owner | Detailed | Coarse interface | Gaps |", "| --- | ---: | ---: | ---: |"]
    for model in sorted(OWNERS):
        counts = collections.Counter(entry["classification"] for entry in rows if entry["owner_model"] == model)
        out.append(f"| {model} | {counts['detailed']} | {counts['coarse-interface']} | {counts['gap']} |")
    out += ["", "The complete per-entry list is [coverage.json](coverage.json), with exact source path/line, "
            "identifier, handler or dispatcher, owner, classification, rationale, actions and observables. "
            "HTTP methods are explicit registrations; Axum's implicit HEAD behavior on GET is not counted as a separate registration.", "",
            "## Meaning of the classifications", "",
            "Detailed rows map a specific source flow projection to named model actions and concrete state fields. "
            "Their rationale records omitted transport, rendering, parameter or unbounded-domain detail. "
            "A detailed row is not a claim that every branch of its containing file or endpoint has been modeled. "
            "Coarse interface rows preserve only the stated boundary and outcomes. Gaps are discovered entries with no reviewed correspondence.", "",
            "Gap totals do not count missing independent flows. Routes and IPC methods are actual registered/dispatch entry points; "
            "screen rows are intentionally over-inclusive presentation declarations, including decorative components, and require reachability review. "
            "Each row's inventory_role records that distinction. Navigation declarations and worker spawners can also share behavior with another row. "
            "An unresolved component declaration is different from an omitted account or queue mutation.", "",
            "The initial reviewed mappings include the six native macOS first-run screen families, core trace preview/approval/policy/history commands, "
            "selected credential/harness/managed-session/profile commands, phase-worker state and explicit auxiliary server interface families. "
            "The submit route is coarse at the HTTP entry: its versioned receipt branch is modeled in detail, while legacy processing and parsing are omitted. "
            "Cross-platform first-run and native widgets are not covered by those macOS mappings.", "",
            "Substantial remaining gaps include unassigned UI component/sheet declarations and platform-specific navigation, "
            "per-card Insights/drilldown behavior and complete learning concurrency, account/device and enrollment route branches, "
            "legacy review/gate scheduling, exact administrative reports/drills, all generic-task spawn sites and standalone operator CLI entry points. "
            "The manifest is deliberately visible about these gaps; the model suite cannot be described as complete application coverage.", "",
            "## Scanner boundaries", ""]
    out.extend("- " + limit for limit in manifest["scanner_limits"])
    out += ["", "## Source boundary and exclusions", "",
            "The model baseline predates unrelated work in the primary checkout. "
            "The following paths were untracked there when the inventory was prepared; their contents were not inspected or modeled. "
            "This list is a historical exclusion snapshot, not a live scan and not a shipping-feature inventory.", "", "```text"]
    out.extend(manifest["excluded_primary_untracked_paths"])
    out += ["```", "", "See [platforms.md](platforms.md) for platform-specific differences and their model limits.", ""]
    return "\n".join(out)


def self_test():
    sample = '''// .route("/ignored", get(fake))
    Router::new().route("/x", get(read).merge(post(write).layer(limit())))
      .route("/y", put(update).delete(remove));
    AccountRoutes::new().post_with_body_limit("/a", account::make, 1024)
      .merge(extra::routes()).patch("/b", account::rename);
    #[cfg(test)] mod tests { fn fixture() { Router::new().route("/test", get(test)); } }
    #[cfg(all(test, target_os = "linux"))] pub(crate) mod fixtures {
      fn fixture() { Router::new().route("/test-public", get(test)); }
    }
    #[cfg(all(test, not(windows)))] mod unix_fixtures {
      fn fixture() { Router::new().route("/test-unix", get(test)); }
    }
    '''
    found, _ = routes(MAIN, sample)
    assert [(r["http_method"], r["path_template"], r["handler"]) for r in found] == [
        ("GET", "/x", "read"), ("POST", "/x", "write"),
        ("PUT", "/y", "update"), ("DELETE", "/y", "remove"),
        ("POST", "/a", "account::make"), ("PATCH", "/b", "account::rename")]
    for bad in ['Router::new().route(PATH, get(handler))',
                'Router::new().route("/x", any(handler))',
                'Router::new().nest("/x", routes())',
                'AccountRoutes::new().extension("/x", handler)']:
        try:
            routes(MAIN, bad)
        except Unsupported:
            pass
        else:
            raise AssertionError("unsupported route was silently omitted: " + bad)
    assert [t[0] for t in tokens('/* /* nested */ */ r#"{ // }"# "\\\"" \'{\'')][0] == 'r#"{ // }"#'
    constants = {"trace_commons_protocol::example::READ_PATH": {
        "value": "/resolved", "source": {"path": "example.rs", "line": 1}}}
    found, _ = routes(MAIN, '''use trace_commons_protocol::example as wire;
      Router::new().route(wire::READ_PATH, get(read));''', constants)
    assert found[0]["path_template"] == "/resolved"
    assert found[0]["path_constant"]["source"]["path"] == "example.rs"
    found, _ = routes(MAIN, '''#[cfg(any(test, feature = "production"))] mod optional {
      fn route() { Router::new().route("/feature", get(read)); }
    }''')
    assert found[0]["path_template"] == "/feature"
    print("inventory extractor self-tests passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if generated inventory differs")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    manifest_path = HERE / "coverage.json"
    previous = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}
    tree = read_tree(BASELINE)
    rows = []
    helpers = 0
    route_files = []
    constants = {}
    protocol = "crates/trace-commons-protocol/src/"
    for path, source in sorted(tree.items()):
        if path.startswith(protocol) and path.endswith(".rs"):
            module = path.removeprefix(protocol).removesuffix(".rs").replace("/", "::")
            for match in PATH_CONSTANT.finditer(production(source)):
                constants[f"trace_commons_protocol::{module}::{match[1]}"] = {
                    "value": json.loads(match[2]), "source": {"path": path, "line": source.count("\n", 0, match.start())+1}}
    by_name = collections.defaultdict(list)
    for name, value in constants.items():
        by_name[name.split("::")[-1]].append(value)
    for name, values in by_name.items():
        if len({value["value"] for value in values}) == 1:
            constants[name] = values[0]
    for path, source in sorted(tree.items()):
        if path.startswith((INGEST, SERVER, CONTRIBUTOR)) and is_production_path(path):
            try:
                found, skipped = routes(path, source, constants)
            except Unsupported as error:
                raise Unsupported(path + ": " + str(error)) from error
            rows.extend(found)
            helpers += skipped
            if found:
                route_files.append(path)
    if helpers != 1:
        raise Unsupported(f"expected one audited AccountRoutes forwarding helper, found {helpers}")
    ipc_rows, dispatch_counts = dispatch(tree[IPC])
    rows.extend(ipc_rows)
    rows.extend(screens(tree))
    rows.extend(workers(tree))
    rows.extend(manual_boundaries(tree))
    rows.extend(native_entries(tree))
    rows.sort(key=lambda r: (r["kind"], r["source"]["path"], r["source"]["line"], r["identifier"]))
    ids = [entry["id"] for entry in rows]
    if len(ids) != len(set(ids)):
        raise Unsupported("duplicate inventory entry IDs")
    reconcile(rows, previous)
    manifest = {
        "schema_version": 1, "source_commit": BASELINE,
        "discovery_worktree_commit": previous.get("discovery_worktree_commit", git("rev-parse", "HEAD").decode().strip()),
        "scope": "Current source baseline; no product promises selected or checked.",
        "discovery": {"route_registration_files": route_files, "account_forwarding_helpers": helpers,
                      "ipc_dispatch_arm_counts": dispatch_counts,
                      "route_methods": "explicit registrations only; GET also has framework-supplied HEAD behavior",
                      "unsupported_constructs": "recognized unsupported routing/dispatch constructs fail the command; none encountered in this baseline"},
        "source_files": {path: {"sha256": source_fingerprint(tree[path]), "line_count": len(tree[path].splitlines())}
                         for path in sorted({r["source"]["path"] for r in rows}
                                            | {r["path_constant"]["source"]["path"] for r in rows if r.get("path_constant")})},
        "scanner_limits": [
            "Pinned Git-tree lexical extraction, not a compiler or reachability proof. Runtime feature flags, deployment configuration, middleware ordering, tenant scope and handler behavior require separate source review.",
            "HTTP scan includes production .rs files in ingest, server and contributor, excludes tests/ and *_test.rs / *_tests.rs, and removes inline cfg(test) modules (including visibility and positive all(test, ...) forms). Axum .route literal registrations, protocol/local path string constants and AccountRoutes::new() helper chains are supported, including delegated builder files and post_with_body_limit. Qualified protocol aliases and unique imported protocol constant names are resolved; the one AccountRoutes forwarding .route(path, method_router) helper is explicitly recognized.",
            "Unknown .nest, .nest_service, .route_service, .fallback_service, method-router any/on, nonliteral .route paths, indirect handler expressions and unknown AccountRoutes builder methods fail extraction. Ordinary Router.merge calls are composition sites: registrations in all scanned files are listed, but their runtime reachability is not inferred. A new routing API or builder type requires scanner review; a custom router with no recognized construct can remain invisible.",
            "IPC scans the top-level literal arms in handle_request and handle_request_async, checks equality with advertised METHODS, and records managed_ asynchronous pre-dispatch delegation. Handler internals, parameter variants, pushed-event payloads and nested dispatchers are not separately enumerated. Future nonliteral/or-pattern method dispatch requires scanner review.",
            "macOS navigation cases include HomePage, TracesPage, MonitorDestination, SettingsSection and six FirstRunStep cases. SwiftUI View declarations are over-inclusive candidates (many are components), not proof of screen reachability. Rendering closures without a named View and dynamic sheets require manual UI review.",
            "Windows inventory includes Window/UserControl/ContentDialog declarations, MainWindow OnShow rail actions and OnboardingStep. GTK includes public *View/*Dialog/*Window declarations and the named screen tables; Tauri includes routePaths/flowPaths and rendered *Page sites. Audited native_entry rows additionally name startup, local Insights entry, close/quit and attached-event entry points, with exact executable selectors checked. These structural scans do not enumerate every local modal, control, conditional branch or platform entry point.",
            "Manual HTTP boundary rows explicitly enumerate native-account, wallet-onboarding, Cloud credential and browser chooser callback dispatch, wallet asset dispatch and the external IronWire listener start. Their exact executable selectors are checked. A new hand-written HTTP parser/listener is not automatically discovered; external IronWire implementation and HTTP routing are outside this source tree.",
            "Worker rows enumerate production spawn_* function declarations in ingest, server and contributor, plus manually audited named daemon supervisor/watch/upload/history/community/estimate/lease/token-cleanup pass entry points; worker HTTP routes are also ordinary route rows. Generic tokio::spawn sites without a spawn_* declaration, other inline tasks, standalone operator binaries and external scheduling/deployment topology are not exhaustively indexed. Worker counts are declaration candidates, not running-process counts.",
            "This extractor does not independently enumerate every CLI subcommand, exported C ABI function, Tauri command adapter, GTK D-Bus activation or native service API. Many delegate to the indexed IPC/domain behavior, but equivalence is not inferred. Native local-service flows (including Insights and mission files) have source notes in their domain models; complete adapter/ABI inventories require additional scanning.",
            "Owner assignments are heuristic triage. Newly discovered entries default to gap. No row becomes detailed or coarse merely because its subsystem has a model file. Concrete action/observable mappings must be reviewed, and model-check results (if later authorized) remain separate evidence.",
        ],
        "excluded_primary_untracked_paths": previous.get("excluded_primary_untracked_paths", []),
        "entries": rows,
    }
    generated = {manifest_path: json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
                 HERE / "coverage.md": document(manifest)}
    dirty = []
    for path, value in generated.items():
        if args.check:
            if not path.exists() or path.read_text() != value:
                dirty.append(str(path.relative_to(ROOT)))
        else:
            path.write_text(value)
    print(json.dumps({"entries": len(rows), "kinds": dict(sorted(collections.Counter(r["kind"] for r in rows).items())),
                      "classifications": dict(sorted(collections.Counter(r["classification"] for r in rows).items())),
                      "ipc_dispatch_arm_counts": dispatch_counts, "source_commit": BASELINE}, sort_keys=True))
    if dirty:
        raise Unsupported("inventory is stale: " + ", ".join(dirty))


if __name__ == "__main__":
    try:
        main()
    except (Unsupported, subprocess.CalledProcessError) as error:
        print("inventory error: " + str(error), file=sys.stderr)
        sys.exit(1)

#!/usr/bin/env python3
"""Assert fixed live-source history callbacks with synthetic clients.

Native XCTest regressions are the primary app validation. This standalone
method-boundary replay extracts current source unchanged; no app, IPC, network,
credentials or application data are used. Historical failures remain in notes
and archived TLC reports, rather than being the default assertion here.
"""
from pathlib import Path
import hashlib
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / 'macos/Sources/TraceCommonsApp/AppModel.swift'
source = SOURCE.read_text()


def extract(start):
    begin = source.index(start)
    # Default closure arguments contain braces before the actual method body.
    parentheses = 0
    brace = None
    for position in range(begin, len(source)):
        if source[position] == '(':
            parentheses += 1
        elif source[position] == ')':
            parentheses -= 1
        elif source[position] == '{' and parentheses == 0:
            brace = position
            break
    if brace is None:
        raise ValueError('No source body: ' + start)
    depth = 0
    for i in range(brace, len(source)):
        if source[i] == '{':
            depth += 1
        elif source[i] == '}':
            depth -= 1
            if depth == 0:
                return source[begin:i + 1]
    raise ValueError('Unclosed method: ' + start)


methods = [extract(start) for start in [
    '    func refreshHistory()',
    '    private func invalidateAccountScopedReads()',
    '    private func accountReadIsCurrent(',
    '    private func perform<T: Sendable>',
]]
client_property = extract('    private var client: DaemonClient?')
# Keep the source's actual epoch identity representation and declarations.
epoch_class = extract('    private final class ReadEpoch') if 'private final class ReadEpoch' in source else ''
epoch_fields = '\n'.join(re.search(r'^    private var ' + field + r'\b[^\n]*', source, re.M).group(0) for field in [
    'clientReadGeneration', 'accountReadGeneration', 'statusReadGeneration', 'historyReadGeneration',
    'automaticGrantReadGeneration', 'automaticGrantMutations'])
print('source SHA256:', hashlib.sha256(source.encode()).hexdigest(), flush=True)
for method in methods:
    print(method.splitlines()[0].strip() + ' SHA256:', hashlib.sha256(method.encode()).hexdigest(), flush=True)

preamble = '''import Foundation
final class Gate: @unchecked Sendable {
 let condition = NSCondition(); var begun = false; var open = false
 func wait() { condition.lock(); begun = true; condition.broadcast(); while !open { condition.wait() }; condition.unlock() }
 func started() -> Bool { condition.lock(); defer { condition.unlock() }; return begun }
 func release() { condition.lock(); open = true; condition.broadcast(); condition.unlock() }
}
final class DaemonClient: Sendable {
 struct Failure: Error { let message: String }
 let owner: String; let gate: Gate?; let fails: Bool
 init(_ owner: String, _ gate: Gate? = nil, fails: Bool = false) { self.owner=owner; self.gate=gate; self.fails=fails }
 func listHistory() throws -> [String] { gate?.wait(); if fails { throw Failure(message: "synthetic-old-error") }; return [owner] }
 func historyRollup() throws -> String { owner }
}
struct ShellWords { struct Settings: Sendable { let projectModeFailed: String }; struct Table: Sendable { let settings: Settings }; static let table: Table? = nil }
struct Status { var accountScope: String? }
@MainActor final class Replay {
CLIENT_PROPERTY
EPOCH_CLASS
EPOCH_FIELDS
 var status = Status(accountScope: "old")
 var history: [String] = []; var historyAnswered = false; var rollup = ""; var lastActionError = ""
 var reads: [String: Int] = [:]
 func replaceClient(_ client: DaemonClient) { self.client = client }
 // Relevant accountScope didSet projection; main performs its new history read.
 // Unrelated contribution text/notices and grant refetch are omitted shims.
 func changeScope(_ scope: String) { status.accountScope = scope; invalidateAccountScopedReads() }
 func recordRead(_ label: String, answered: Bool) { reads[label, default: 0] += 1 }
 func publishIfChanged<T: Equatable>(_ key: ReferenceWritableKeyPath<Replay,T>, _ value: T) { self[keyPath:key] = value }
'''.replace('CLIENT_PROPERTY', client_property).replace('EPOCH_CLASS', epoch_class).replace('EPOCH_FIELDS', epoch_fields)
main = '''
}
@main struct Main {
 @MainActor static func main() async {
  let control = Replay(); control.replaceClient(DaemonClient("old"))
  let c = control.refreshHistory(); await c.history?.value; await c.rollup?.value
  precondition(control.history == ["old"] && control.rollup == "old")
  print("control: current replies publish history and rollup")
  for staleFails in [false, true] {
   let gate = Gate(); let app = Replay(); app.replaceClient(DaemonClient("old", gate, fails: staleFails))
   let old = app.refreshHistory()
   while !gate.started() { await Task.yield() }
   app.changeScope("new"); app.replaceClient(DaemonClient("new"))
   let fresh = app.refreshHistory(); await fresh.history?.value; await fresh.rollup?.value
   precondition(app.history == ["new"] && app.rollup == "new")
   let reads = app.reads
   gate.release(); await old.history?.value; await old.rollup?.value
   precondition(app.status.accountScope == "new" && app.history == ["new"] && app.rollup == "new")
   precondition(app.lastActionError.isEmpty && app.reads == reads)
   print(staleFails ? "fixed: stale failure discarded before error/read bookkeeping" : "fixed: late old-account success cannot replace current history")
  }
 }
}
'''
with tempfile.TemporaryDirectory(prefix='tc-history-source-replay-', dir='/private/tmp') as folder:
    folder = Path(folder)
    swift = folder / 'history_source_replay.swift'
    binary = folder / 'history_source_replay'
    swift.write_text(preamble + '\n'.join(methods) + main)
    subprocess.run(['swiftc', '-parse-as-library', '-swift-version', '6', '-module-cache-path', str(folder / 'module-cache'), str(swift), '-o', str(binary)], check=True, timeout=120)
    subprocess.run([str(binary)], check=True, timeout=20)

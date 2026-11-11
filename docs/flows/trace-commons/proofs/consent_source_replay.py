#!/usr/bin/env python3
"""Assert fixed live-source saved-grant callbacks with synthetic clients.

Extracted methods remain unchanged. Native XCTest is the primary app check;
this focused harness uses no app launch, IPC, network, credentials or data.
Historical failing behavior is retained in proof notes and archived TLC logs.
"""
from pathlib import Path
import hashlib
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
source = (ROOT / 'macos/Sources/TraceCommonsApp/AppModel.swift').read_text()
choice_source = (ROOT / 'macos/Sources/TraceCommonsApp/Views/Settings/SharingChoiceCard.swift').read_text()


def extract(text, start):
    begin = text.index(start)
    # Default closure arguments contain braces before the actual method body.
    parentheses = 0
    brace = None
    for position in range(begin, len(text)):
        if text[position] == '(':
            parentheses += 1
        elif text[position] == ')':
            parentheses -= 1
        elif text[position] == '{' and parentheses == 0:
            brace = position
            break
    if brace is None:
        raise ValueError('No source body: ' + start)
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == '{':
            depth += 1
        elif text[i] == '}':
            depth -= 1
            if depth == 0:
                return text[begin:i + 1]
    raise ValueError('Unclosed source method: ' + start)


methods = [extract(source, start) for start in [
    '    func refreshProjects()',
    '    private func refreshAutomaticGrant()',
    '    func setAskEachTime() async',
    '    private func firstRunCall<T: Sendable>',
    '    private func invalidateAccountScopedReads()',
    '    private func accountReadIsCurrent(',
    '    private func settleAutomaticGrantOwner()',
    '    private func beginAutomaticGrantMutation()',
    '    private func finishAutomaticGrantMutation(',
    '    private func perform<T: Sendable>',
]]
choice = extract(choice_source, '    static func current(_ grant: AutomaticGrant?)')
client_property = extract(source, '    private var client: DaemonClient?')
# Keep the source's actual epoch identity representation and declarations.
epoch_class = extract(source, '    private final class ReadEpoch') if 'private final class ReadEpoch' in source else ''
epoch_fields = '\n'.join(re.search(r'^    private var ' + field + r'\b[^\n]*', source, re.M).group(0) for field in [
    'clientReadGeneration', 'accountReadGeneration', 'statusReadGeneration', 'historyReadGeneration',
    'automaticGrantReadGeneration', 'automaticGrantMutations'])
for label, text in [('AppModel', source), ('SharingChoiceCard', choice_source)]:
    print(label + ' SHA256:', hashlib.sha256(text.encode()).hexdigest(), flush=True)
for body in methods + [choice]:
    print(body.splitlines()[0].strip() + ' SHA256:', hashlib.sha256(body.encode()).hexdigest(), flush=True)

preamble = '''import Foundation
struct AutomaticGrant: Equatable, Sendable { let granted: Bool; let includeExisting: Bool? }
enum SharingPath: Equatable { case automatic, askMe }
enum SharingChoice {
CHOICE
}
final class Gate: @unchecked Sendable {
 let condition = NSCondition(); var begun = false; var open = false
 func wait() { condition.lock(); begun = true; condition.broadcast(); while !open { condition.wait() }; condition.unlock() }
 func started() -> Bool { condition.lock(); defer { condition.unlock() }; return begun }
 func release() { condition.lock(); open = true; condition.broadcast(); condition.unlock() }
}
final class DaemonClient: @unchecked Sendable {
 struct Failure: Error { let message: String }
 let lock = NSLock(); var granted = true; var calls = 0; let gate: Gate; let failsOld: Bool
 init(_ gate: Gate, failsOld: Bool = false) { self.gate = gate; self.failsOld = failsOld }
 func listProjects() throws -> [String] { [] }
 func automaticGrant() throws -> AutomaticGrant {
  lock.lock(); let snapshot = granted; calls += 1; let first = calls == 1; lock.unlock()
  if first { gate.wait(); if failsOld { throw Failure(message: "synthetic-old-error") } }
  return AutomaticGrant(granted: snapshot, includeExisting: true)
 }
 func setAskEachTime() throws -> Bool { lock.lock(); defer { lock.unlock() }; let old = granted; granted = false; return old }
 func liveGrant() -> Bool { lock.lock(); defer { lock.unlock() }; return granted }
}
enum DaemonWork {
 static func run<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T { await Task.detached { work() }.value }
}
struct ShellWords { struct Settings: Sendable { let projectModeFailed: String }; struct Table: Sendable { let settings: Settings }; static let table: Table? = nil }
struct Status { var accountScope: String? }
@MainActor final class Replay {
CLIENT_PROPERTY
EPOCH_CLASS
EPOCH_FIELDS
 // This replay starts with an established owner. Unknown-owner preflight is
 // covered by the native XCTest regression, and must not be silently skipped.
 private enum CurrentStatusRead: Equatable { case published, unavailable, stale }
 private func readStatusForCurrentAccount() async -> CurrentStatusRead {
  preconditionFailure("unknown-owner path belongs to native XCTest")
 }
 var status = Status(accountScope: "current")
 var automaticGrant: AutomaticGrant? = AutomaticGrant(granted: true, includeExisting: true)
 var projects: [String] = []; var lastActionError = ""; var reads: [String:Int] = [:]
 func replaceClient(_ client: DaemonClient) { self.client = client }
 func refreshStatus() {}
 func refreshArmingOffer() {}
 func recordRead(_ label: String, answered: Bool) { reads[label,default:0] += 1 }
 func publishIfChanged<T: Equatable>(_ key: ReferenceWritableKeyPath<Replay,T>, _ value: T) { self[keyPath:key] = value }
'''.replace('CHOICE', choice).replace('CLIENT_PROPERTY', client_property).replace('EPOCH_CLASS', epoch_class).replace('EPOCH_FIELDS', epoch_fields)
main = '''
}
@main struct Main {
 @MainActor static func main() async {
  let controlGate = Gate(); controlGate.release(); let control = Replay(); control.replaceClient(DaemonClient(controlGate))
  await control.refreshProjects()?.value
  precondition(SharingChoice.current(control.automaticGrant) == .automatic)
  print("control: current completed grant is displayed Automatic")
  for staleFails in [false, true] {
   let gate = Gate(); let client = DaemonClient(gate, failsOld: staleFails); let app = Replay(); app.replaceClient(client)
   precondition(SharingChoice.current(app.automaticGrant) == .automatic)
   let old = app.refreshProjects()
   while !gate.started() { await Task.yield() }
   let saved = await app.setAskEachTime()
   precondition(saved && !client.liveGrant())
   await app.refreshProjects()?.value
   precondition(SharingChoice.current(app.automaticGrant) == .askMe)
   let reads = app.reads["automatic_grant"]
   gate.release(); await old?.value
   precondition(!client.liveGrant() && SharingChoice.current(app.automaticGrant) == .askMe)
   precondition(app.lastActionError.isEmpty && app.reads["automatic_grant"] == reads)
   print(staleFails ? "fixed: stale grant failure discarded before error/read bookkeeping" : "fixed: completed Ask remains selected after old Automatic reply")
  }
 }
}
'''
with tempfile.TemporaryDirectory(prefix='tc-consent-source-replay-', dir='/private/tmp') as folder:
    folder = Path(folder)
    swift = folder / 'consent_source_replay.swift'
    binary = folder / 'consent_source_replay'
    swift.write_text(preamble + '\n'.join(methods) + main)
    subprocess.run(['swiftc', '-parse-as-library', '-swift-version', '6', '-module-cache-path', str(folder / 'module-cache'), str(swift), '-o', str(binary)], check=True, timeout=120)
    subprocess.run([str(binary)], check=True, timeout=20)

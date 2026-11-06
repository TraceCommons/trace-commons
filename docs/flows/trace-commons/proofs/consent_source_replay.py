#!/usr/bin/env python3
"""Replay exact saved-grant callbacks against synthetic clients; no app/IPC/key use."""
from pathlib import Path
import hashlib
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
app_source = (ROOT / 'macos/Sources/TraceCommonsApp/AppModel.swift').read_text()
choice_source = (ROOT / 'macos/Sources/TraceCommonsApp/Views/Settings/SharingChoiceCard.swift').read_text()


def extract(source, start):
    begin = source.index(start)
    brace = source.index('{', begin)
    depth = 0
    for i in range(brace, len(source)):
        if source[i] == '{':
            depth += 1
        elif source[i] == '}':
            depth -= 1
            if depth == 0:
                return source[begin:i + 1]
    raise ValueError('Unclosed source method: ' + start)


methods = [extract(app_source, start) for start in [
    '    func refreshProjects()',
    '    func setAskEachTime() async',
    '    private func firstRunCall<T: Sendable>',
    '    private func perform<T: Sendable>',
]]
choice = extract(choice_source, '    static func current(_ grant: AutomaticGrant?)')
for label, source in [('AppModel', app_source), ('SharingChoiceCard', choice_source)]:
    print(label + ' SHA256:', hashlib.sha256(source.encode()).hexdigest(), flush=True)
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
 let lock = NSLock(); var granted = true; var calls = 0; let gate: Gate
 init(_ gate: Gate) { self.gate = gate }
 func listProjects() throws -> [String] { [] }
 func automaticGrant() throws -> AutomaticGrant {
  lock.lock(); let snapshot = granted; calls += 1; let first = calls == 1; lock.unlock()
  if first { gate.wait() }
  return AutomaticGrant(granted: snapshot, includeExisting: true)
 }
 func setAskEachTime() throws -> Bool { lock.lock(); defer { lock.unlock() }; let old = granted; granted = false; return old }
 func liveGrant() -> Bool { lock.lock(); defer { lock.unlock() }; return granted }
}
enum DaemonWork {
 static func run<T: Sendable>(_ work: @escaping @Sendable () -> T) async -> T { await Task.detached { work() }.value }
}
struct ShellWords { struct Settings: Sendable { let projectModeFailed: String }; struct Table: Sendable { let settings: Settings }; static let table: Table? = nil }
@MainActor final class Replay {
 var client: DaemonClient?; var automaticGrant: AutomaticGrant? = AutomaticGrant(granted: true, includeExisting: true)
 var projects: [String] = []; var lastActionError = ""
 func refreshStatus() {}
 func refreshArmingOffer() {}
 func recordRead(_ label: String, answered: Bool) {}
 func publishIfChanged<T: Equatable>(_ key: ReferenceWritableKeyPath<Replay,T>, _ value: T) { self[keyPath:key] = value }
'''.replace('CHOICE', choice)
main = '''
}
@main struct Main {
 @MainActor static func main() async {
  let gate = Gate(); let client = DaemonClient(gate); let app = Replay(); app.client = client
  precondition(SharingChoice.current(app.automaticGrant) == .automatic)
  app.refreshProjects()
  while !gate.started() { await Task.yield() }
  let saved = await app.setAskEachTime()
  precondition(saved && !client.liveGrant())
  while SharingChoice.current(app.automaticGrant) != .askMe { await Task.yield() }
  print("Ask completed: daemon grant=false; displayed choice=Ask")
  gate.release()
  while SharingChoice.current(app.automaticGrant) != .automatic { await Task.yield() }
  precondition(!client.liveGrant())
  print("late old read: daemon grant=false; displayed choice=Automatic")
 }
}
'''
with tempfile.TemporaryDirectory(prefix='tc-consent-source-replay-', dir='/private/tmp') as tmp:
    tmp = Path(tmp)
    swift = tmp / 'consent_source_replay.swift'
    binary = tmp / 'consent_source_replay'
    swift.write_text(preamble + '\n'.join(methods) + main)
    subprocess.run(['swiftc', '-parse-as-library', '-swift-version', '6', '-module-cache-path', str(tmp / 'module-cache'), str(swift), '-o', str(binary)], check=True, timeout=120)
    subprocess.run([str(binary)], check=True, timeout=15)

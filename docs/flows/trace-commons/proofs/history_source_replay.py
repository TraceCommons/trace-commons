#!/usr/bin/env python3
"""Replay exact current AppModel history callbacks with synthetic Swift clients.

No native app launch, IPC, credentials, or production source modifications.
The stand-in account property represents status.accountScope; the current
status.didSet has no history reset/cancellation hook. This is method-boundary
source replay, not a complete native UI integration test.
"""
from pathlib import Path
import hashlib
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / 'macos/Sources/TraceCommonsApp/AppModel.swift'
src=SOURCE.read_text()
def extract(start):
 begin=src.index(start); brace=src.index('{',begin);depth=0
 for i in range(brace,len(src)):
  if src[i]=='{':depth+=1
  elif src[i]=='}':
   depth-=1
   if depth==0:return src[begin:i+1]
perform=extract('    private func perform<T: Sendable>')
refresh=extract('    func refreshHistory()')
print('source HEAD:', subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
print('source SHA256:', hashlib.sha256(src.encode()).hexdigest())
for label, body in [('refreshHistory', refresh), ('perform', perform)]:
 print(label + ' SHA256:', hashlib.sha256(body.encode()).hexdigest())
preamble=''' import Foundation
final class Gate: @unchecked Sendable {
 let condition = NSCondition(); var begun = false; var open = false
 func wait() { condition.lock(); begun = true; condition.broadcast(); while !open { condition.wait() }; condition.unlock() }
 func started() -> Bool { condition.lock(); defer { condition.unlock() }; return begun }
 func release() { condition.lock(); open = true; condition.broadcast(); condition.unlock() }
}
final class DaemonClient: Sendable {
 struct Failure: Error { let message: String }
 let owner: String; let gate: Gate?
 init(_ owner: String, _ gate: Gate? = nil) { self.owner=owner; self.gate=gate }
 func listHistory() throws -> [String] { gate?.wait(); return [owner] }
 func historyRollup() throws -> String { owner }
}
struct ShellWords { struct Settings: Sendable { let projectModeFailed: String }; struct Table: Sendable { let settings: Settings }; static let table: Table? = nil }
@MainActor final class Replay {
 var client: DaemonClient?; var history: [String] = []; var historyAnswered = false; var rollup = ""; var lastActionError = ""; var account = "old"
 func recordRead(_ label: String, answered: Bool) {}
 func publishIfChanged<T: Equatable>(_ key: ReferenceWritableKeyPath<Replay,T>, _ value: T) { self[keyPath:key] = value }
'''
main='''
}
@main struct Main {
 @MainActor static func main() async {
  let controlGate = Gate(); let control = Replay(); control.client = DaemonClient("old",controlGate); control.refreshHistory()
  while !controlGate.started() { await Task.yield() }
  controlGate.release()
  while control.history != ["old"] { await Task.yield() }
  precondition(control.history == [control.account])
  print("control: no account change; history matches current account")
  let gate = Gate(); let app = Replay(); app.client = DaemonClient("old",gate); app.refreshHistory()
  while !gate.started() { await Task.yield() }
  app.account = "new"; app.client = DaemonClient("new"); app.refreshHistory()
  while app.history != ["new"] { await Task.yield() }
  print("current account=new; new history published=\\(app.history)")
  gate.release()
  while app.history != ["old"] { await Task.yield() }
  precondition(app.account == "new" && app.history == ["old"])
  print("late old result replaced current history=\\(app.history)")
 }
}
'''
with tempfile.TemporaryDirectory(prefix='tc-history-source-replay-', dir='/private/tmp') as tmp:
 tmp = Path(tmp)
 swift = tmp / 'history_source_replay.swift'
 binary = tmp / 'history_source_replay'
 swift.write_text(preamble+refresh+'\n'+perform+main)
 subprocess.run(['swiftc', '-parse-as-library', '-swift-version', '6', '-module-cache-path', str(tmp / 'module-cache'), str(swift), '-o', str(binary)], check=True, timeout=120)
 subprocess.run([str(binary)], check=True, timeout=15)


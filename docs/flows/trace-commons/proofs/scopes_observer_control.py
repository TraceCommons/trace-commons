#!/usr/bin/env python3
"""Positive/fault-injection controls for the actual P2 pre-send observer.

The production projection is copied unchanged. A temporary test fixture sets
an already claimed row, then invokes its actual pre-send action. The negative
control substitutes a new scope into the context without a matching approval;
this is deliberate test injection, not a claimed reachable product trace.
"""
from pathlib import Path
import hashlib
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
model = ROOT / 'docs/flows/trace-commons/models/client_contribution.qnt'
source = model.read_text()
print('model SHA256:', hashlib.sha256(source.encode()).hexdigest(), flush=True)
tests = '''
  action prepareSendContext(scope: int): bool = s' = { ...s,
    upload: "checking", uploadEntry: 0, uploadScopes: scope,
    entries: s.entries.set(0, { ...s.entries.get(0), state: "uploading", approvedScopes: 0, approvedInputs: 0 }) }
  run approved_scope_control = init
    .then(prepareSendContext(0))
    .then(TheUploadPassesItsPreconditions)
    .then(all { assert(P2 and s.sentScopes == 0), s' = s })
  run unapproved_scope_control = init
    .then(prepareSendContext(1))
    .then(TheUploadPassesItsPreconditions)
    .then(all { assert(not(P2) and s.sentScopes == 1), s' = s })
}
'''
with tempfile.TemporaryDirectory(prefix='tc-scopes-observer-control-', dir='/private/tmp') as tmp:
    temporary_model = Path(tmp) / 'client_contribution.qnt'
    temporary_model.write_text(source.rsplit('}', 1)[0] + tests)
    subprocess.run(['quint', 'test', str(temporary_model), '--main', 'client_contribution', '--match', 'scope_control', '--max-samples', '1', '--backend', 'typescript'], check=True, timeout=30)

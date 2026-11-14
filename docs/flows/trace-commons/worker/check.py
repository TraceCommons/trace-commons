#!/usr/bin/env python3
"""Reproduce worker model checks; sampled/bounded success is never an unbounded proof."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(HERE.parent / 'scripts'))
from induction_checks import classify_obligation

MODEL = HERE / 'witness_worker.qnt'
TESTS = HERE / 'witness_worker_tests.qnt'
SOURCES = [ROOT / 'crates/trace-commons-contributor/src' / path for path in
           ['daemon/witness_worker.rs', 'daemon/witness_retry.rs', 'daemon/queue.rs', 'daemon/mod.rs', 'submit.rs']]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def classify(kind, code, log, timed_out=False, changed=False):
    if timed_out or changed:
        return 'incomplete'
    if kind == 'bounded':
        verdict = classify_obligation(log, code)
        return {'proved': 'no-counterexample-through-bound', 'counterexample': 'counterexample'}.get(verdict, 'incomplete')
    if code != 0:
        return 'failed'
    if kind == 'typecheck':
        return 'passed'
    if kind == 'tests':
        return 'passed' if re.search(r'\b[1-9]\d* passing\b', log) and not re.search(r'\b[1-9]\d* failing\b', log) else 'incomplete'
    return 'sampled-no-counterexample' if '[ok]' in log and 'No violation found' in log else 'incomplete'


def run_check(name, kind, command, args, env):
    folder = args.out / name
    folder.mkdir()
    logpath = folder / 'check.log'
    before = {str(p.relative_to(ROOT)): sha(p) for p in [MODEL, TESTS, Path(__file__), HERE / "test_check.py", *SOURCES]}
    timed_out = False
    start = time.monotonic()
    with logpath.open('w') as log:
        proc = subprocess.Popen(command, cwd=folder, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = proc.wait(timeout=args.seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(proc.pid, signal.SIGTERM)
            try:
                code = proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                code = proc.wait()
    after = {str(p.relative_to(ROOT)): sha(p) for p in [MODEL, TESTS, Path(__file__), HERE / "test_check.py", *SOURCES]}
    log = logpath.read_text()
    result = {'name': name, 'kind': kind, 'status': classify(kind, code, log, timed_out, before != after),
              'command': command, 'exitCode': code, 'timedOut': timed_out, 'artifactsChanged': before != after,
              'seconds': round(time.monotonic() - start, 3), 'budgetSeconds': args.seconds,
              'hashes': before, 'log': str(logpath.relative_to(args.out)), 'logSha256': sha(logpath)}
    (folder / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(name + ': ' + result['status'], flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--samples', type=int, default=1000)
    parser.add_argument('--steps', type=int, default=100)
    parser.add_argument('--bound', type=int, default=0, help='Optional bounded search; 0 skips it. Use induction.py for full-step safety.')
    parser.add_argument('--seconds', type=int, default=240)
    parser.add_argument('--port', type=int, default=8898)
    args = parser.parse_args()
    args.out = args.out.resolve()
    if args.out.exists() or ROOT == args.out or ROOT in args.out.parents:
        parser.error('Use a fresh evidence directory outside the checkout')
    if min(args.samples, args.steps, args.seconds) < 1 or args.bound < 0:
        parser.error('Samples, steps and budget must be positive; symbolic bound must be nonnegative')
    args.out.mkdir(parents=True)
    env = dict(os.environ)
    java = Path('/opt/homebrew/opt/openjdk@21/bin')
    if java.exists():
        env['PATH'] = str(java) + ':' + env['PATH']
    env.setdefault('JDK_JAVA_OPTIONS', '-Xmx3g')
    commands = [
        ('model-typecheck', 'typecheck', ['quint', 'typecheck', str(MODEL)]),
        ('tests-typecheck', 'typecheck', ['quint', 'typecheck', str(TESTS)]),
        ('regressions', 'tests', ['quint', 'test', str(TESTS), '--backend', 'rust', '--seed', '20261114', '--max-samples', '1']),
        ('simulation', 'simulation', ['quint', 'run', str(MODEL), '--backend', 'rust', '--seed', '20261114',
            '--max-samples', str(args.samples), '--max-steps', str(args.steps), '--invariant', 'safety', '--verbosity', '1']),
    ]
    if args.bound:
        commands.append(('bounded', 'bounded', ['quint', 'verify', str(MODEL), '--invariant', 'safety', '--max-steps', str(args.bound),
            '--server-endpoint', 'localhost:' + str(args.port), '--apalache-version', '0.62.1', '--verbosity', '3']))
    results = []
    for name, kind, command in commands:
        result = run_check(name, kind, command, args, env)
        results.append(result)
        report = {'kind': 'source projection: deterministic regressions, sampled invariants, bounded symbolic safety',
                  'unboundedProof': False, 'temporalFairnessChecked': False,
                  'createdAt': datetime.now(timezone.utc).isoformat(),
                  'quintVersion': subprocess.check_output(['quint', '--version'], text=True).strip(),
                  'nodeVersion': subprocess.check_output(['node', '--version'], text=True).strip(),
                  'javaVersion': subprocess.check_output(['java', '-version'], env=env, stderr=subprocess.STDOUT, text=True).strip(),
                  'sourceCommit': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
                  'samples': args.samples, 'sampleSteps': args.steps, 'symbolicBound': args.bound or None,
                  'results': results}
        (args.out / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        if result['status'] not in {'passed', 'sampled-no-counterexample', 'no-counterexample-through-bound'}:
            return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Reproduce complete induction obligations; semantic harness review is separate."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from induction_checks import classify_obligation, certificate_status
from proof_checks import selected_for

FLOW = Path(__file__).resolve().parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_phase(command, cwd, seconds):
    started = time.monotonic()
    timed_out = False
    with (cwd / 'verify.log').open('w') as log:
        process = subprocess.Popen(command, cwd=cwd, stdout=log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        try:
            code = process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                code = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                code = process.wait()
    return code, timed_out, round(time.monotonic() - started, 2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--spec', type=Path, required=True, help='Reviewed obligation definitions')
    parser.add_argument('--out', type=Path, required=True, help='Fresh output outside the flow tree')
    parser.add_argument('--model', action='append', default=[])
    parser.add_argument('--endpoint', default='localhost:8860', help='Exclusive Apalache endpoint')
    parser.add_argument('--seconds', type=int, default=600, help='Budget per obligation; never a proof bound')
    args = parser.parse_args()
    out = args.out.resolve()
    if out.exists() or out == FLOW or FLOW in out.parents or args.seconds < 1:
        parser.error('Use a fresh output outside the flow tree and a positive time budget')
    spec = json.loads(args.spec.read_text())
    configs = spec['models']
    if not isinstance(configs, list) or not configs:
        parser.error('The spec must contain at least one reviewed model')
    if len({m['model'] for m in configs}) != len(configs):
        parser.error('Duplicate model definitions')
    if set(args.model) - {m['model'] for m in configs}:
        parser.error('Unknown requested model')
    configs = [m for m in configs if not args.model or m['model'] in args.model]
    out.mkdir(parents=True)
    records = []
    for config in configs:
        model = config['model']
        chosen = selected_for(FLOW, model)
        if not chosen:
            parser.error(f'{model}: no selected safety hypotheses')
        harness = (FLOW / 'proofs' / config['harness']).resolve()
        if harness.parent != FLOW / 'proofs' or not harness.is_file():
            parser.error(f'{model}: harness must be an existing proof file')
        paths = {'modelSha256': FLOW / 'models' / f'{model}.qnt', 'harnessSha256': harness,
                 'configSha256': FLOW / 'models' / f'{model}.cfg',
                 'selectionSha256': FLOW / 'promise-selection.json', 'specSha256': args.spec.resolve()}
        before = {key: sha(path) for key, path in paths.items()}
        target = config['inductive']
        conclusion = ' and '.join(p['definition'] for p in chosen)
        phases = [('initiation', config['originalInit'], target, 0),
                  ('consecution', config['arbitraryInit'], target, 1),
                  ('implication', config['arbitraryInit'], f'({target}) and ({conclusion})', 0),
                  ('nonvacuity', config['arbitraryInit'], f'({target}) and false', 0)]
        folder = out / model
        folder.mkdir()
        phase_records = []
        for phase, initializer, invariant, steps in phases:
            directory = folder / phase
            directory.mkdir()
            command = ['quint', 'verify', str(harness), '--main', config['module'],
                       '--init', initializer, '--step', config['originalStep'],
                       '--invariant', invariant, '--max-steps', str(steps),
                       '--server-endpoint', args.endpoint, '--verbosity', '2',
                       '--out-itf', str(directory / 'counterexample.itf.json')]
            code, timeout, elapsed = run_phase(command, directory, args.seconds)
            after = {key: sha(path) for key, path in paths.items()}
            changed = before != after
            log = (directory / 'verify.log').read_text()
            record = {'phase': phase, 'command': command, 'exitCode': code,
                      'timedOut': timeout, 'artifactsChanged': changed, 'elapsedSeconds': elapsed,
                      **before, 'logSha256': sha(directory / 'verify.log'),
                      'status': classify_obligation(log, code, timed_out=timeout, artifacts_changed=changed)}
            (directory / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            phase_records.append(record)
            print(f'{model} {phase}: {record["status"]}', flush=True)
            expected = 'counterexample' if phase == 'nonvacuity' else 'proved'
            if record['status'] != expected:
                break
        changed = before != {key: sha(path) for key, path in paths.items()}
        status = 'incomplete' if changed else certificate_status(phase_records)
        result = {'model': model, 'harness': str(harness), 'status': status, **before,
                  'semanticReviewRequired': 'Generator covers Domain+Strength; initiation/closure use original init/full step; target implies selected predicates',
                  'selected': chosen, 'phases': phase_records}
        (folder / 'certificate.json').write_text(json.dumps(result, indent=2) + '\n')
        records.append(result)
    evidence = {'recordedAtUTC': datetime.now(timezone.utc).isoformat(),
                'kind': 'induction obligations over original full transitions; source correspondence is separately reviewed',
                'quintVersion': subprocess.check_output(['quint', '--version'], text=True).strip(),
                'models': records}
    (out / 'certificates.json').write_text(json.dumps(evidence, indent=2) + '\n')
    return 0 if all(m['status'] == 'proved' for m in records) else 1


if __name__ == '__main__':
    raise SystemExit(main())

#!/usr/bin/env python3
"""Run selected full-model TLC checks; resource stops are never reported as proofs."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from proof_checks import selected_for, report_results

FLOW = Path(__file__).resolve().parent.parent


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_model(model, args):
    chosen = selected_for(FLOW, model)
    folder = args.out / model
    folder.mkdir()
    command = ['python3', str(FLOW / 'scripts/check_selected.py'), model,
               '--skill-root', str(args.skill_root), '--out', str(folder / 'check')]
    paths = {key: FLOW / 'models' / f'{model}{suffix}' for key, suffix in
             [('modelSha256', '.qnt'), ('configSha256', '.cfg'), ('viewSha256', '.view.mts')]}
    paths['selectionSha256'] = FLOW / 'promise-selection.json'
    before = {key: sha(path) for key, path in paths.items()}
    started = time.monotonic()
    env = dict(os.environ)
    env['TMPDIR'] = str(folder)
    timed_out = False
    with (folder / 'command.log').open('w') as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT,
                                   env=env, start_new_session=True)
        try:
            code = process.wait(timeout=args.seconds)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                code = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                code = process.wait()
    report_file = folder / 'check/report.md'
    report = report_file.read_text() if report_file.exists() else ''
    after = {key: sha(path) for key, path in paths.items()}
    changed = before != after
    result = {'model': model, 'command': command, 'exitCode': code,
              'timedOut': timed_out, 'elapsedSeconds': round(time.monotonic() - started, 2),
              'budgetSeconds': args.seconds, 'budgetKind': 'outer wall-clock process budget; no input-model constraint',
              'checkerFollowupBound': 'Pinned statechart caps reruns after a first break at 100000 distinct states; capped rules are not checked',
              **before, 'artifactsChangedDuringRun': changed,
              'results': report_results(report, [p['definition'] for p in chosen],
                                        timed_out=timed_out or changed, exit_code=code)}
    if changed:
        for verdict in result['results']:
            verdict['status'] = 'not checked'
    (folder / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(f'{model}: exit={code} timeout={timed_out} {result["results"]}', flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skill-root', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--model', action='append', default=[])
    parser.add_argument('--seconds', type=int, default=180)
    parser.add_argument('--jobs', type=int, default=1, help='Default 1 avoids races while Quint starts its compiler server')
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.skill_root = args.skill_root.resolve()
    if args.out.exists() or FLOW == args.out or FLOW in args.out.parents:
        parser.error('Use a fresh output directory outside the committed seed')
    if args.seconds < 1 or not 1 <= args.jobs <= 4:
        parser.error('Positive seconds and 1–4 jobs required')
    selection = json.loads((FLOW / 'promise-selection.json').read_text())
    names = sorted({p['model'] for p in selection['promises'] if p.get('selected') is True})
    if set(args.model) - set(names):
        parser.error('Unknown or unselected model')
    names = args.model or names
    for name in names:
        selected_for(FLOW, name)
    args.out.mkdir(parents=True)
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        results = list(executor.map(lambda name: run_model(name, args), names))
    evidence = {'kind': 'selected safety properties in bounded source models; not source equivalence',
                'finishedAtUTC': datetime.now(timezone.utc).isoformat(),
                'selectionSha256': sha(FLOW / 'promise-selection.json'),
                'models': results}
    (args.out / 'results.json').write_text(json.dumps(evidence, indent=2) + '\n')
    return 0 if all(r['status'] == 'holds' for m in results for r in m['results']) else 1


if __name__ == '__main__':
    raise SystemExit(main())

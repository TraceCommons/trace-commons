#!/usr/bin/env python3
"""Typecheck and simulate artifacts; never claims to verify product promises."""
import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from proof_checks import selected_for

FLOW = Path(__file__).resolve().parent.parent
MODELS = FLOW / 'models'
REQUIRED = {'client_shell', 'client_contribution', 'client_services', 'client_learning',
            'client_consent', 'client_providers', 'platform_clients',
            'server_identity', 'server_receipt', 'server_pipeline', 'server_settlement', 'server_lifecycle',
            'server_interfaces', 'end_to_end'}


def run(command, log=None):
    result = subprocess.run(command, text=True, capture_output=True, timeout=300)
    if log:
        log.write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"command failed ({result.returncode}): {' '.join(map(str, command))}\n{result.stdout}{result.stderr}")
    return result.stdout.strip()


def validate_frames(name, models=MODELS):
    frames = json.loads((models / f'{name}.frames.json').read_text())
    if not isinstance(frames, list) or not frames:
        raise ValueError(f'{name}: frames must be a nonempty array')
    seen = set()
    expressions = []
    for f in frames:
        if not all(isinstance(f.get(key), str) and f[key] for key in ('label', 'title')) or not isinstance(f.get('body'), str):
            raise ValueError(f'{name}: missing frame content')
        if f['label'] in seen:
            raise ValueError(f'{name}: duplicate frame {f["label"]}')
        seen.add(f['label'])
        controls = f.get('controls')
        if not isinstance(controls, list) or not all(isinstance(c, str) for c in controls) or len(set(controls)) != len(controls):
            raise ValueError(f'{name}: invalid controls')
        if f.get('kind') not in ('client', 'server'):
            raise ValueError(f'{name}: invalid frame kind')
        args = ', '.join(json.dumps(c) for c in controls)
        expressions.append(f'controls({json.dumps(f["label"])}) == Set({args})')
    output = run(['quint', 'repl', '-q', '-r', f'{models / (name + ".qnt")}::{name}', *expressions, '.exit'])
    lines = output.splitlines()
    if lines != ['true'] * len(frames):
        failed = [frames[i]['label'] for i, line in enumerate(lines[:len(frames)]) if line != 'true']
        raise ValueError(f'{name}: model/frame control mismatch: {failed}; output={output}')
    return len(frames)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True, help='New directory outside the model seed')
    parser.add_argument('--samples', type=int, default=128)
    parser.add_argument('--steps', type=int, default=100)
    parser.add_argument('--model', action='append', default=[], help='Recheck only this model after a scoped change; repeatable')
    args = parser.parse_args()
    out = args.out.resolve()
    if out.exists():
        raise ValueError('Use a new output directory to prevent stale evidence')
    if FLOW == out or FLOW in out.parents:
        raise ValueError('Keep simulation output outside the committed flow directory')
    if args.samples < 1 or args.steps < 1:
        raise ValueError('samples and steps must be positive')
    out.mkdir(parents=True)
    models = sorted(MODELS.glob('*.qnt'))
    names = {p.stem for p in models}
    if not REQUIRED.issubset(names):
        raise ValueError(f'Missing required models: {sorted(REQUIRED - names)}')
    if set(args.model) - names:
        raise ValueError(f'Unknown requested models: {sorted(set(args.model) - names)}')
    if args.model:
        models = [p for p in models if p.stem in args.model]
    evidence = {'kind': 'typechecking and random simulation only; no selected product promises checked',
                'startedAtUTC': datetime.now(timezone.utc).isoformat(),
                'modelSeedBaseline': '8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6',
                'initialSelectedSeamReviewBase': 'a2e0aa56608f8c403deff679f8b51e76ac2f5ca7',
                'sourceCorrespondenceEvidence': 'proofs/evidence/manifest.json; source review is separate from structural checks',
                'quintVersion': run(['quint', '--version']), 'nodeVersion': run(['node', '--version']),
                'pythonVersion': sys.version.split()[0], 'samplesPerModel': args.samples,
                'maximumStepsPerTrace': args.steps, 'seed': '20261105', 'models': []}
    run(['node', str(FLOW / 'scripts/test_itf.mjs')], out / 'itf-tests.log')
    for model in models:
        name = model.stem
        selected_for(FLOW, name)
        if re.search(r'(?m)^\s*temporal\s', model.read_text()):
            raise ValueError(f'{name}: temporal checks require a separate authorized checker')
        for suffix in ('.view.mts', '.frames.json', '.md'):
            if not (MODELS / (name + suffix)).is_file():
                raise ValueError(f'{name}: missing {suffix}')
        run(['quint', 'typecheck', str(model)], out / f'{name}-typecheck.log')
        count = validate_frames(name)
        command = ['quint', 'run', str(model), '--backend', 'typescript', '--seed', '20261105',
                   '--max-samples', str(args.samples), '--n-traces', str(args.samples),
                   '--max-steps', str(args.steps), '--mbt', '--verbosity', '1',
                   '--out-itf', str(out / f'{name}-{{seq}}.json')]
        run(command, out / f'{name}-simulation.log')
        actions = re.findall(r'(?m)^\s*action\s+(\w+)\s*=', model.read_text())
        evidence['models'].append({'name': name, 'typecheck': 'passed', 'controlFrames': count,
                                   'modelSha256': hashlib.sha256(model.read_bytes()).hexdigest(),
                                   'viewSha256': hashlib.sha256((MODELS / f'{name}.view.mts').read_bytes()).hexdigest(),
                                   'framesSha256': hashlib.sha256((MODELS / f'{name}.frames.json').read_bytes()).hexdigest(),
                                   'simulationCommand': command, 'declaredActions': sorted(set(actions) - {'step'})})
        print(f'{name}: typechecked, {count} frame/control projections matched, simulated', flush=True)
    run(['node', str(FLOW / 'scripts/inspect_views.mjs'), str(MODELS), str(out), str(out / 'views.json'),
         *[p.stem for p in models]], out / 'views.log')
    evidence['views'] = json.loads((out / 'views.json').read_text())
    for model, view in zip(evidence['models'], evidence['views']):
        if model['name'] != view['name']:
            raise ValueError('view result order mismatch')
        model['unobservedActions'] = sorted(set(model['declaredActions']) - set(view['observedActions']))
    evidence['finishedAtUTC'] = datetime.now(timezone.utc).isoformat()
    (out / 'verification.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(f'Evidence: {out / "verification.json"}. Simulation is not exhaustive verification.')


if __name__ == '__main__':
    main()

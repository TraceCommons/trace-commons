#!/usr/bin/env python3
"""Generate and check full-step induction harnesses for the client source models.

No model transition is rewritten. Harnesses import init and the complete step.
The arbitrary initializer covers every original record field and exact map key.
Primitive records use symbolic generate(1), then the explicit domain/strength
filter; this avoids enumerating the finite product while covering all its values.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

FLOW = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(FLOW / 'scripts'))
from induction_checks import classify_obligation, certificate_status


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def without_ids(value):
    if isinstance(value, dict):
        return {k: without_ids(v) for k, v in value.items() if k != 'id'}
    if isinstance(value, list):
        return [without_ids(v) for v in value]
    return value


def generate(model):
    path = FLOW / 'models' / (model + '.qnt')
    source = path.read_text()
    with tempfile.TemporaryDirectory(prefix='tc-client-induction-ast-', dir='/private/tmp') as folder:
        ast_path = Path(folder) / 'ast.json'
        subprocess.run(['quint', 'parse', str(path), '--out', str(ast_path)], check=True, timeout=60)
        ast = json.loads(ast_path.read_text())
    declarations = ast['modules'][0]['declarations']
    aliases = {d['name']: d['type'] for d in declarations if d['kind'] == 'typedef'}
    state_type = next(d['typeAnnotation'] for d in declarations if d['kind'] == 'var' and d['name'] == 's')
    strings = set()
    def visit(value):
        if isinstance(value, dict):
            if value.get('kind') == 'str' and 'value' in value:
                strings.add(value['value'])
            for v in value.values():
                visit(v)
        elif isinstance(value, list):
            for v in value:
                visit(v)
    visit(ast['modules'])
    bounds, choices, coverage, record_types, record_names = [], [], [], [], {}
    index_fields = {'selected', 'uploadEntry', 'previewEntry', 'withdrawalEntry', 'checkEntry', 'buildEntry', 'sourceRoot'}
    def resolve(t):
        return resolve(aliases[t['name']]) if t['kind'] == 'const' else t
    def render(t):
        if t['kind'] == 'const':
            return t['name']
        if t['kind'] == 'rec':
            return '{ ' + ', '.join(f['fieldName'] + ': ' + render(f['fieldType']) for f in t['fields']['fields']) + ' }'
        if t['kind'] == 'fun':
            return '(' + render(t['arg']) + ' -> ' + render(t['res']) + ')'
        return t['kind']
    def primitive_record(t):
        t = resolve(t)
        return t['kind'] == 'rec' and all(resolve(f['fieldType'])['kind'] in {'int', 'str', 'bool'} for f in t['fields']['fields'])
    def record_alias(t):
        if t['kind'] == 'const':
            return t['name']
        key = json.dumps(without_ids(t), sort_keys=True)
        if key not in record_names:
            name = 'ArbitraryRecord' + str(len(record_names))
            record_names[key] = name
            record_types.append('  type ' + name + ' = ' + render(t))
        return record_names[key]
    def scalar_domain(t, path):
        if t['kind'] == 'bool':
            return 'booleans'
        if t['kind'] == 'str':
            return 'texts'
        leaf = path.rsplit('.', 1)[-1]
        if leaf in index_fields or '.historyEntry.get(' in path:
            return 'Set(0, 1)'
        if leaf in {'grantId', 'generation'} or path == 't.account':
            return 'Set(1, 2)'
        return 'ints'
    def add_bounds(t, path, variable):
        t = resolve(t)
        if t['kind'] == 'rec':
            for f in t['fields']['fields']:
                add_bounds(f['fieldType'], path + '.' + f['fieldName'], variable + '.' + f['fieldName'])
        else:
            domain = scalar_domain(t, path)
            bounds.append(domain + '.contains(' + path + ')')
            coverage.append({'path': path, 'type': t['kind'], 'domain': domain, 'arbitraryValue': variable})
    def construct(t, path):
        original = t
        t = resolve(t)
        if primitive_record(original):
            alias = record_alias(original)
            name = 'a' + str(len(choices))
            choices.append('    nondet ' + name + ': ' + alias + ' = apalache::generate(1)')
            add_bounds(t, path, name)
            return name
        if t['kind'] == 'rec':
            return '{ ' + ', '.join(f['fieldName'] + ': ' + construct(f['fieldType'], path + '.' + f['fieldName']) for f in t['fields']['fields']) + ' }'
        if t['kind'] == 'fun':
            field = path.rsplit('.', 1)[-1]
            keys = [1, 2] if field in {'historyPending', 'historyCurrent', 'historyStale', 'historyEntry'} else [0, 1]
            bounds.append(path + '.keys() == Set(' + ', '.join(map(str, keys)) + ')')
            coverage.append({'path': path, 'type': 'map', 'keys': keys})
            return 'Map(' + ', '.join(str(i) + ' -> ' + construct(t['res'], path + '.get(' + str(i) + ')') for i in keys) + ')'
        name = 'a' + str(len(choices))
        choices.append('    nondet ' + name + ': ' + t['kind'] + ' = apalache::generate(1)')
        add_bounds(t, path, name)
        return name
    candidate = construct(state_type, 't')
    if model == 'client_consent':
        conclusion = 'P1'
        strength = '''all {
    t.proofP1,
    Set(0, 1).forall(i => t.folders.get(i).mode == "auto" implies t.folders.get(i).completedAuthorization),
    t.override == "auto" implies t.overrideAuthorization,
    Set(0, 1).forall(i => t.entries.get(i).origin == "automatic" implies t.entries.get(i).automaticAuthorizationCompleted),
    t.sharingRead == "automatic" implies t.sharingGrantCompleted,
    t.sharingRead != "none" implies (t.sharingReadCurrent and not(t.grantMutation) and t.withdrawalCall == "none")
  }'''
    else:
        conclusion = 'P2 and P3 and P4 and P12'
        strength = '''all {
    t.proof.P2, t.proof.P3, t.proof.P4, t.proof.P12,
    Set(0, 1).forall(i => {
      val e = t.entries.get(i)
      all {
        Set(0, 1).contains(e.sourceRev), e.offeredRev == 0,
        Set(0, 1).contains(e.pinnedRev),
        e.state == "absent" implies e.sourceRev == 0,
        e.sourceRev == 0 implies e.pinnedRev == 0
      }
    }),
    t.upload != "none" implies (t.entries.get(t.uploadEntry).state == "uploading" and t.entries.get(t.uploadEntry).approvedScopes == t.uploadScopes)
  }'''
    name = model + '_induction'
    harness = FLOW / 'proofs' / (name + '.qnt')
    text = '\n'.join([
        '// Generated by client_induction.py; imports the entire unchanged original step.',
        'module ' + name + ' {',
        '  import ' + model + '.* from "../models/' + model + '"',
        '  type State = ' + render(state_type),
        *record_types,
        '  pure val ints = Set(-2, -1, 0, 1, 2, 3)',
        '  pure val texts = Set(' + ', '.join(json.dumps(s) for s in sorted(strings)) + ')',
        '  pure val booleans = Set(false, true)',
        '  pure def domainFor(t: State): bool = all {\n    ' + ',\n    '.join(bounds) + '\n  }',
        '  pure def strongFor(t: State): bool = ' + strength,
        '  val strengthened = domainFor(s) and strongFor(s)',
        '  val Conclusion = ' + conclusion,
        '  val implication = not(strengthened) or Conclusion',
        '  action arbitraryInit = {',
        *choices,
        '    val candidate: State = ' + candidate,
        "    all { domainFor(candidate), strongFor(candidate), s' = candidate }",
        '  }',
        '  action originalInit = init',
        '  action originalStep = step',
        '}',
    ]) + '\n'
    harness.write_text(text)
    # There is one constructor value and one membership condition for every
    # scalar leaf, and every map is constructed with exactly the domain keys.
    scalar_paths = [c['path'] for c in coverage if c['type'] != 'map']
    assert len(scalar_paths) == len(set(scalar_paths))
    for c in coverage:
        if c['type'] != 'map':
            assert c['domain'] + '.contains(' + c['path'] + ')' in text
    metadata = {'model': model, 'modelSha256': sha(path), 'harnessSha256': sha(harness),
                'fullStepLink': 'action originalStep = step (imported original module)',
                'coverage': coverage, 'strings': sorted(strings),
                'domainConstruction': 'exact-key maps; independent arbitrary primitive records/scalars; filter by identical domainFor and strongFor'}
    return path, harness, metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('model', choices=['client_consent', 'client_contribution'])
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--seconds', type=int, default=600)
    parser.add_argument('--port', type=int, default=8851)
    parser.add_argument('--generate-only', action='store_true')
    args = parser.parse_args()
    if args.out.exists():
        parser.error('Use a fresh evidence directory')
    args.out.mkdir(parents=True)
    model, harness, metadata = generate(args.model)
    (args.out / 'domain-coverage.json').write_text(json.dumps(metadata, indent=2) + '\n')
    subprocess.run(['quint', 'typecheck', str(harness)], check=True, timeout=90)
    if args.generate_only:
        return
    phases = [
        ('initiation', 'originalInit', 'strengthened', 0),
        ('consecution', 'arbitraryInit', 'strengthened', 1),
        ('implication', 'arbitraryInit', 'implication', 0),
        ('nonvacuity', 'arbitraryInit', 'false', 0),
    ]
    results = []
    env = dict(os.environ)
    env['PATH'] = '/opt/homebrew/opt/openjdk@21/bin:' + env['PATH']
    env['JDK_JAVA_OPTIONS'] = '-Xmx3g'
    for phase, init, invariant, steps in phases:
        folder = args.out / phase
        folder.mkdir()
        command = ['quint', 'verify', str(harness), '--main', harness.stem,
                   '--init', init, '--step', 'originalStep', '--invariant', invariant,
                   '--max-steps', str(steps), '--server-endpoint', 'localhost:' + str(args.port),
                   '--verbosity', '3']
        before = (sha(model), sha(harness))
        started = time.monotonic()
        timed_out = False
        with (folder / 'check.log').open('w') as log:
            # Preserve terminal verdicts and the actual counterexample in stdout.
            process = subprocess.Popen(command, cwd=folder, env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
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
        after = (sha(model), sha(harness))
        changed = before != after
        log = (folder / 'check.log').read_text()
        status = classify_obligation(log, code, timed_out=timed_out, artifacts_changed=changed)
        result = {'phase': phase, 'status': status, 'command': command,
                  'modelSha256': before[0], 'harnessSha256': before[1],
                  'exitCode': code, 'timedOut': timed_out, 'artifactsChanged': changed,
                  'logPath': str(folder / 'check.log'), 'logSha256': sha(folder / 'check.log'),
                  'cwd': str(folder), 'budgetSeconds': args.seconds,
                  'environment': {'PATHPrefix': '/opt/homebrew/opt/openjdk@21/bin', 'JDK_JAVA_OPTIONS': '-Xmx3g', 'serverEndpoint': 'localhost:' + str(args.port)},
                  'seconds': round(time.monotonic() - started, 3)}
        (folder / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
        results.append(result)
        print(args.model + ' ' + phase + ': ' + status, flush=True)
    summary = {'model': args.model, 'recordedAt': datetime.now(timezone.utc).isoformat(),
               'status': certificate_status(results), 'phases': results,
               'meaning': 'Induction over the entire finite model step; source refinement reviewed separately.'}
    (args.out / 'certificate.json').write_text(json.dumps(summary, indent=2) + '\n')
    print('certificate: ' + summary['status'], flush=True)


if __name__ == '__main__':
    main()

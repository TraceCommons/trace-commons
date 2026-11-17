#!/usr/bin/env python3
"""Run pinned bounded checks with exclusive solver, coherent hashes and fail-closed results."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
CLASSIFIER = ROOT / 'docs/flows/trace-commons/scripts/induction_checks.py'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fingerprint():
    paths = list(HERE.glob('*.qnt')) + [HERE / 'checks.json', HERE / 'check.py', CLASSIFIER]
    for base in ['crates/trace-commons-ingest/src', 'crates/trace-commons-server/src',
                 'crates/trace-commons-protocol/src', 'migrations']:
        paths += [p for p in (ROOT / base).rglob('*') if p.is_file() and p.suffix in {'.rs', '.sql'}]
    return {str(p.relative_to(ROOT)): sha(p) for p in sorted(set(paths))}


def stop(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def classify(client, server, code, timed_out, changed):
    spec = importlib.util.spec_from_file_location('induction_checks', CLASSIFIER)
    classifier = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(classifier)
    result = classifier.classify_obligation(client + '\n' + server, code,
        timed_out=timed_out, artifacts_changed=changed)
    return f'bounded-no-counterexample' if result == 'proved' else result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path, help='Fresh directory outside repository')
    parser.add_argument('--port', type=int, default=8871, help='Exclusive local solver port')
    parser.add_argument('--java', default='/opt/homebrew/opt/openjdk@21/bin/java')
    parser.add_argument('--jar', type=Path, default=Path.home() / '.quint/apalache-dist-0.62.1/apalache/lib/apalache.jar')
    parser.add_argument('--seconds', type=int, default=180)
    args = parser.parse_args()
    out = args.out.resolve()
    if out.exists() or out == ROOT or ROOT in out.parents or not 1024 <= args.port <= 65535 or args.seconds < 1:
        parser.error('Use a fresh output directory outside repository, unprivileged port and positive budget')
    # Never reuse somebody else's solver or attribute their logs to this run.
    with socket.socket() as probe:
        try:
            probe.bind(('127.0.0.1', args.port))
        except OSError:
            parser.error('Solver port already in use')
    cases = json.loads((HERE / 'checks.json').read_text())['cases']
    if not cases or len({c['name'] for c in cases}) != len(cases):
        parser.error('Need nonempty uniquely named checks')
    out.mkdir()
    before = fingerprint()
    server_log = out / 'server.log'
    command = [args.java, '-Xmx3g', '-jar', str(args.jar), 'server', f'--port={args.port}']
    results = []
    with server_log.open('w') as stream:
        server = subprocess.Popen(command, cwd=out, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            deadline = time.monotonic() + 30
            while 'The Apalache server is running' not in server_log.read_text():
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('Solver did not start; inspect server.log')
                time.sleep(.1)
            for case in cases:
                folder = out / case['name']
                folder.mkdir()
                model = HERE / case['model']
                if model.parent != HERE or not model.is_file() or case['steps'] < 0:
                    raise ValueError('Invalid model or bound')
                offset = server_log.stat().st_size
                command = ['quint', 'verify', str(model), '--main', model.stem.replace('-', '_'),
                    '--init', case.get('init', 'init'), '--step', case.get('step', 'step'),
                    '--invariant', case['invariant'], '--max-steps', str(case['steps']),
                    '--server-endpoint', f'localhost:{args.port}', '--verbosity', '2',
                    '--out-itf', str(folder / 'counterexample.itf.json')]
                started = time.monotonic()
                timed_out = False
                with (folder / 'client.log').open('w') as log:
                    process = subprocess.Popen(command, cwd=folder, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    try:
                        code = process.wait(timeout=args.seconds)
                    except subprocess.TimeoutExpired:
                        timed_out = True
                        stop(process)
                        code = process.returncode
                client = (folder / 'client.log').read_text()
                with server_log.open('rb') as log:
                    log.seek(offset)
                    solver = log.read().decode()
                (folder / 'solver.log').write_text(solver)
                changed = fingerprint() != before
                status = classify(client, solver, code, timed_out, changed)
                trace = folder / 'counterexample.itf.json'
                if status == 'counterexample' and (not trace.is_file() or not json.loads(trace.read_text()).get('states')):
                    status = 'incomplete'
                record = {**case, 'status': status, 'exitCode': code, 'command': command,
                    'timedOut': timed_out, 'artifactsChanged': changed,
                    'elapsedSeconds': round(time.monotonic() - started, 3),
                    'modelSha256': sha(model), 'clientLogSha256': sha(folder / 'client.log'),
                    'solverLogSha256': sha(folder / 'solver.log')}
                (folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
                results.append(record)
                print(f'{case["name"]}: {status} (expected {case["expect"]})', flush=True)
                if timed_out or changed or status == 'incomplete':
                    break  # timed-out solver is not safe to reuse
        finally:
            stop(server)
    final = {'scope': 'bounded symbolic search, not unbounded induction or executable equivalence',
        'baseline': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'quintVersion': subprocess.check_output(['quint', '--version'], text=True).strip(),
        'solverJarSha256': sha(args.jar), 'sourceHashes': before, 'cases': results,
        'complete': len(results) == len(cases) and all(c['status'] == c['expect'] for c in results)
            and fingerprint() == before}
    (out / 'results.json').write_text(json.dumps(final, indent=2) + '\n')
    raise SystemExit(0 if final['complete'] else 1)


if __name__ == '__main__':
    main()

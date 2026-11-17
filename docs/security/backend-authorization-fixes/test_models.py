#!/usr/bin/env python3
"""Typecheck every audit model and execute every named run (never accept zero tests)."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    if args.out.exists():
        parser.error('Use a fresh result file')
    records = []
    with tempfile.TemporaryDirectory(prefix='tc-backend-qnt-witnesses-') as cwd:
        for model in sorted(HERE.glob('*.qnt')):
            before = hashlib.sha256(model.read_bytes()).hexdigest()
            names = re.findall(r'^\s*run\s+(\w+)\s*=', model.read_text(), re.M)
            if not names or len(names) != len(set(names)):
                raise ValueError(f'{model}: need nonempty uniquely named witness/control runs')
            commands = [['quint', 'typecheck', str(model)],
                ['quint', 'test', str(model), '--max-samples', '1', '--match', '^(' + '|'.join(names) + ')$']]
            logs = []
            for command in commands:
                result = subprocess.run(command, cwd=cwd, text=True, capture_output=True, timeout=120)
                logs.append({'command': command, 'exitCode': result.returncode,
                             'output': result.stdout + result.stderr})
            passed = re.findall(r'\bok (\w+) passed 1 test\(s\)', logs[1]['output'])
            stable = before == hashlib.sha256(model.read_bytes()).hexdigest()
            ok = stable and all(log['exitCode'] == 0 for log in logs) and sorted(passed) == sorted(names)
            records.append({'model': model.name, 'sha256': before, 'runs': names, 'ok': ok, 'checks': logs})
            print(f'{model.name}: {len(passed)}/{len(names)} named runs; typecheck/test {"passed" if ok else "FAILED"}', flush=True)
    complete = bool(records) and all(record['ok'] for record in records)
    args.out.write_text(json.dumps({'complete': complete, 'models': records}, indent=2) + '\n')
    raise SystemExit(0 if complete else 1)


if __name__ == '__main__':
    main()

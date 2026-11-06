#!/usr/bin/env python3
"""Run the pinned flow-promises checker after explicit product promise selection."""
import argparse
import json
import subprocess
from pathlib import Path

FLOW = Path(__file__).resolve().parent.parent
PIN = '6f35f6ae6b3581c499bdfcae080932b4ffa7e281'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('model', help='Model basename')
    parser.add_argument('--skill-root', required=True, type=Path,
                        help='Pinned bkase/plugins checkout')
    parser.add_argument('--out', required=True, type=Path, help='New TLC output directory')
    args = parser.parse_args()
    if args.model != Path(args.model).name or not (FLOW / 'models' / f'{args.model}.qnt').is_file():
        parser.error('Unknown model basename')
    selection = json.loads((FLOW / 'promise-selection.json').read_text())
    selected = [p for p in selection['promises'] if p.get('model') == args.model and p.get('selected') is True]
    if selection['status'] != 'selected' or not selected or not selection.get('ownerSelection'):
        parser.error('No owner-selected promises recorded for this model; show the surface and request selection first')
    commit = subprocess.check_output(['git', '-C', str(args.skill_root), 'rev-parse', 'HEAD'], text=True).strip()
    if commit != PIN:
        parser.error(f'Skill checkout must be pinned at {PIN}')
    cfg = FLOW / 'models' / f'{args.model}.cfg'
    import re
    identifiers = []
    for line in cfg.read_text().splitlines():
        identifiers.extend(re.findall(r'^\s*(?:INVARIANT|PROPERTY)\s+(\w+)\s*$', line))
    if set(identifiers) != {p['definition'] for p in selected}:
        parser.error('Config checks must match the selected promise definitions exactly')
    if args.out.exists():
        parser.error('Use a new output directory')
    script = args.skill_root / 'pstack/skills/flow-promises/scripts/check'
    command = [str(script), str(FLOW / 'models' / f'{args.model}.qnt'), '--config', str(cfg),
               '--view', str(FLOW / 'models' / f'{args.model}.view.mts'), '--out', str(args.out.resolve())]
    raise SystemExit(subprocess.run(command).returncode)


if __name__ == '__main__':
    main()

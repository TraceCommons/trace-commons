"""Selection and report validation for the pinned checker, without verdict guessing."""
import json
import re


def selected_for(flow, model):
    selection = json.loads((flow / 'promise-selection.json').read_text())
    if selection.get('status') != 'selected' or not selection.get('ownerSelection'):
        raise ValueError('Owner selection is required before checking promises')
    chosen = [p for p in selection['promises']
              if p.get('selected') is True and p.get('model') == model]
    cfg = (flow / 'models' / f'{model}.cfg').read_text()
    if re.search(r'(?m)^\s*(?:CONSTRAINT|ACTION_CONSTRAINT)\b', cfg):
        raise ValueError('Exploration constraints are not allowed in the checked model')
    configured = re.findall(r'(?m)^\s*(?:INVARIANT|PROPERTY)\s+(\w+)\s*$', cfg)
    if len(re.findall(r'(?m)^\s*(?:INVARIANT|PROPERTY)\b', cfg)) != len(configured):
        raise ValueError('Checks must each name exactly one selected definition')
    names = [p['definition'] for p in chosen]
    if len(names) != len(set(names)) or len(configured) != len(set(configured)) or set(configured) != set(names):
        raise ValueError(f'{model}: configured checks must exactly match owner selection')
    return chosen


def report_results(report, definitions, *, timed_out=False, exit_code=None):
    """Only an explicit pinned-checker verdict may become holds or breaks."""
    breaks = set(re.findall(r'(?m)^## (\w+) breaks\s*$', report))
    hold_section = re.search(r'(?m)^## Rules that hold[ \t]*\n([A-Za-z0-9_, \t]+)$', report)
    holds = set(re.findall(r'\b\w+\b', hold_section.group(1))) if hold_section else set()
    results = []
    for name in definitions:
        if name in breaks:
            status = 'breaks'
        elif name in holds and not timed_out and exit_code in (0, 1):
            status = 'holds'
        else:
            status = 'not checked'
        results.append({'definition': name, 'status': status})
    return results

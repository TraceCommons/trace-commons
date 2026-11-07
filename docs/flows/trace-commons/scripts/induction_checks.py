"""Classify proof obligations, not application correctness or source equivalence.

A complete certificate additionally needs reviewed domain coverage and linkage
of its initializer, original full step and selected conclusion. These helpers
check tool completion and source-snapshot coherence; they cannot infer those
semantic obligations from a successful log.
"""
import re


def classify_obligation(log, exit_code, *, timed_out=False, artifacts_changed=False):
    if timed_out or artifacts_changed:
        return 'incomplete'
    clean = re.sub(r'\x1b\[[0-9;]*m', '', log)
    unexpected = re.sub(r'(?mi)^error: found a counterexample[ \t]*$', '', clean)
    if re.search(r'(?mi)(?:\[failure\]|^error:|\bE@\d|unsupported expression|\b\w*Exception\b)', unexpected):
        return 'incomplete'
    if re.search(r'\[violation\]\s+Found an issue', clean) and re.search(r'counterexample', clean, re.I):
        return 'counterexample' if exit_code == 1 else 'incomplete'
    # Quint may exit zero after an unsupported Apalache expression. A successful
    # process alone, or a partially checked invariant, is not a proof result.
    if exit_code != 0 or re.search(r'(?mi)(?:\[failure\]|^error:|\bE@\d|unsupported expression|\b\w*Exception\b)', clean):
        return 'incomplete'
    outcomes = re.findall(r'The outcome is:\s*(\w+)', clean)
    if outcomes and outcomes[-1] == 'NoError' and re.search(r'\[ok\]\s+No violation found', clean):
        return 'proved'
    return 'incomplete'


def certificate_status(phases):
    required = {'initiation': 'proved', 'consecution': 'proved',
                'implication': 'proved', 'nonvacuity': 'counterexample'}
    if len(phases) != len(required) or {p.get('phase') for p in phases} != set(required):
        return 'incomplete'
    if any(p.get('status') != required[p['phase']] for p in phases):
        return 'incomplete'
    for key in ['modelSha256', 'harnessSha256']:
        hashes = {p.get(key) for p in phases}
        if len(hashes) != 1 or not all(hashes):
            return 'incomplete'
    return 'proved'

#!/usr/bin/env python3
"""Replay extracted Rust config capture helpers with synthetic data, no network."""
from pathlib import Path
import hashlib
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
source = (ROOT / 'crates/trace-commons-contributor/src/submit.rs').read_text()
mod = (ROOT / 'crates/trace-commons-contributor/src/daemon/mod.rs').read_text()


def extract(start):
    begin = source.index(start)
    brace = source.index('{', begin)
    depth = 0
    for i in range(brace, len(source)):
        if source[i] == '{':
            depth += 1
        elif source[i] == '}':
            depth -= 1
            if depth == 0:
                return source[begin:i + 1]
    raise ValueError('Unclosed source method')


effective = extract('fn effective_config(')
getter = extract('    pub fn effective_cfg(')
# These actual source expressions establish the handoff from pass snapshot to
# pipeline context, the claim check, and the context's envelope config copy.
for expression in [
    'let Some(cfg) = shared.store.load_config()?',
    'crate::submit::SubmitContext::new(&store, &cfg, &opts, near_ai)',
    'current.approved_scopes.as_deref() != Some(cfg.consent_scopes.as_slice())',
]:
    assert expression in mod, 'Source handoff changed: ' + expression
assert 'let effective_cfg = effective_config(cfg, opts);' in source
for label, body in [('submit.rs', source), ('daemon/mod.rs', mod), ('effective_config', effective), ('effective_cfg', getter)]:
    print(label + ' SHA256:', hashlib.sha256(body.encode()).hexdigest(), flush=True)

harness = '''#[derive(Clone)]
struct ContributorConfig { consent_scopes: Vec<String>, pii_filter: Option<String> }
struct SubmitOptions { unenrolled_preview: bool, pii_filter: Option<String> }
struct SubmitContext { effective_cfg: ContributorConfig }
impl SubmitContext {
GETTER
}
EFFECTIVE
fn main() {
 let mut saved = ContributorConfig { consent_scopes: vec!["approved-old-scope".into()], pii_filter: None };
 let approved_scopes = Some(saved.consent_scopes.clone());
 // ConfigStore::load_config returns an owned config; the source passes its
 // reference to SubmitContext::new before the claim. No live-config alias.
 let cfg = saved.clone();
 let opts = SubmitOptions { unenrolled_preview: false, pii_filter: None };
 let ctx = SubmitContext { effective_cfg: effective_config(&cfg, &opts) };
 // Exact source claim predicate, replacing only current.approved_scopes.
 assert!(!(approved_scopes.as_deref() != Some(cfg.consent_scopes.as_slice())));
 saved.consent_scopes = vec!["newly-saved-scope".into()];
 assert_ne!(saved.consent_scopes, cfg.consent_scopes);
 assert_eq!(approved_scopes.as_deref(), Some(ctx.effective_cfg().consent_scopes.as_slice()));
 println!("saved config changed after claim: {:?}", saved.consent_scopes);
 println!("captured send context retains approved scopes: {:?}", ctx.effective_cfg().consent_scopes);
 // Negative control: next pass captures the changed config, so the old
 // approval no longer matches the exact claim predicate.
 let cfg = saved.clone();
 assert!(approved_scopes.as_deref() != Some(cfg.consent_scopes.as_slice()));
 println!("next pass: old approval mismatches new captured config");
}
'''.replace('GETTER', getter).replace('EFFECTIVE', effective)
with tempfile.TemporaryDirectory(prefix='tc-scopes-source-replay-', dir='/private/tmp') as tmp:
    tmp = Path(tmp)
    rust = tmp / 'scopes_source_replay.rs'
    binary = tmp / 'scopes_source_replay'
    rust.write_text(harness)
    subprocess.run(['rustc', '--edition=2024', '-D', 'warnings', str(rust), '-o', str(binary)], check=True, timeout=60)
    subprocess.run([str(binary)], check=True, timeout=10)

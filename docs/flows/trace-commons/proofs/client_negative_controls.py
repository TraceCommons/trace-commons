#!/usr/bin/env python3
"""Focused regression and deliberate fault-injection controls for P1-P4/P12.

Copies current models into temporary test modules. Fault fixtures/mutants never
change ordinary model files and are not represented as reachable product bugs.
"""
from pathlib import Path
import hashlib
import subprocess
import tempfile

FLOW = Path(__file__).resolve().parent.parent


def run(model, extra, label, transform=lambda text: text):
    path = FLOW / 'models' / (model + '.qnt')
    source = path.read_text()
    print(label + ' model SHA256: ' + hashlib.sha256(source.encode()).hexdigest(), flush=True)
    with tempfile.TemporaryDirectory(prefix='tc-client-proof-control-', dir='/private/tmp') as folder:
        file = Path(folder) / (model + '.qnt')
        file.write_text(transform(source).rsplit('}', 1)[0] + extra + '\n}\n')
        subprocess.run(['quint', 'test', str(file), '--main', model, '--match', 'control', '--max-samples', '1', '--backend', 'typescript'], check=True, timeout=60)


consent_controls = '''
  action priorCompletedGrant = s' = { ...s, screen: "consent_internal", grant: true, includeExisting: true, displayedSharing: "automatic" }
  run ask_reply_order_control = init.then(priorCompletedGrant)
    .then(TheSharingChoiceRequestsTheSavedGrant).then(YouWithdrawTheGrant)
    .then(TheGrantWithdrawalIsSaved).then(TheGrantWithdrawalReplyArrives)
    .then(TheSharingChoiceRequestsTheSavedGrant).then(TheSharingChoiceReceivesTheSavedGrant)
    .then(TheSharingChoiceDiscardsAStaleGrantRead)
    .then(all { assert(P1 and s.displayedSharing == "ask"), s' = s })
  // Fault fixture: an Automatic snapshot without completed grant evidence.
  action injectUncompletedGrantSnapshot = s' = { ...s, sharingRead: "automatic", sharingReadCurrent: true, sharingGrantCompleted: false }
  run uncompleted_grant_control = init.then(injectUncompletedGrantSnapshot)
    .then(TheSharingChoiceReceivesTheSavedGrant)
    .then(all { assert(not(P1)), s' = s })
'''
run('client_consent', consent_controls, 'P1 current Ask ordering + incomplete-grant detector')

stale_consent = '''
  action injectOldGrantCallback = s' = { ...s, sharingRead: "automatic", sharingReadCurrent: false, sharingGrantCompleted: true }
  run stale_grant_control = init.then(injectOldGrantCallback)
    .then(TheSharingChoiceReceivesTheSavedGrant)
    .then(all { assert(not(P1)), s' = s })
'''
def remove_grant_epoch_guard(source):
    old = 's.sharingRead != "none" and s.sharingReadCurrent and not(s.grantMutation)'
    assert old in source
    return source.replace(old, 's.sharingRead != "none" and not(s.grantMutation)', 1)
run('client_consent', stale_consent, 'P1 guard-regression detector', remove_grant_epoch_guard)

contribution_fixture = '''
  action preparedSend(scope: int, enrolled: bool, stage: str): bool = s' = { ...s,
    enrolled: enrolled, upload: stage, uploadEntry: 0, uploadScopes: scope,
    entries: s.entries.set(0, { ...s.entries.get(0), state: "uploading", approvedScopes: 0, approvedInputs: 0 }) }
'''
positive_controls = contribution_fixture + '''
  run approved_scope_control = init.then(preparedSend(0, true, "checking"))
    .then(TheUploadPassesItsPreconditions).then(all { assert(P2 and s.sentScopes == 0), s' = s })
  run unapproved_scope_control = init.then(preparedSend(1, true, "checking"))
    .then(TheUploadPassesItsPreconditions).then(all { assert(not(P2) and s.sentScopes == 1), s' = s })
  run capacity_retains_approval_control = init.then(preparedSend(0, true, "checking"))
    .then(TheWitnessIsAtCapacity).then(all { assert(P3 and s.entries.get(0).state == "approved" and s.entries.get(0).attempts == 0), s' = s })
  run remote_rate_limit_retains_approval_control = init.then(preparedSend(0, true, "sending"))
    .then(TheIngestRateLimitDefersTheSend).then(all { assert(P3 and s.entries.get(0).state == "approved" and s.entries.get(0).attempts == 0), s' = s })
  run removed_enrollment_control = init.then(preparedSend(0, false, "checking"))
    .then(TheUploaderRefusesMissingEnrollment).then(all { assert(P4 and s.upload == "none"), s' = s })
  run account_aba_stale_callback_control = init
    .then(TheBackgroundHistoryRefreshBegins).then(TheCurrentAccountChanges)
    .then(TheBackgroundHistoryRefreshBegins).then(TheCurrentAccountChanges)
    .then(TheBackgroundHistoryRefreshBegins).then(TheHistoryReadDiscardsAStaleSnapshot)
    .then(all { assert(P12 and s.account == 1 and s.historyPending.get(1)), s' = s })
'''
run('client_contribution', positive_controls, 'P2/P3/P4/P12 positive controls + scope fault detector')

capacity_fault = contribution_fixture + '''
  run lost_approval_control = init.then(preparedSend(0, true, "checking"))
    .then(TheWitnessIsAtCapacity).then(all { assert(not(P3)), s' = s })
'''
def lose_approval(source):
    old = 'val after = { ...before, state: "approved", retry: true }'
    assert old in source
    return source.replace(old, 'val after = { ...before, state: "failed", retry: true }', 1)
run('client_contribution', capacity_fault, 'P3 lost-approval detector', lose_approval)

logout_fault = contribution_fixture + '''
  run removed_enrollment_send_control = init.then(preparedSend(0, false, "checking"))
    .then(TheUploadPassesItsPreconditions).then(all { assert(not(P4)), s' = s })
'''
def bypass_enrollment(source):
    old = 's.upload == "checking" and s.enrolled and s.entries.get(s.uploadEntry).sourceRev'
    assert old in source
    return source.replace(old, 's.upload == "checking" and s.entries.get(s.uploadEntry).sourceRev', 1)
run('client_contribution', logout_fault, 'P4 pre-send authority detector', bypass_enrollment)

history_fault = '''
  action injectOldCallback = s' = { ...s, account: 2, historyStale: Map(1 -> true, 2 -> false) }
  run old_account_publication_control = init.then(injectOldCallback)
    .then(TheHistoryReadDiscardsAStaleSnapshot).then(all { assert(not(P12)), s' = s })
'''
def publish_stale(source):
    old = "s' = { ...s, historyStale: s.historyStale.set(owner, morePending) }"
    assert old in source
    return source.replace(old, "s' = { ...s, historyStale: s.historyStale.set(owner, morePending), displayedHistoryOwner: owner, proof: { ...s.proof, P12: s.proof.P12 and owner == s.account } }", 1)
run('client_contribution', history_fault, 'P12 stale-publication detector', publish_stale)

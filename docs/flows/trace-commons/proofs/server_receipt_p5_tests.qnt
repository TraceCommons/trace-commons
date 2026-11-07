module server_receipt_p5_tests {
  import server_receipt.* from "../models/server_receipt"

  // Synthetic committed-row setup exists only in this observer test module.
  action seedReceipt = all {
    s' = {...s, receipts: s.receipts.set(0, {...emptyReceipt, present: true, bytes: 1, sourceBytes: 3,
      principal: 0, bundle: 1, submission: 1, admission: "quarantine"})},
    proof' = proof
  }
  action retryResult(body: int, result: str, returned: Receipt): bool = {
    val nextState = {...s, requests: s.requests.set(0, {...emptyRequest, bytes: body,
      submission: 1, phase: "done", completedRetry: true, result: result, returnedReceipt: returned}),
      outcome: result}
    all {s' = nextState, proof' = observe(s, nextState, proof)}
  }
  run sameBodyReturnsSavedResultTest = init.then(seedReceipt)
    .then(retryResult(1, "replayed", s.receipts.get(0))).expect(P5)
  run changedBodyIsRefusedTest = init.then(seedReceipt)
    .then(retryResult(0, "content_conflict", emptyReceipt)).expect(P5)
  run incorrectRefusalIsDetectedTest = init.then(seedReceipt)
    .then(retryResult(1, "owner_refused", emptyReceipt)).expect(not(P5))
  run incorrectSavedResultIsDetectedTest = init.then(seedReceipt)
    .then(retryResult(1, "replayed", {...s.receipts.get(0), admission: "admit"})).expect(not(P5))
  run silentContentConflictIsDetectedTest = init.then(seedReceipt)
    .then(retryResult(0, "created", emptyReceipt)).expect(not(P5))
}

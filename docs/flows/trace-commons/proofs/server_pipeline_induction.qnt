module server_pipeline_induction {
  import server_pipeline.* from "../models/server_pipeline"

  pure def stateStrengthening(state: State): bool =
    state.runs.keys() == Runs and state.workers.keys() == Workers
    and state.humanRequests.keys() == Reviewers and state.artifacts.keys() == 0.to(7)
    and state.runnable.keys() == 0.to(11)
    and Runs.forall(id => {
      val r = state.runs.get(id)
      Set(0, 1).contains(r.tenant) and Set(0, 1).contains(r.bundle)
        and 0.to(2).contains(r.attempt) and 0.to(2).contains(r.renewals)
        and 0.to(16).contains(r.token) and 0.to(4).contains(r.reviewToken)
        and Workers.contains(r.worker) and Reviewers.contains(r.reviewOwner)
        and (r.state != "absent" or r == emptyRun)
        and (r.state != "leased" or r.attempt >= 1)
    })
    and Workers.forall(worker => {
      val w = state.workers.get(worker)
      Runs.contains(w.runId) and 0.to(16).contains(w.token)
    })
    and Reviewers.forall(reviewer => {
      val h = state.humanRequests.get(reviewer)
      Runs.contains(h.runId) and 0.to(4).contains(h.token)
    })
    and 0.to(7).forall(id => {
      val a = state.artifacts.get(id)
      Runs.contains(a.runId) and 0.to(16).contains(a.token)
    })
  val Inductive = stateStrengthening(s) and P6 and P7 and P8

  // Proof-only arbitrary state: each original key is present exactly once.
  // Primitive records have no nested collections; generate(1) covers all
  // their scalar values. Collection choices use the original complete domains.
  action arbitraryInit = {
    nondet runs0: Run = apalache::generate(1)
    nondet runs1: Run = apalache::generate(1)
    nondet workers0: Work = apalache::generate(1)
    nondet workers1: Work = apalache::generate(1)
    nondet humanRequests0: HumanWork = apalache::generate(1)
    nondet humanRequests1: HumanWork = apalache::generate(1)
    nondet artifacts0: Artifact = apalache::generate(1)
    nondet artifacts1: Artifact = apalache::generate(1)
    nondet artifacts2: Artifact = apalache::generate(1)
    nondet artifacts3: Artifact = apalache::generate(1)
    nondet artifacts4: Artifact = apalache::generate(1)
    nondet artifacts5: Artifact = apalache::generate(1)
    nondet artifacts6: Artifact = apalache::generate(1)
    nondet artifacts7: Artifact = apalache::generate(1)
    nondet runnable0: bool = Set(false, true).oneOf()
    nondet runnable1: bool = Set(false, true).oneOf()
    nondet runnable2: bool = Set(false, true).oneOf()
    nondet runnable3: bool = Set(false, true).oneOf()
    nondet runnable4: bool = Set(false, true).oneOf()
    nondet runnable5: bool = Set(false, true).oneOf()
    nondet runnable6: bool = Set(false, true).oneOf()
    nondet runnable7: bool = Set(false, true).oneOf()
    nondet runnable8: bool = Set(false, true).oneOf()
    nondet runnable9: bool = Set(false, true).oneOf()
    nondet runnable10: bool = Set(false, true).oneOf()
    nondet runnable11: bool = Set(false, true).oneOf()
    nondet dependencyValue: bool = Set(false, true).oneOf()
    nondet privacyControlValue: bool = Set(false, true).oneOf()
    nondet frameValue: str = apalache::generate(1)
    nondet eventValue: str = apalache::generate(1)
    val state: State = {
      runs: Map(0 -> runs0, 1 -> runs1),
      workers: Map(0 -> workers0, 1 -> workers1),
      humanRequests: Map(0 -> humanRequests0, 1 -> humanRequests1),
      artifacts: Map(0 -> artifacts0, 1 -> artifacts1, 2 -> artifacts2, 3 -> artifacts3, 4 -> artifacts4, 5 -> artifacts5, 6 -> artifacts6, 7 -> artifacts7),
      runnable: Map(0 -> runnable0, 1 -> runnable1, 2 -> runnable2, 3 -> runnable3, 4 -> runnable4, 5 -> runnable5, 6 -> runnable6, 7 -> runnable7, 8 -> runnable8, 9 -> runnable9, 10 -> runnable10, 11 -> runnable11),
      dependency: dependencyValue,
      privacyControl: privacyControlValue,
      frame: frameValue,
      event: eventValue
    }
    all { stateStrengthening(state), s' = state, proof' = {P6: true, P7: true, P8: true} }
  }
  action originalInit = init
  action originalStep = step
  val Conclusion = P6 and P7 and P8
}

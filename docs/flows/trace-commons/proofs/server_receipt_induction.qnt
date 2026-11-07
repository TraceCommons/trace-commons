module server_receipt_induction {
  import server_receipt.* from "../models/server_receipt"

  pure def requestDomain(r: Request): bool = Tenants.contains(r.tenant) and Principals.contains(r.principal)
    and Keys.contains(r.key) and Set(0, 1).contains(r.bytes) and Set(0, 1).contains(r.submission)
    and Set(0, 1).contains(r.bundle)
  pure def stateStrengthening(state: State): bool =
    state.requests.keys() == Attempts and state.receipts.keys() == 0.to(5)
    and state.owner.keys() == 0.to(5) and state.routing.keys() == Tenants
    and state.activeBundle.keys() == Tenants and state.runnable.keys() == 0.to(5)
    and state.controlsPresent.keys() == Tenants and state.usagePrincipal.keys() == 0.to(5)
    and state.usage.subseteq(0.to(5)) and state.sourceWithdrawn.subseteq(0.to(5)) and state.tombstones.subseteq(0.to(5))
    and 0.to(5).forall(cell => Set(0, 1).contains(state.receipts.get(cell).submission))
    and Tenants.forall(t => Set(0, 1).contains(state.activeBundle.get(t)))
    and Attempts.forall(a => {
      val r = state.requests.get(a)
      requestDomain(r) and (r.phase != "unused" or r == emptyRequest)
        and (not(Set("prechecked", "prepared", "staged", "written", "admitted", "committed", "responded", "response_lost").contains(r.phase)) or r.validatedControls)
    })
  val Inductive = stateStrengthening(s) and P5 and P8 and P11

  // Proof-only arbitrary state: each original key is present exactly once.
  // Primitive records have no nested collections; generate(1) covers all
  // their scalar values. Collection choices use the original complete domains.
  action arbitraryInit = {
    nondet requests0: Request = apalache::generate(1)
    nondet requests1: Request = apalache::generate(1)
    nondet requests2: Request = apalache::generate(1)
    nondet requests3: Request = apalache::generate(1)
    nondet receipts0: Receipt = apalache::generate(1)
    nondet receipts1: Receipt = apalache::generate(1)
    nondet receipts2: Receipt = apalache::generate(1)
    nondet receipts3: Receipt = apalache::generate(1)
    nondet receipts4: Receipt = apalache::generate(1)
    nondet receipts5: Receipt = apalache::generate(1)
    nondet owner0: str = apalache::generate(1)
    nondet owner1: str = apalache::generate(1)
    nondet owner2: str = apalache::generate(1)
    nondet owner3: str = apalache::generate(1)
    nondet owner4: str = apalache::generate(1)
    nondet owner5: str = apalache::generate(1)
    nondet routing0: str = apalache::generate(1)
    nondet routing1: str = apalache::generate(1)
    nondet activeBundle0: int = apalache::generate(1)
    nondet activeBundle1: int = apalache::generate(1)
    nondet runnable0: bool = Set(false, true).oneOf()
    nondet runnable1: bool = Set(false, true).oneOf()
    nondet runnable2: bool = Set(false, true).oneOf()
    nondet runnable3: bool = Set(false, true).oneOf()
    nondet runnable4: bool = Set(false, true).oneOf()
    nondet runnable5: bool = Set(false, true).oneOf()
    nondet controlsPresent0: bool = Set(false, true).oneOf()
    nondet controlsPresent1: bool = Set(false, true).oneOf()
    nondet usagePrincipal0: int = apalache::generate(1)
    nondet usagePrincipal1: int = apalache::generate(1)
    nondet usagePrincipal2: int = apalache::generate(1)
    nondet usagePrincipal3: int = apalache::generate(1)
    nondet usagePrincipal4: int = apalache::generate(1)
    nondet usagePrincipal5: int = apalache::generate(1)
    nondet frameValue: str = apalache::generate(1)
    nondet outcomeValue: str = apalache::generate(1)
    nondet usageValue = 0.to(5).powerset().oneOf()
    nondet sourceWithdrawnValue = 0.to(5).powerset().oneOf()
    nondet tombstonesValue = 0.to(5).powerset().oneOf()
    val state: State = {
      requests: Map(0 -> requests0, 1 -> requests1, 2 -> requests2, 3 -> requests3),
      receipts: Map(0 -> receipts0, 1 -> receipts1, 2 -> receipts2, 3 -> receipts3, 4 -> receipts4, 5 -> receipts5),
      owner: Map(0 -> owner0, 1 -> owner1, 2 -> owner2, 3 -> owner3, 4 -> owner4, 5 -> owner5),
      routing: Map(0 -> routing0, 1 -> routing1),
      activeBundle: Map(0 -> activeBundle0, 1 -> activeBundle1),
      runnable: Map(0 -> runnable0, 1 -> runnable1, 2 -> runnable2, 3 -> runnable3, 4 -> runnable4, 5 -> runnable5),
      controlsPresent: Map(0 -> controlsPresent0, 1 -> controlsPresent1),
      usagePrincipal: Map(0 -> usagePrincipal0, 1 -> usagePrincipal1, 2 -> usagePrincipal2, 3 -> usagePrincipal3, 4 -> usagePrincipal4, 5 -> usagePrincipal5),
      frame: frameValue,
      outcome: outcomeValue,
      usage: usageValue,
      sourceWithdrawn: sourceWithdrawnValue,
      tombstones: tombstonesValue
    }
    all { stateStrengthening(state), s' = state, proof' = {P5: true, P8: true, P11: true} }
  }
  action originalInit = init
  action originalStep = step
  val Conclusion = P5 and P8 and P11
}

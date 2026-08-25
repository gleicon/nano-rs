-------------------------------- MODULE HotSwap --------------------------------
(***************************************************************************)
(* A TLA+ model of nano-rs's zero-downtime sliver hot-swap.                 *)
(*                                                                          *)
(* Code under test: `SliverPoolSlot` in src/worker/sliver_pool.rs.          *)
(*                                                                          *)
(*   - A request reads the slot (`current()`), which clones the Arc of the  *)
(*     pool currently installed, then dispatches to it and runs.            *)
(*   - `hotswap` builds a NEW pool, atomically replaces the slot, and        *)
(*     returns the OLD pool. `hotswap_and_drain` drops that old Arc after a  *)
(*     drain timeout.                                                        *)
(*   - A pool's worker threads exit only when its LAST Arc is dropped —      *)
(*     the drain timer dropping the slot's clone is NOT enough while a       *)
(*     request handler still holds its own clone.                           *)
(*                                                                          *)
(* We model each pool by a generation number and a lifecycle status:        *)
(*   absent -> live -> retired -> dead                                       *)
(* `live`   : installed in the slot, accepting new requests.                 *)
(* `retired`: replaced by a newer pool; existing requests still running.     *)
(* `dead`   : Arc fully dropped, worker threads have exited.                 *)
(*                                                                          *)
(* The constant HardKill selects which teardown rule we check:              *)
(*   FALSE = the SHIPPED design: reap only when no request still holds it.   *)
(*   TRUE  = a BUGGY variant that hard-kills on the drain timer, ignoring    *)
(*           in-flight requests. TLC finds a counterexample for this one.    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    MaxGen,        \* highest pool generation to explore (bounds hot-swaps)
    MaxInflight,   \* bound on concurrent in-flight requests (bounds state)
    HardKill       \* FALSE = shipped design; TRUE = buggy hard-kill variant

ASSUME MaxGen \in Nat /\ MaxInflight \in Nat /\ HardKill \in BOOLEAN

Gens == 0..MaxGen

VARIABLES
    slotGen,   \* generation currently installed in the slot
    status,    \* [Gens -> {"absent","live","retired","dead"}]
    drained,   \* subset of Gens whose drain timer has elapsed
    inflight   \* [Gens -> Nat] : # of in-flight requests bound to each pool

vars == <<slotGen, status, drained, inflight>>

TotalInflight == LET S[g \in Gens] == IF g = 0 THEN inflight[0]
                                       ELSE inflight[g] + S[g-1]
                 IN S[MaxGen]

TypeOK ==
    /\ slotGen \in Gens
    /\ status \in [Gens -> {"absent","live","retired","dead"}]
    /\ drained \subseteq Gens
    /\ inflight \in [Gens -> 0..MaxInflight]

Init ==
    /\ slotGen = 0
    /\ status = [g \in Gens |-> IF g = 0 THEN "live" ELSE "absent"]
    /\ drained = {}
    /\ inflight = [g \in Gens |-> 0]

(* A request arrives: `current()` reads the slot (always a live pool) and    *)
(* binds to it for the duration of the request.                              *)
Dispatch ==
    /\ status[slotGen] = "live"
    /\ TotalInflight < MaxInflight
    /\ inflight' = [inflight EXCEPT ![slotGen] = @ + 1]
    /\ UNCHANGED <<slotGen, status, drained>>

(* An in-flight request finishes and releases its Arc clone of pool g.       *)
Complete(g) ==
    /\ inflight[g] > 0
    /\ inflight' = [inflight EXCEPT ![g] = @ - 1]
    /\ UNCHANGED <<slotGen, status, drained>>

(* Deploy a new version: build gen g+1, install it, retire the old current.  *)
HotSwap ==
    /\ slotGen + 1 \in Gens
    /\ status[slotGen + 1] = "absent"
    /\ status' = [status EXCEPT ![slotGen + 1] = "live", ![slotGen] = "retired"]
    /\ slotGen' = slotGen + 1
    /\ UNCHANGED <<drained, inflight>>

(* The drain timeout elapses for a retired pool (the spawned drain task's     *)
(* sleep returns and it drops the slot's old Arc clone).                      *)
DrainElapse(g) ==
    /\ status[g] = "retired"
    /\ g \notin drained
    /\ drained' = drained \cup {g}
    /\ UNCHANGED <<slotGen, status, inflight>>

(* Workers actually exit (pool becomes dead).                                *)
(*  - Shipped design (HardKill=FALSE): only once the drain elapsed AND no     *)
(*    request still holds the pool (inflight = 0) — Arc refcount at zero.     *)
(*  - Buggy variant (HardKill=TRUE): as soon as the drain elapsed, even with  *)
(*    requests still bound. This models "hard kill on timeout".               *)
Reap(g) ==
    /\ status[g] = "retired"
    /\ g \in drained
    /\ (HardKill \/ inflight[g] = 0)
    /\ status' = [status EXCEPT ![g] = "dead"]
    /\ UNCHANGED <<slotGen, drained, inflight>>

Next ==
    \/ Dispatch
    \/ HotSwap
    \/ \E g \in Gens : Complete(g)
    \/ \E g \in Gens : DrainElapse(g)
    \/ \E g \in Gens : Reap(g)

(* Fairness: requests eventually complete, timers eventually fire, and a      *)
(* reapable pool is eventually reaped. Without these, nothing is forced to    *)
(* make progress and liveness is trivially false.                            *)
Fairness ==
    /\ \A g \in Gens : WF_vars(Complete(g))
    /\ \A g \in Gens : WF_vars(DrainElapse(g))
    /\ \A g \in Gens : WF_vars(Reap(g))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* SAFETY                                                                   *)
(***************************************************************************)

(* The slot always points at a live pool, so a newly-arriving request never  *)
(* binds to a retired or dead pool.                                          *)
SlotIsLive == status[slotGen] = "live"

(* THE key property: no in-flight request is ever bound to a pool whose       *)
(* workers have exited. Violating this means a request was dispatched to a     *)
(* torn-down pool — a dropped/failed request during a deploy.                 *)
NoDispatchToDead == \A g \in Gens : inflight[g] > 0 => status[g] # "dead"

(***************************************************************************)
(* LIVENESS                                                                  *)
(***************************************************************************)

(* Every retired pool is eventually reaped — no unbounded accumulation of     *)
(* draining pools under repeated deploys. Holds only because requests         *)
(* complete (WF on Complete); that is exactly the design point — teardown is  *)
(* bounded by request completion, not by the wall-clock drain timer alone.    *)
EventuallyReaped == \A g \in Gens : (status[g] = "retired") ~> (status[g] = "dead")

=============================================================================

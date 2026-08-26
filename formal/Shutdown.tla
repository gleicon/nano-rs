-------------------------------- MODULE Shutdown --------------------------------
(***************************************************************************)
(* Graceful shutdown / request draining.                                    *)
(*                                                                          *)
(* Code modelled: src/app/drain.rs (`RequestDrain`) and src/signal.rs        *)
(* (`ShutdownState`). On a shutdown signal the server stops accepting new     *)
(* requests, then waits for in-flight requests to finish (`await_complete`)   *)
(* up to a drain timeout.                                                    *)
(*                                                                          *)
(* Two things must hold for the drain to be correct:                        *)
(*                                                                          *)
(*  1. No request is accepted after the shutdown signal. `await_complete`     *)
(*     does an empty-check and then waits on a semaphore permit; that pattern *)
(*     only guarantees "count == 0 on return" if `active` cannot rise again   *)
(*     once draining begins. The accept path is gated on `is_shutting_down`,  *)
(*     which is what makes `active` monotonically non-increasing while        *)
(*     draining.                                                             *)
(*                                                                          *)
(*  2. A "drained" result really means zero in-flight. `request_completed`    *)
(*     releases the drain permit only on the 1 -> 0 transition, so a          *)
(*     successful `await_complete` corresponds to `active == 0`. The drain     *)
(*     timeout is the escape that guarantees shutdown terminates even if some  *)
(*     request never finishes — but that path is reported as a timeout, not a  *)
(*     clean drain.                                                          *)
(***************************************************************************)
EXTENDS Naturals

CONSTANTS
    MaxStarts,   \* total requests that may ever start (bounds the state space)
    MaxActive    \* in-flight capacity (bounds the counter)

ASSUME MaxStarts \in Nat /\ MaxActive \in Nat /\ MaxActive >= 1

VARIABLES
    shuttingDown,  \* has the shutdown signal been received?
    active,        \* in-flight request count (RequestDrain.active_requests)
    remaining,     \* how many more requests may still start
    outcome        \* "none" | "drained" | "timedout"

vars == <<shuttingDown, active, remaining, outcome>>

TypeOK ==
    /\ shuttingDown \in BOOLEAN
    /\ active \in 0..MaxActive
    /\ remaining \in 0..MaxStarts
    /\ outcome \in {"none","drained","timedout"}

Init ==
    /\ shuttingDown = FALSE
    /\ active = 0
    /\ remaining = MaxStarts
    /\ outcome = "none"

(* A request is accepted and begins. Gated on NOT shutting down — this is the  *)
(* `is_shutting_down` check on the accept path.                               *)
StartReq ==
    /\ ~shuttingDown
    /\ active < MaxActive
    /\ remaining > 0
    /\ active' = active + 1
    /\ remaining' = remaining - 1
    /\ UNCHANGED <<shuttingDown, outcome>>

(* An in-flight request finishes (DrainHandle::drop -> request_completed).     *)
CompleteReq ==
    /\ active > 0
    /\ active' = active - 1
    /\ UNCHANGED <<shuttingDown, remaining, outcome>>

(* The shutdown signal arrives, once. Accepting stops from here on.           *)
Signal ==
    /\ ~shuttingDown
    /\ shuttingDown' = TRUE
    /\ UNCHANGED <<active, remaining, outcome>>

(* Clean drain: `await_complete` returns true because the last request         *)
(* completed and active is now zero.                                         *)
DrainDone ==
    /\ shuttingDown
    /\ active = 0
    /\ outcome = "none"
    /\ outcome' = "drained"
    /\ UNCHANGED <<shuttingDown, active, remaining>>

(* Drain timeout: shutdown proceeds even though requests are still in flight.  *)
(* Reported as a timeout, never as a clean drain.                            *)
DrainTimeout ==
    /\ shuttingDown
    /\ outcome = "none"
    /\ outcome' = "timedout"
    /\ UNCHANGED <<shuttingDown, active, remaining>>

Next ==
    \/ StartReq
    \/ CompleteReq
    \/ Signal
    \/ DrainDone
    \/ DrainTimeout

Fairness ==
    /\ WF_vars(CompleteReq)
    /\ WF_vars(DrainDone \/ DrainTimeout)

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* SAFETY                                                                    *)
(***************************************************************************)

(* No request is accepted after the shutdown signal: while shutting down, the  *)
(* in-flight count never rises. This is the property that makes the            *)
(* empty-check-then-wait drain sound.                                         *)
NoAcceptAfterSignal == [][ shuttingDown => active' <= active ]_vars

(* A clean "drained" result really means there were zero in-flight requests.   *)
DrainedMeansEmpty == (outcome = "drained") => (active = 0)

(* Terminal outcomes are stable — a drain result is never overwritten.         *)
OutcomeStable == [][ outcome # "none" => outcome' = outcome ]_vars

(***************************************************************************)
(* LIVENESS                                                                  *)
(***************************************************************************)

(* Once shutting down, the drain always terminates — cleanly or via timeout.   *)
ShutdownTerminates == shuttingDown ~> (outcome # "none")

=============================================================================

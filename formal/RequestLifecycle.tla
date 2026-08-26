---------------------------- MODULE RequestLifecycle ----------------------------
(***************************************************************************)
(* The HTTP handler's contract: every accepted request produces a response, *)
(* the worker queue is bounded (backpressure), and the worker keeps making   *)
(* progress.                                                                 *)
(*                                                                          *)
(* Code modelled:                                                           *)
(*   - src/http/router.rs `dispatch_to_worker_pool` -> WorkQueue::try_dispatch*)
(*     (bounded `mpsc::sync_channel`; full => 503 Retry-After)               *)
(*   - src/worker/pool.rs request loop, `response_tx` (a oneshot channel)    *)
(*   - the per-request CPU-time limit (`CpuTimeoutGuard`, armed only when     *)
(*     `task.cpu_time_limit_ms > 0`) which calls `terminate_execution`        *)
(*   - the server-side timeout layer (`TimeoutLayer`, 30s)                    *)
(*                                                                          *)
(* Two DISTINCT timeouts, which earlier versions of this spec conflated:      *)
(*                                                                          *)
(*   CpuTerminate — the CPU-time limit aborts V8 execution on the worker      *)
(*     thread. This both responds to the client AND frees the worker to take  *)
(*     the next request. It is armed ONLY when a positive CPU limit is in     *)
(*     effect for the app (`cpu_time_limit_ms > 0`).                          *)
(*                                                                          *)
(*   ClientTimeout — the tower `TimeoutLayer` abandons the client side after  *)
(*     30s and returns 408. It frees the CLIENT but does NOT terminate the    *)
(*     worker's execution: a runaway handler keeps running and the worker     *)
(*     stays occupied.                                                        *)
(*                                                                          *)
(* The constant `CpuLimit` models whether a worker-side CPU limit is armed:   *)
(*   TRUE  = a limit is in effect. `get_cpu_time_limit_ms` now returns the      *)
(*           default (50ms) on every path unless an app disables it, so this    *)
(*           is the normal case — CpuTerminate is available.                   *)
(*   FALSE = no limit armed. Reachable only when an app explicitly sets         *)
(*           `cpu_time_enabled: false`; CpuTerminate never fires.              *)
(*                                                                          *)
(* With CpuLimit=TRUE the worker always frees and every request responds.     *)
(* With CpuLimit=FALSE a runaway handler wedges the worker: the client still  *)
(* gets a 408, but a request queued behind it never runs — TLC reports the    *)
(* liveness violation. This is why disabling the CPU limit is a foot-gun; the  *)
(* shared-router path used to hit it by default (get_cpu_time_limit_ms -> 0),   *)
(* which is now fixed.                                                        *)
(***************************************************************************)
EXTENDS Naturals

CONSTANTS
    NumRequests,   \* number of requests to explore (bounds the state space)
    QueueCap,      \* bounded worker-queue capacity (sync_channel depth)
    CpuLimit       \* is a worker-side CPU-time limit armed?

ASSUME NumRequests \in Nat /\ QueueCap \in Nat /\ QueueCap >= 1
       /\ CpuLimit \in BOOLEAN

Requests == 1..NumRequests

VARIABLES
    clientStatus,  \* [Requests -> {"new","queued","running","responded"}] (client view)
    outcome,       \* [Requests -> {"none","ok","rejected","timeout","client_timeout"}]
    worker,        \* the request occupying the single worker, or 0 for "free"
    qlen           \* number of requests sitting in the worker queue

vars == <<clientStatus, outcome, worker, qlen>>

TypeOK ==
    /\ clientStatus \in [Requests -> {"new","queued","running","responded"}]
    /\ outcome \in [Requests -> {"none","ok","rejected","timeout","client_timeout"}]
    /\ worker \in 0..NumRequests
    /\ qlen \in 0..QueueCap

Init ==
    /\ clientStatus = [r \in Requests |-> "new"]
    /\ outcome = [r \in Requests |-> "none"]
    /\ worker = 0
    /\ qlen = 0

(* Dispatch: enqueue if the bounded queue has room, else fail fast with 503.  *)
Enqueue(r) ==
    /\ clientStatus[r] = "new"
    /\ qlen < QueueCap
    /\ clientStatus' = [clientStatus EXCEPT ![r] = "queued"]
    /\ qlen' = qlen + 1
    /\ UNCHANGED <<outcome, worker>>

Reject(r) ==
    /\ clientStatus[r] = "new"
    /\ qlen = QueueCap
    /\ clientStatus' = [clientStatus EXCEPT ![r] = "responded"]
    /\ outcome' = [outcome EXCEPT ![r] = "rejected"]
    /\ UNCHANGED <<worker, qlen>>

(* The worker pulls one queued request and runs it — one at a time.           *)
StartWork(r) ==
    /\ clientStatus[r] = "queued"
    /\ worker = 0
    /\ clientStatus' = [clientStatus EXCEPT ![r] = "running"]
    /\ worker' = r
    /\ qlen' = qlen - 1
    /\ UNCHANGED outcome

(* The handler returns normally: respond OK and free the worker. Deliberately  *)
(* NOT a fair action — a runaway handler may never take this step.            *)
Finish(r) ==
    /\ worker = r
    /\ clientStatus[r] = "running"
    /\ clientStatus' = [clientStatus EXCEPT ![r] = "responded"]
    /\ outcome' = [outcome EXCEPT ![r] = "ok"]
    /\ worker' = 0
    /\ UNCHANGED qlen

(* CPU-time limit fires: terminate execution, freeing the worker, and respond  *)
(* if the client has not already been answered. Only available when a limit    *)
(* is armed.                                                                   *)
CpuTerminate(r) ==
    /\ CpuLimit
    /\ worker = r
    /\ worker' = 0
    /\ clientStatus' = IF clientStatus[r] = "running"
                       THEN [clientStatus EXCEPT ![r] = "responded"]
                       ELSE clientStatus
    /\ outcome' = IF clientStatus[r] = "running"
                  THEN [outcome EXCEPT ![r] = "timeout"]
                  ELSE outcome
    /\ UNCHANGED qlen

(* Server timeout layer: answer the client with 408, but leave the worker      *)
(* occupied — the handler is still running.                                    *)
ClientTimeout(r) ==
    /\ clientStatus[r] = "running"
    /\ clientStatus' = [clientStatus EXCEPT ![r] = "responded"]
    /\ outcome' = [outcome EXCEPT ![r] = "client_timeout"]
    /\ UNCHANGED <<worker, qlen>>

Next ==
    \E r \in Requests :
        \/ Enqueue(r) \/ Reject(r) \/ StartWork(r)
        \/ Finish(r) \/ CpuTerminate(r) \/ ClientTimeout(r)

(* Fairness models the real progress guarantees:                              *)
(*  - every arriving request is dispatched (enqueued or 503-rejected).         *)
(*  - the bounded queue is FIFO, so a queued request is not starved by the      *)
(*    scheduler: strong fairness on StartWork.                                 *)
(*  - the server timeout always eventually answers a running client.           *)
(*  - the CPU limit, WHEN ARMED, always eventually frees the worker.           *)
(*  - Finish is NOT fair: a runaway handler need never return on its own. The   *)
(*    worker is freed only by Finish or CpuTerminate, so with no CPU limit a    *)
(*    runaway wedges it.                                                        *)
Fairness ==
    /\ \A r \in Requests : WF_vars(Enqueue(r) \/ Reject(r))
    /\ \A r \in Requests : SF_vars(StartWork(r))
    /\ \A r \in Requests : WF_vars(ClientTimeout(r))
    /\ \A r \in Requests : WF_vars(CpuTerminate(r))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* SAFETY                                                                    *)
(***************************************************************************)

RespondedIffOutcome ==
    \A r \in Requests : (clientStatus[r] = "responded") <=> (outcome[r] # "none")

QueueBounded == qlen <= QueueCap

(* The worker, when busy, holds a request that is running or has been           *)
(* client-timed-out (worker still occupied after a 408).                        *)
WorkerOccupancy ==
    worker # 0 => clientStatus[worker] \in {"running","responded"}

(***************************************************************************)
(* LIVENESS                                                                  *)
(***************************************************************************)

(* Every request's client eventually gets a response. Holds with a CPU limit;  *)
(* with CpuLimit=FALSE a request queued behind a wedged worker never runs, and  *)
(* TLC returns that as a counterexample.                                        *)
EveryClientResponds == \A r \in Requests : <>(clientStatus[r] = "responded")

=============================================================================

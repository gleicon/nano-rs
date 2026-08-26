---------------------------- MODULE RequestLifecycle ----------------------------
(***************************************************************************)
(* The HTTP handler's core contract: every accepted request produces        *)
(* EXACTLY ONE terminal response, and no request hangs forever.             *)
(*                                                                          *)
(* Code modelled:                                                           *)
(*   - src/http/router.rs `dispatch_to_worker_pool` -> WorkQueue::dispatch   *)
(*   - src/worker/queue.rs bounded per-worker `mpsc::sync_channel`           *)
(*   - src/worker/pool.rs request loop, `response_tx` (a oneshot channel)    *)
(*   - the server-side timeout layer (TimeoutLayer, 30s) and per-request     *)
(*     CPU-time limit                                                        *)
(*                                                                          *)
(* Each request ends in exactly one outcome:                                *)
(*   ok       - the handler ran and the worker sent a response               *)
(*   rejected - the worker queue was full; dispatch returned 503 Retry-After *)
(*   timeout  - the CPU-time limit or the server timeout layer fired         *)
(*                                                                          *)
(* The oneshot sender is consumed by a send, so a response is sent AT MOST   *)
(* once by construction; if the worker drops the sender without sending, the *)
(* receiver observes an error and the handler still emits one 500 response.  *)
(* Either way the caller sees exactly one response, which is what `ok`       *)
(* abstracts. The `timeout` outcome is the escape that guarantees no request *)
(* is stuck `running` forever even if a handler never returns.               *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    NumRequests,   \* number of requests to explore (bounds the state space)
    QueueCap       \* bounded worker-queue capacity (sync_channel depth)

ASSUME NumRequests \in Nat /\ QueueCap \in Nat /\ QueueCap >= 1

Requests == 1..NumRequests

VARIABLES
    status,   \* [Requests -> {"new","queued","running","done"}]
    outcome,  \* [Requests -> {"none","ok","rejected","timeout"}]
    qlen,     \* number of requests currently sitting in the worker queue
    running   \* the request currently executing, or 0 for "none"

vars == <<status, outcome, qlen, running>>

TypeOK ==
    /\ status \in [Requests -> {"new","queued","running","done"}]
    /\ outcome \in [Requests -> {"none","ok","rejected","timeout"}]
    /\ qlen \in 0..QueueCap
    /\ running \in 0..NumRequests

Init ==
    /\ status = [r \in Requests |-> "new"]
    /\ outcome = [r \in Requests |-> "none"]
    /\ qlen = 0
    /\ running = 0

(* A new request is dispatched. If the bounded queue has room it is enqueued; *)
(* otherwise dispatch fails fast with 503 (backpressure), a terminal outcome. *)
Enqueue(r) ==
    /\ status[r] = "new"
    /\ qlen < QueueCap
    /\ status' = [status EXCEPT ![r] = "queued"]
    /\ qlen' = qlen + 1
    /\ UNCHANGED <<outcome, running>>

Reject(r) ==
    /\ status[r] = "new"
    /\ qlen = QueueCap
    /\ status' = [status EXCEPT ![r] = "done"]
    /\ outcome' = [outcome EXCEPT ![r] = "rejected"]
    /\ UNCHANGED <<qlen, running>>

(* The worker pulls one queued request and runs it (one at a time).          *)
StartWork(r) ==
    /\ status[r] = "queued"
    /\ running = 0
    /\ status' = [status EXCEPT ![r] = "running"]
    /\ running' = r
    /\ qlen' = qlen - 1
    /\ UNCHANGED outcome

(* The handler returns and the worker sends the response on the oneshot.      *)
Finish(r) ==
    /\ status[r] = "running"
    /\ running = r
    /\ status' = [status EXCEPT ![r] = "done"]
    /\ outcome' = [outcome EXCEPT ![r] = "ok"]
    /\ running' = 0
    /\ UNCHANGED qlen

(* The CPU-time limit or the server timeout layer fires — the escape that      *)
(* guarantees a running request always reaches a response even if the handler *)
(* never returns.                                                             *)
Timeout(r) ==
    /\ status[r] = "running"
    /\ running = r
    /\ status' = [status EXCEPT ![r] = "done"]
    /\ outcome' = [outcome EXCEPT ![r] = "timeout"]
    /\ running' = 0
    /\ UNCHANGED qlen

Next ==
    \E r \in Requests :
        Enqueue(r) \/ Reject(r) \/ StartWork(r) \/ Finish(r) \/ Timeout(r)

(* Fairness models the real system's progress guarantees:                      *)
(*  - every arriving request is dispatched (enqueued or 503-rejected): WF.       *)
(*  - the bounded queue is FIFO (mpsc::sync_channel), so a queued request is     *)
(*    not starved even while others run: strong fairness on StartWork.           *)
(*  - a running request always reaches a response, via Finish OR the Timeout     *)
(*    escape — this is what makes "no request hangs" hold regardless of what     *)
(*    the handler does: WF.                                                      *)
Fairness ==
    /\ \A r \in Requests : WF_vars(Enqueue(r) \/ Reject(r))
    /\ \A r \in Requests : SF_vars(StartWork(r))
    /\ \A r \in Requests : WF_vars(Finish(r) \/ Timeout(r))

Spec == Init /\ [][Next]_vars /\ Fairness

(***************************************************************************)
(* SAFETY                                                                    *)
(***************************************************************************)

(* A request has a response iff it is done — the two are inseparable.          *)
DoneIffResponded == \A r \in Requests : (status[r] = "done") <=> (outcome[r] # "none")

(* The queue is never over capacity (bounded backpressure holds).             *)
QueueBounded == qlen <= QueueCap

(* At most one request runs at a time on the worker.                          *)
AtMostOneRunning ==
    running # 0 => (status[running] = "running"
                    /\ \A r \in Requests : status[r] = "running" => r = running)

(***************************************************************************)
(* LIVENESS                                                                  *)
(***************************************************************************)

(* THE contract: every request eventually reaches a terminal response. No     *)
(* accepted request hangs; no request is silently dropped. Backpressure gives  *)
(* rejected requests an immediate 503; the timeout escape bounds running ones. *)
EveryRequestResponds == \A r \in Requests : <>(status[r] = "done")

=============================================================================

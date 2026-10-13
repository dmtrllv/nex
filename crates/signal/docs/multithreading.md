# Multithreading

## Goals

The signal runtime should support sharing reactive state across threads while preserving its existing reactive semantics and consumer-controlled scheduling model.

Multithreading support must allow:

- Signals to be explicitly declared as local or shared.
- Shared signals to be read and updated from multiple threads.
- Dependencies to cross thread boundaries.
- Writes to invalidate dependent scopes owned by other threads.
- Reactive work to execute according to the ownership and scheduling rules of the dependent computation.
- The core crate to remain independent of any particular executor or async runtime.
- Local reactive operations to avoid unnecessary synchronization overhead.

The initial implementation supports cross-thread state sharing and invalidation. Parallel execution of reactive computations is a separate, optional capability and is not part of the initial implementation.

## Design principles

### 1. Local and shared signals are distinct types

The public API distinguishes thread-confined state from cross-thread state.

- `Signal<T>` represents local reactive state.
- `SharedSignal<T>` represents reactive state that can be accessed across threads.

Both types participate in the same reactive model: tracked reads register dependencies, writes invalidate dependents, and pending work is queued for processing.

The distinction exists to make synchronization and access guarantees explicit. Local signals should not incur the synchronization costs required by shared signals.

The names and exact public signatures may change during implementation, but the distinction between local and shared state must remain explicit.

### 2. Shared values require thread-safe types

Shared signals initially support values satisfying:

```rust
T: Send + Sync + 'static
```

These bounds establish the initial contract for storing and accessing values across threads. They do not imply that all reactive resources, scopes, callbacks, or computations must be `Send + Sync`.

In particular, a scope may capture thread-confined state and therefore remain non-transferable.

### 3. Dependency tracking can cross thread boundaries

A reactive computation may depend on a shared signal regardless of which thread owns the computation.

For example, a scope owned by thread A may read a shared signal that thread B subsequently updates. The update must invalidate the dependent scope even though the scope does not execute on thread B.

Cross-thread dependencies are a fundamental requirement, not an optional integration layer.

### 4. Execution remains consumer-controlled

The runtime does not introduce a global scheduler or automatically execute reactive computations on arbitrary threads.

A shared signal write may invalidate dependent nodes and enqueue work, but it must not execute a foreign scope's closure or invoke its callbacks directly on the writing thread.

The consumer responsible for a scope determines when its pending work is processed. A wake-up mechanism may notify that consumer that work is available, but the mechanism used to deliver the notification is not part of the core reactive semantics.

### 5. Invalidation, queueing, and execution are separate operations

These operations must be treated as distinct:

1. **Invalidation:** A dependency changes, making a dependent computation or cached result potentially stale.
2. **Queueing:** Required reactive work is marked pending.
3. **Wake-up:** The owner is notified that work may need processing.
4. **Execution:** The owner processes the work according to the runtime's scheduling rules.

An implementation may combine some of these operations internally when safe, but it must preserve their distinct semantics.

## API design

### Local signals

Local signals retain the simple, reference-based access model where supported by the implementation.

Conceptually:

```rust
let count = signal(0);

count.set(1);

let value: &i32 = count.get();
```

Local signals are not required to support concurrent access. Their implementation should use the simplest synchronization model consistent with the runtime's local ownership rules.

### Shared signals

Shared signals provide cross-thread access through a separate API.

Conceptually:

```rust
let count = shared_signal(0);

count.set(1);

let value: Arc<i32> = count.get();
```

The example illustrates the proposed snapshot-based read model; the final signature must be established before implementation.

Shared reads must not return an ordinary reference whose validity depends on a synchronization guard that has already been released.

An owned snapshot such as `Arc<T>` is a candidate for the shared read API. It allows a reader to retain a stable version of the value while another thread publishes an update.

The implementation must decide whether snapshots are stored directly, produced on demand, or represented by another safe mechanism. That decision must preserve the required read and update semantics without exposing references into concurrently mutable storage.

Tracked and untracked reads must both be supported:

- A tracked read registers a dependency on the current reactive computation.
- An untracked read retrieves the value without registering a dependency.

The difference between these operations must remain consistent with the local signal API.

### Updates and snapshots

Concurrent writes must have a well-defined ordering and a linearization point: each successful write must take effect at a clearly defined point relative to other writes.

The initial contract does not promise that every intermediate value will be observed by dependent computations or subscription callbacks.

For example, if several writes occur before a dependent scope processes its pending work, the scope may execute once and observe the latest value.

This behavior is intentional and consistent with coalescing pending reactive work. Signals represent state, not an event log.

If consumers need guaranteed delivery of every intermediate value, that behavior should be provided by a separate event-stream abstraction.

## Dependency graph and synchronization

### Shared logical dependency model

The runtime must represent dependencies between local and shared reactive nodes, including dependencies that cross thread boundaries.

The initial architecture uses one logical dependency model rather than immediately introducing separate graphs connected by cross-thread subscriptions.

This is a logical design decision, not a requirement to place the entire graph behind one global mutex.

The implementation should allow synchronization to be localized where possible. Local operations should not require a global lock merely because shared signals exist elsewhere in the runtime.

The precise representation of node identities, dependency edges, and synchronization primitives remains an implementation decision.

### Concurrent invalidation

When a signal changes, the runtime must identify and invalidate its dependent nodes safely, including dependents owned by other threads.

The implementation must account for concurrent writes, dependency registration and replacement, work queue updates, and resource disposal.

It must not hold a graph-wide lock while executing user closures or callbacks.

If invalidation requires locks on multiple resources, lock ordering and lock lifetime must be designed to prevent deadlocks and avoid running arbitrary user code while internal synchronization is held.

### Dynamic dependencies

Reactive computations may change which signals they read between executions.

After a successful recomputation, the runtime must replace obsolete dependency edges with the dependencies observed during the new execution.

This must remain correct when a dependency changes concurrently with recomputation or dependency replacement. The implementation must define how it detects or prevents missed invalidations during this transition.

The implementation must not assume that dependency edges remain constant after initial evaluation.

### Memo dependencies

Memos retain their lazy evaluation semantics.

Invalidating a memo does not necessarily mean its output has changed. The memo must recompute when required by a read, and downstream dependents should only be notified according to the existing memo change-detection policy.

Cross-thread invalidation must preserve the distinction between:

- A memo becoming dirty.
- A memo being recomputed.
- The memo's output changing.
- Downstream work being queued.

Parallel memo evaluation is not required for the initial implementation.

## Scope ownership and scheduling

### Owner-controlled execution

A scope is associated with an execution owner. The initial implementation should preserve the ownership established by the runtime's scope and execution model.

A shared signal may invalidate that scope from another thread, but the scope's closure must execute through its owner-controlled processing path.

The same principle applies to effects and subscription callbacks that are associated with thread-confined state.

The implementation must not infer that a scope is safe to move across threads merely because one of its dependencies is a shared signal.

### Queues and wake-ups

Pending reactive work must be routed to the appropriate execution owner.

Cross-thread invalidation must be able to make work pending and notify the owner without executing that work on the invalidating thread.

The queue and wake-up design must preserve the existing guarantees:

- Pending work is coalesced by stable work identity.
- A work item can become pending again after it has been processed.
- Processing work may enqueue additional work.
- The consumer determines when to process pending work.
- Scope-local processing continues according to the existing `flush()` semantics.

The initial implementation does not prescribe a particular channel, atomic notification mechanism, or queue implementation.

### Batching

Batching groups writes within its defined execution context and defers processing until the appropriate batch boundary.

The initial multithreading implementation treats batches on different threads as independent. A batch on one thread is not an atomic transaction covering writes made concurrently by other threads.

Batching must not be used as a substitute for synchronization or cross-thread transaction semantics.

## Effects and subscriptions

Effects and subscription callbacks must obey the same execution ownership rules as other reactive work.

A thread updating a shared signal may cause callbacks to become pending, but it must not directly invoke callbacks that belong to another execution owner.

The implementation must define the interaction between concurrent notification, unsubscription, and disposal.

At minimum, it must ensure that internal data structures remain safe when unsubscription races with pending work. Whether unsubscription guarantees cancellation of callbacks that have not started, or waits for callbacks that are already running, must be explicitly specified.

Waiting for an in-progress callback can introduce deadlocks, so synchronous waiting must not be added without a deliberate design.

Subscription notifications follow the state-oriented semantics of the runtime. Unless explicitly guaranteed by another abstraction, they do not promise delivery of every intermediate value.

## Disposal and lifecycle

Multithreading introduces races between invalidation, work queueing, dependency changes, unsubscription, and disposal.

The implementation must ensure that:

- Disposed resources cannot be accessed through invalid internal references.
- Stale dependency edges are removed or safely ignored.
- Pending work cannot corrupt runtime state after its target is disposed.
- A disposed resource cannot be accidentally treated as a newly created resource with the same identity.
- Concurrent invalidation cannot resurrect disposed work.
- Queue entries and dependency edges have clearly defined ownership and cleanup rules.

Resource identity must remain stable for the lifetime of a resource and must not rely solely on memory addresses that can be reused.

Panic recovery and panic-safe runtime guards are deferred until the core implementation is established. Multithreading must not be treated as a reason to silently assume that panic recovery already exists.

## Error handling and consistency

The runtime must distinguish between thread-safe state access and atomic application-level operations.

A single signal update can be synchronized without making a sequence of updates to several signals an atomic transaction.

Likewise, the fact that an invalidation has been queued does not mean the dependent scope has already recomputed or that its cached result is current.

The existing semantics of cached scope results remain in effect:

- A scope can expose its cached result without triggering execution.
- A scope may be invalidated while retaining its previous cached result.
- Re-execution occurs through the scope's processing API.
- Consumers must not interpret invalidation as proof that execution has completed.

Any stronger consistency guarantees must be specified explicitly rather than inferred from thread-safe storage.

## Implementation plan

The implementation should proceed in the following stages.

### Stage 1: Finalize the ownership and API contracts

- Introduce the explicit distinction between local and shared signals.
- Define the shared signal's `Send + Sync + 'static` value bounds.
- Specify tracked and untracked shared reads.
- Finalize the shared read representation, including snapshot lifetime and update semantics.
- Define scope execution ownership.
- Specify the semantics of concurrent writes and coalesced notifications.
- Define the initial guarantees for unsubscription and disposal.

Do not begin by changing every existing signal or runtime type to use synchronization primitives.

### Stage 2: Define reactive identities and graph synchronization

- Establish stable identities for reactive nodes and queued work.
- Define how local and shared nodes are represented in the logical dependency graph.
- Define safe dependency registration, replacement, and removal.
- Establish lock ordering and synchronization boundaries.
- Ensure user code never executes while a graph-wide lock is held.

This stage should resolve the graph invariants before concurrent behavior is implemented.

### Stage 3: Implement shared signal storage and access

- Implement shared signal creation.
- Implement synchronized updates.
- Implement safe tracked and untracked reads.
- Ensure snapshots remain valid after subsequent writes.
- Integrate shared reads with dependency tracking.
- Add tests for concurrent reads and writes.

### Stage 4: Implement cross-thread invalidation

- Allow shared signal writes to invalidate dependent nodes owned by other threads.
- Make invalidation safe against concurrent dependency changes.
- Ensure pending work is deduplicated.
- Ensure invalidation does not execute foreign computations.
- Verify that concurrent writes cannot leave a dependent permanently stale without pending work or another valid reason for deferred execution.

### Stage 5: Integrate owner-directed queues and wake-ups

- Route invalidated work to the correct execution owner.
- Provide the internal mechanism required to notify that owner.
- Preserve consumer-controlled execution.
- Ensure processing newly queued work follows the existing `flush()` contract.
- Verify that foreign writes cannot execute owner-confined closures or callbacks.

### Stage 6: Harden lifecycle and concurrency behavior

- Test unsubscription racing with notification.
- Test disposal racing with invalidation.
- Test dependency replacement during concurrent updates.
- Test repeated writes before a scope is processed.
- Test nested reactive dependencies crossing ownership boundaries.
- Test concurrent invalidation of a scope with multiple dependencies.
- Test deadlock-sensitive operations and lock ordering.

Concurrency tests should verify observable semantics, not merely the absence of crashes or data races.

### Stage 7: Evaluate optional parallel execution

Only after cross-thread sharing and invalidation are reliable should the runtime consider executing independent computations in parallel.

Parallel execution requires additional rules for scheduling, ordering, failure propagation, and dependency consistency. It must not be assumed to follow automatically from making signals and the dependency graph thread-safe.

## Testing requirements

The multithreading implementation should include tests for the following cases:

1. A shared signal is read and updated from multiple threads.
2. A local scope depends on a shared signal updated by another thread.
3. A dependent scope is invalidated without executing on the writing thread.
4. Several writes are coalesced into pending work while the final state remains observable.
5. Tracked reads register dependencies and untracked reads do not.
6. A memo depending on a shared signal remains lazy.
7. A memo only notifies downstream consumers when its output changes according to the configured policy.
8. Dynamic dependencies are replaced correctly under concurrent updates.
9. Subscription cancellation and resource disposal are safe during concurrent notification.
10. A scope's cached result remains distinguishable from the scope's current invalidation state.
11. Batches on separate threads do not accidentally imply transaction semantics.
12. Processing work that queues additional work does not lose notifications or violate execution ownership.

## Out of scope for the initial implementation

The following are deliberately deferred:

- Automatic parallel execution of reactive computations.
- A global work-stealing scheduler.
- Executor-specific integration.
- Cross-thread atomic transactions spanning multiple signals.
- Guaranteed delivery of every intermediate signal value.
- Automatic migration of scope execution between threads.
- Panic recovery and runtime repair after a panic.
- A partitioned dependency graph connected through remote subscriptions.

A partitioned graph may be reconsidered if profiling shows that synchronization in the shared logical graph becomes a meaningful bottleneck. It should not be introduced before the simpler architecture has been implemented and measured.

## Relationship to async support

Multithreading and async support share some concepts, particularly work queueing and wake-ups, but they are separate concerns.

A thread-owned scope has an execution owner associated with a thread. An async task may resume on a different thread after being suspended.

Therefore, thread affinity must not be assumed to be equivalent to async task ownership.

The async integration should adapt the runtime's pending-work notifications to the executor's wake-up mechanism while preserving the core's consumer-controlled scheduling model. Its exact contract belongs in `async.md`.

## Acceptance criteria

The initial multithreading implementation is complete when:

- Local and shared signals are explicitly distinguished in the API.
- Shared signals enforce the `Send + Sync + 'static` value contract.
- Tracked and untracked shared reads are safe.
- Dependencies can cross thread boundaries.
- Concurrent writes safely invalidate dependent reactive nodes.
- Work is queued for the appropriate owner without executing foreign closures or callbacks.
- Existing memo, scope, batching, and coalescing semantics remain intact.
- Resource disposal and subscription lifecycle operations remain safe under concurrent activity.
- The implementation passes the defined concurrency tests without relying on a particular async executor.

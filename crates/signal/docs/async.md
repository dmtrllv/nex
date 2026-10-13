# Async Runtime Integration

## 1. Goals

The signals crate must support integration with asynchronous runtimes without depending on, exposing, or owning a specific async runtime.

The core reactive system must remain runtime-independent. Applications and runtime adapters are responsible for scheduling scope execution and deciding when queued work should be processed.

The async architecture must support:

- Notifying consumers when a scope is invalidated or has pending reactive work.
- Deferred notification delivery through queues.
- Safe access to a scope when delivering subscription callbacks.
- Consumer-controlled scheduling of `Scope::run()` and `Scope::flush()`.
- Integration with async executors through runtime-specific adapters.
- Coalescing redundant notifications without losing pending work.
- Inspecting cached scope results without triggering recomputation.
- Preserving the last successful result when a subsequent execution fails.

## 2. Architecture and responsibilities

The architecture consists of three layers.

### 2.1. Reactive core

The signals crate owns:

- Signals, memos, effects, and scopes.
- Dependency tracking and invalidation.
- Queues for reactive work and deferred scope notifications.
- Scope event subscriptions.
- Cached scope results.
- Scope execution and flushing.
- Delivery of scope notifications at safe execution points.

The core must not depend on Tokio, async-std, or another executor. It must not impose a global scheduling policy.

### 2.2. Consumer or application

The consumer decides:

- When to execute an invalidated scope.
- When to flush pending work.
- Whether to process notifications immediately or defer them.
- Whether to enqueue scopes for later execution.
- How to prioritize, batch, or limit scope processing.
- How to prevent unnecessary work when several notifications refer to the same scope.

The consumer can implement a synchronous event loop, an asynchronous task, or a custom scheduler.

### 2.3. Runtime adapter

A runtime-specific adapter connects the core to an executor.

An adapter may translate scope notifications into task wake-ups, schedule processing on an executor, or bridge the notification mechanism to a runtime's synchronization primitives.

The adapter must not replace the core's dependency tracking or take ownership of its reactive execution semantics.

## 3. Scope execution model

A scope represents a reactive computation and owns its execution state, dependencies, resources, and cached result.

The initial execution initializes the scope's resources and establishes its dependencies. Subsequent executions reuse resources according to the core's resource-management rules.

### `Scope::run()`

`run()` executes the scope only when it has never successfully executed or has been invalidated by a relevant dependency change.

Its intended behavior is:

- On the first successful execution, initialize resources, establish dependencies, and cache the result.
- If the scope is valid, return the cached result without executing the closure again.
- If a tracked dependency changes, mark the scope invalidated.
- On the next `run()`, execute the closure again and replace the cached result only after a successful execution.
- Return an error if execution fails, allowing the consumer to inspect the failure.

The intended API shape is conceptually:

`run(&mut self) -> Result<&T, ScopeError>`

The exact error type and error propagation mechanism belong to the core API design.

A failed execution must not discard the last successful cached result. The scope remains invalidated so that a later `run()` can attempt recomputation again.

### `Scope::result()`

`result()` provides read-only access to the last successful result without triggering execution.

Its intended API shape is:

`result(&self) -> Option<&T>`

It returns `None` before the first successful execution. After a successful execution, it returns the cached result even if the scope has subsequently been invalidated or a later execution failed.

Consequently, the returned result may be stale. This is intentional: `result()` is an inspection API, not an execution API.

### `Scope::flush()`

`flush()` processes queued reactive work belonging to that scope only.

It does not inherently rerun the scope closure. Scope invalidation and pending reactive work are independent conditions:

- An invalidated scope may require `run()`.
- A scope with pending work may require `flush()`.
- A scope may require both operations.
- A scope may require neither operation.

The consumer chooses the order and timing of these operations.

## 4. Scope notifications

A scope subscription notifies the consumer when the scope requires attention.

The event type distinguishes the two reasons:

- `ScopeEvent::Invalidated`: the scope's cached computation may need to be recomputed.
- `ScopeEvent::Pending`: the scope has queued reactive work that may need to be flushed.

The conceptual callback signature is:

`FnMut(&mut Scope<T>, ScopeEvent)`

The scope is the first argument because the callback is expected to manage scope execution. The event is the second argument because it describes why the callback was invoked.

Example:

```rust
let _subscription = app.subscribe(|scope, event| match event {
    ScopeEvent::Invalidated => {
        if let Err(error) = scope.run() {
            eprintln!("Scope execution failed: {error}");
        }
    }
    ScopeEvent::Pending => {
        scope.flush();
    }
});
```

This illustrates the intended API contract. The final implementation must ensure that callback delivery and mutable scope access are borrow-safe.

### 4.1. Deferred notification delivery

Notifications must not recursively invoke subscribers while the scope is already executing, flushing, or delivering a callback.

Instead:

1. A reactive operation changes scope state.
2. The core records the relevant event in a queue.
3. The event is delivered at a safe execution point.
4. The callback receives temporary mutable access to the scope.
5. Events generated during callback execution are queued for later delivery.

The core must define which operations establish safe delivery points and ensure that callbacks cannot cause overlapping mutable access to the same scope.

The notification queue and the reactive-work queue serve different purposes. Queuing an event does not itself execute the work described by that event.

### 4.2. Coalescing notifications

Notifications should be coalesced while their corresponding condition remains active.

For example, repeated dependency changes while a scope is already invalidated should not require an independent `Invalidated` notification for every change.

The implementation must track invalidation and pending work independently. Clearing or processing one condition must not accidentally suppress the other.

Coalescing must also preserve correctness when new changes occur during `run()`, `flush()`, or callback delivery. An event must not be lost merely because an earlier event is currently being processed.

The exact notification-reset and acknowledgement rules must be defined alongside the core queue implementation.

### 4.3. Subscription lifecycle

Subscriptions must have an explicit lifecycle and well-defined unsubscription behavior.

The implementation must specify:

- Whether subscriptions remain active until explicitly unsubscribed or are removed on drop.
- What happens when a subscription is removed while notifications are queued.
- Whether subscribing to a scope that already needs attention produces an initial notification.
- How callback storage and temporary callback access avoid conflicting mutable borrows.
- How queued events are handled when a scope or subscription is disposed.

These rules must be deterministic and must not depend on a particular async executor.

## 5. Consumer-controlled scheduling

The core reports scope events; the consumer decides what to do with them.

A synchronous consumer may process notifications and execute the scope directly. An asynchronous consumer may wake a task and schedule scope processing later.

For example, an application may maintain separate queues:

- A queue of invalidated scopes that need `run()`.
- A queue of scopes with pending reactive work that need `flush()`.

The consumer may also maintain a single queue of scopes requiring attention and inspect their state when processing them.

The signals crate must not require either scheduling strategy. It provides the state, operations, and notifications needed to implement them.

If the consumer defers a scope for later processing, it remains responsible for ensuring that the scope is eventually processed according to the application's scheduling policy.

## 6. Async runtime integration

The core notification mechanism must be suitable for bridging to asynchronous executors.

A runtime adapter may use an executor-specific wake mechanism to notify a task that scope processing is required. The task can then arrange for the consumer's scheduler to call `run()` or `flush()` when it has exclusive access to the scope.

The adapter must account for:

- Notifications arriving before a task begins waiting.
- Multiple notifications arriving while a task is already scheduled.
- Notifications generated while a scope is being processed.
- Avoiding lost wake-ups when transitioning between processing and waiting.
- Ensuring that queued scope work remains discoverable after a wake-up.
- Avoiding simultaneous mutable access to the same scope.
- Runtime-specific cancellation and task shutdown.

A wake-up is an indication that work may be available, not a guarantee that exactly one event has occurred. The consumer must inspect or drain the relevant queues and state rather than assuming one wake-up corresponds to one unit of work.

The core remains responsible for correct reactive state and notification bookkeeping. The adapter is responsible for bridging those notifications to its executor without losing the ability to make progress.

## 7. Execution failures and cached results

Scope execution failures must not corrupt the last successfully computed result.

If a scope has a cached result and a later execution fails:

- The previous result remains available through `result()`.
- The failed execution returns an error to the caller.
- The scope remains invalidated.
- A subsequent `run()` may retry execution.
- The consumer decides when and whether to retry.

The implementation must also define how dependency registrations and resources are handled when an execution fails partway through. In particular, it must not silently treat a failed execution as a successful recomputation.

Panic recovery is a separate concern from ordinary execution errors. Panic guards and unwind recovery are not required by this async design and can be addressed independently later.

## 8. Implementation invariants

The implementation must preserve the following invariants:

1. The core has no dependency on a particular async runtime.
2. The consumer owns scheduling decisions.
3. `run()` caches successful results and skips execution when the scope remains valid.
4. `result()` never triggers execution and preserves access to the last successful result.
5. `flush()` processes only the scope's queued reactive work.
6. Scope invalidation and pending reactive work are tracked independently.
7. Notifications are deferred when immediate delivery would cause unsafe reentrancy or overlapping mutable access.
8. Events generated during processing remain queued for subsequent delivery.
9. Notification coalescing never loses relevant work.
10. Runtime adapters may wake tasks but do not replace the core's reactive state management.
11. Failed executions preserve the previous successful result and leave the scope invalidated.
12. Disposal and unsubscription prevent further delivery according to the documented lifecycle rules.

## 9. Suggested implementation order

1. Define `ScopeEvent` and the exact semantics of invalidation and pending-work notifications.
2. Finalize the scope notification queue and event-coalescing rules.
3. Implement deferred callback delivery with safe mutable access to the scope.
4. Integrate notification delivery with `run()` and `flush()` execution boundaries.
5. Implement cached scope results, including failure behavior.
6. Define subscription initialization, unsubscription, and disposal semantics.
7. Add tests for simultaneous invalidation and pending work, repeated invalidations, events generated during callbacks, and failed executions.
8. Verify that notifications cannot be lost across processing and waiting transitions.
9. Build a runtime-independent consumer loop using only the core API.
10. Implement runtime-specific adapters after the core contract is stable.

## 10. Scope of this document

This document specifies the async integration architecture and its implementation requirements. It does not mandate a particular executor, task model, scheduling algorithm, or concurrency strategy.

Cross-thread sharing and parallel execution are separate design concerns covered by the multithreading architecture. They must not be assumed to follow automatically from async runtime integration.
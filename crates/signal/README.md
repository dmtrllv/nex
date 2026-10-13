# Signals

A generic, single-threaded reactive state-management crate written in Rust.

The crate provides signals, lazy memos, effects, persistent scopes, subscriptions, and batching. It is independent of any UI framework or rendering backend and can be integrated into synchronous applications, event loops, and asynchronous runtimes.

The runtime manages reactive state, tracks dependencies, and queues reactive work. **Scheduling belongs to the consumer.** The crate does not expose a global scheduler or decide when every scope should execute. Consumers use scope events to decide when to reevaluate a scope or flush its pending work.

> **Development status:** The runtime is under implementation. This document describes the intended API and semantics; behavior should be considered implemented only when supported by the code and tests.

## Example

Run the example from the workspace root:

```sh
cargo run -p signals-example
```

```rust
use signal::{ScopeEvent, effect, memo, scope, signal};

fn main() {
    let mut app = scope(|| {
        let price = signal(25.0);
        let quantity = signal(2);

        let total = memo(|| price.value() * quantity.value() as f64);

        effect(|| {
            println!("Price: €{:.2}", price.value());
        });

        effect(|| {
            println!("Cart total: €{:.2}", total.value());
        });

        println!("Inspecting price: €{:.2}", price.inspect());

        (price, quantity)
    });

    let _subscription = app.subscribe(|app, event| match event {
        ScopeEvent::Invalidated => {
            let (price, quantity) = app.run();

            println!("Price: €{:.2}", price.get());
            println!("Quantity: {}", quantity.get());
        }
        ScopeEvent::Pending => {
            app.flush();
        }
    });

    let (price, quantity) = app.run();

    println!(
        "Initial price: {}, quantity: {}",
        price.value(),
        quantity.value()
    );

    quantity.set(3);
    price.set(30.0);

    // The consumer decides when to reevaluate or flush.
    let result = app.run();
    println!("Latest result: {result:?}");
}
```

The example illustrates the main responsibilities:

- `Signal<T>` stores mutable reactive state.
- `Memo<T>` lazily computes and caches a derived value.
- `effect()` creates a reactive side effect.
- `Scope<T>` owns a persistent execution context and cached result.
- `ScopeEvent` communicates invalidation and pending work to the consumer.
- `run()` reevaluates an invalidated scope or returns its cached result.
- `flush()` drains pending reactive work belonging to that scope.
- `subscribe()` registers callbacks for notifications.
- The consumer decides when and how to respond to scope events.

The example demonstrates the intended API and scheduling model. Exact callback timing depends on the implementation of the runtime and its notification queue.

## Public API

| API | Responsibility |
|---|---|
| `signal(value)` | Creates or retrieves a mutable signal in the active scope. |
| `memo(f)` | Creates or retrieves a lazy, cached, read-only derived value. |
| `effect(f)` | Creates or retrieves a reactive side effect. |
| `scope(f)` | Creates a deferred, persistent execution scope. |
| `get()` | Reads a value and tracks a dependency. |
| `inspect()` | Reads a value without tracking a dependency. |
| `value()` | Copies a `Copy` value and tracks a dependency. |
| `inspect_value()` | Copies a `Copy` value without tracking. |
| `set(value)` | Replaces a signal's value. |
| `update(f)` | Computes a replacement signal value. |
| `subscribe(f)` | Subscribes to signal, memo, or scope notifications. |
| `run()` | Evaluates an invalidated scope or returns its cached result. |
| `result()` | Returns the cached scope result, if available, without evaluating. |
| `flush()` | Drains pending reactive work belonging to a scope. |
| `batch(f)` | Groups writes into a batch. |
| `unsubscribe()` | Explicitly removes a subscription. |

`Signal<T>` and `Memo<T>` share read and subscription semantics, but only signals are directly mutable. A memo's value is controlled by its computation and dependencies.

## Reactive reads

Reactive dependency tracking is based on reads performed while a reactive computation is active.

| Operation | Tracks a dependency? | Returns a copy? |
|---|---|---|
| `get()` | Yes | No |
| `inspect()` | No | No |
| `value()` | Yes | Yes, for `T: Copy` |
| `inspect_value()` | No | Yes, for `T: Copy` |

A tracked read registers the active reactive context as a dependent of the value being read. An untracked read obtains the value without registering that dependency.

For example:

```rust
let count = signal(0);

effect(|| {
    println!("Count: {}", count.value());
});
```

The effect depends on `count` because it reads the signal through `value()`.

By contrast:

```rust
effect(|| {
    println!("Count: {}", count.inspect_value());
});
```

This read does not make the effect depend on `count`.

The same distinction applies to memos. Whether a read tracks a dependency is determined by the chosen read method, not by whether reading a dirty memo causes it to recompute.

## Signals

A signal stores mutable state and acts as a source of reactive dependencies.

```rust
let count = signal(0);

count.set(1);
count.update(|value| value + 1);

assert_eq!(count.inspect_value(), 2);
```

### Writes and invalidation

`set()` replaces the stored value. `update()` computes a replacement from the current value.

A write may invalidate dependent memos, effects, scopes, and subscriptions according to their respective semantics. Invalidating a dependent does not mean that all resulting work executes immediately.

The intended processing sequence is:

1. A write updates a signal.
2. Relevant dependents are invalidated.
3. Required reactive work and notifications are queued in the appropriate scope.
4. Duplicate pending work is coalesced.
5. The consumer decides when to flush the scope.
6. The runtime processes the queued work and any further work produced during processing.

A signal can be written multiple times before its scope is flushed. The runtime should avoid queueing the same pending work repeatedly when one queued execution or notification is sufficient.

Deduplication applies to pending work, not to the writes themselves. Each write still updates the signal's current value.

### Subscriptions

A signal subscription registers a callback for signal change notifications.

```rust
let count = signal(0);

let subscription = count.subscribe(|value| {
    println!("Count changed: {value}");
});

count.set(1);

// Remove the subscription explicitly.
subscription.unsubscribe();
```

A subscription is an explicit notification mechanism, distinct from dependency tracking.

- An **effect** tracks reactive reads and reruns when its dependencies require it.
- A **subscription** listens for notifications from the signal or memo to which it is attached.

Subscription callbacks are queued as reactive work rather than invoked immediately in the middle of a write. The owning scope's flush processes that work.

If a subscription already has a pending notification, additional invalidations should not create duplicate pending entries for that subscription. The callback observes state according to the subscription's documented callback contract; consumers should not assume that every intermediate value is delivered unless that behavior is explicitly guaranteed.

Unsubscription must prevent future callback execution according to the runtime's disposal contract, including the case where a notification is already queued.

## Memos

A memo is a read-only, cached computation derived from reactive values.

```rust
let count = signal(21);
let doubled = memo(|| count.value() * 2);

assert_eq!(doubled.value(), 42);
```

`memo()` returns a `Memo<T>` handle, not the computed `T` directly. The handle retains the cached result and the dependencies observed by its computation.

Memos are read-only with respect to their derived values, but they can be subscribed to. Subscribers are notified when the memo's output changes according to its change-detection policy.

### Lazy evaluation

A memo evaluates its computation when its result is first requested. Once computed, the result is reused until the memo is invalidated.

When an input changes, the memo becomes dirty. It recomputes the next time a read requires its value.

The intended lifecycle is:

1. Create or retrieve the memo resource.
2. Evaluate the computation on the first read.
3. Track the reactive sources read during computation.
4. Cache the result.
5. Mark the memo dirty when a dependency changes.
6. Recompute on a subsequent read when the cache is dirty.
7. Determine whether the recomputed output changed.
8. If the output changed, invalidate downstream dependents and queue the relevant notifications.

Reading a clean memo does not rerun its computation.

### Tracked and untracked memo reads

```rust
let count = signal(10);
let doubled = memo(|| count.value() * 2);

effect(|| {
    println!("Doubled: {}", doubled.value());
});
```

The effect depends on `doubled` because it performs a tracked read.

Using `doubled.inspect_value()` instead reads the memo without registering the effect as a dependent through that read.

Both tracked and untracked reads must return a current value. If a memo is dirty, reading it must recompute it before returning the result.

### Dynamic dependencies

A memo's dependencies must match the sources read during its latest successful computation.

```rust
let use_first = signal(true);
let first = signal(10);
let second = signal(20);

let selected = memo(|| {
    if use_first.value() {
        first.value()
    } else {
        second.value()
    }
});
```

Initially, `selected` depends on `use_first` and `first`. After switching to `second` and recomputing, it should stop depending on `first`.

The runtime must remove obsolete dependency edges when replacing a memo's dependency set.

### Memo subscriptions and downstream effects

A memo's subscription callbacks and its reactive dependents serve related but distinct purposes.

```rust
let count = signal(1);
let doubled = memo(|| count.value() * 2);

let _subscription = doubled.subscribe(|value| {
    println!("Doubled changed: {value}");
});

effect(|| {
    println!("Effect sees: {}", doubled.value());
});
```

When `count` changes, `doubled` becomes dirty. When the memo recomputes, the runtime determines whether its output changed.

If the output changed:

- The memo's reactive dependents, including effects that read it through tracked reads, must be invalidated.
- The memo's subscribers must have their notifications queued as scope work.
- Repeated invalidations must not create duplicate pending entries for the same work item.

The dependent effect must see the current memo value when it executes. The runtime must therefore ensure that dirty memos are brought up to date before dependent work consumes their values.

Memo notifications should not execute subscriber callbacks in the middle of memo recomputation. They should be delivered through the scope's pending-work mechanism.

### Memo change detection

Invalidation, recomputation, and output change are different concepts.

For example, a memo computing `count.value() > 0` may recompute when `count` changes from `1` to `2`, while its output remains `true`.

The runtime should distinguish these cases:

- **Invalidation:** an input changed, so the cached result may be stale.
- **Recomputation:** the memo evaluates its computation to produce a current result.
- **Output change:** the newly computed result differs according to the memo's change-detection policy.

A memo's subscribers should be notified when the output is considered changed, not merely because the memo became dirty. The exact equality or change-detection policy must be defined by the implementation and must not accidentally impose a `PartialEq` bound on all memo values.

Memos are read-only. To change a memo's result, change one of its input signals.

## Effects

An effect performs side effects in response to changes in the reactive values it reads.

```rust
let count = signal(0);

effect(|| {
    println!("Count: {}", count.value());
});
```

An effect executes on its initial creation. Reactive reads during execution establish its dependencies. When a dependency changes, the effect is invalidated and its subsequent execution is scheduled as pending work.

The intended lifecycle is:

1. Create or retrieve the effect resource.
2. Execute the effect initially.
3. Track the dependencies read during execution.
4. Invalidate and queue the effect when a dependency changes.
5. Deduplicate its pending work by stable work identity.
6. Execute pending work when its owning scope is flushed.
7. Replace the effect's dependencies after a successful rerun.

Conditional reads are supported, so an effect's dependency set can change between executions.

An effect that depends on a memo must observe the current memo result when it runs. If the memo's output has not changed, the runtime should avoid rerunning the effect solely because an upstream input changed, provided the implementation's change-detection policy supports that guarantee.

## Scopes

A scope is a deferred, persistent execution context that owns reactive resources and a cached result.

```rust
let mut app = scope(|| {
    let count = signal(42);
    let doubled = memo(|| count.value() * 2);

    (count, doubled)
});
```

Creating a scope does not execute its closure. The consumer explicitly calls `run()` to obtain its initial result.

### `run()`

`run()` evaluates the scope when its result is unavailable or invalidated. If the scope is already valid, it returns the cached result rather than executing the closure again.

```rust
let first = app.run();
let second = app.run();
```

If no relevant dependency changed between these calls, the second call should reuse the result of the first execution.

The scope closure uses persistent resource identity: signals, memos, and effects created at corresponding resource positions are reused across runs. Existing signal values must not be reset to their initial values merely because the scope is reevaluated.

### `result()`

`result()` returns `Option<&T>` for the currently cached scope result.

- `None` means the scope has not produced a result yet.
- `Some(&T)` provides access to the cached result.
- Calling `result()` does not run the scope or force reevaluation.

Use `result()` when the consumer needs to inspect the cached output without triggering execution.

### Positional resource reuse

Resources are associated with their positions in the scope's execution sequence. Resource-creating calls should remain in a stable order between runs.

Conditionally creating a signal before another signal can shift the latter's position and cause the runtime to associate a slot with the wrong logical resource.

Keep conditional logic inside a stable memo or effect rather than conditionally changing the scope's resource-creation sequence. The runtime should validate resource kinds and types when reusing slots, although same-type logical swaps cannot always be detected automatically.

### Scope invalidation

A scope tracks the dependencies relevant to its execution result. When a relevant dependency changes, the scope becomes invalidated.

**Invalidation does not automatically execute the scope closure.** It tells the consumer that the cached result may no longer be current. The consumer decides when to call `run()` again.

Reads inside a memo belong to that memo's computation. They should not automatically become direct dependencies of the enclosing scope merely because the memo was read while the scope was executing.

## Scope events and subscriptions

Scopes expose notifications through `Scope::subscribe()`.

```rust
let mut app = scope(|| {
    let count = signal(0);
    count
});

let subscription = app.subscribe(|app, event| {
    match event {
        ScopeEvent::Invalidated => {
            let count = app.run();
            println!("Current count: {}", count.get());
        }
        ScopeEvent::Pending => {
            app.flush();
        }
    }
});
```

The callback receives access to the scope so the consumer can respond to notifications. The consumer is responsible for deciding how to schedule that response.

### `ScopeEvent::Invalidated`

Indicates that the cached scope result is invalid and should be reevaluated when appropriate.

The consumer can call `run()` immediately, schedule a later run, or mark the scope as needing reevaluation in its own application scheduler.

Repeated invalidations should not generate redundant notifications while an equivalent invalidation notification is already pending.

### `ScopeEvent::Pending`

Indicates that the scope has reactive work waiting to be processed.

The consumer can call `flush()` immediately or arrange for it to run later. If additional work is queued while work is already pending, the consumer should not need a separate wake-up for every item.

The runtime must ensure that the pending state accurately reflects whether scope work remains to be processed.

### Event delivery and reentrancy

Invalidation and pending work are distinct conditions and should be tracked independently.

Notifications are queued rather than delivered synchronously in the middle of a signal write, memo recomputation, or other reactive operation. Repeated notifications for the same pending condition should be coalesced.

Event delivery must account for reentrancy. A callback may call `run()` or `flush()`, and those operations may produce further events. The runtime should queue or defer those events safely instead of causing uncontrolled nested notification delivery.

Consumers should rely on the documented meaning of events, not on incidental callback ordering beyond the established contract.

## Flushing and scheduling

The crate does **not** own a global scheduler. It provides the scope operations required for a consumer to implement scheduling.

`Scope::flush()` drains pending reactive work belonging to that scope. It must not drain another scope's work as a side effect.

Pending work can include effect executions, signal or memo subscription callbacks, and other reactive notifications associated with the scope.

### Queued work and deduplication

The runtime maintains pending reactive work in scope-owned queues. Work is queued for later processing rather than executed immediately at the point of invalidation.

The queue follows these semantics:

1. When reactive work becomes necessary, enqueue it in the appropriate scope queue.
2. Identify work using a stable internal identity.
3. If that work is already queued, do not enqueue another copy in the same queue.
4. When `flush()` processes an item, remove its pending status at the appropriate point.
5. If processing an item creates additional work, enqueue that work according to the same rules.
6. Continue processing until the scope's queue has no remaining work.

Deduplication is based on work identity, not callback equality. Distinct subscriptions remain distinct even if they use similar callbacks.

Deduplication does not discard writes. Signals still store their latest values; the queue avoids redundant processing of work that is already pending.

### Flushing

`Scope::flush()` drains the scope's pending reactive work until its queue is empty. This includes work queued while the flush is already running.

A flush may execute effects, subscription callbacks, and other reactive work associated with the scope. Processing one item may invalidate computations or enqueue further notifications, which are processed during the same flush.

The runtime must ensure that pending-work bookkeeping remains correct when an item is executing and causes itself or other work to be queued again.

Reactive cycles or callbacks that continually generate new work can prevent a queue from becoming empty. Cycle detection or other protections may be considered separately; they must not silently change the normal contract that `flush()` drains newly queued work in the same call.

### Batching and queue separation

Batching will introduce separate queues for work associated with different batching states. This allows the runtime to defer processing until the relevant batch completes, rather than processing work between every write.

When batching is implemented, the queue-selection and release rules must preserve scope ownership and work deduplication. `flush()` should drain the queue or queues that are eligible for processing under the active batching rules, including newly queued eligible work, without processing work that remains deferred by an active batch.

### Work created during a flush

Processing queued work can invalidate additional computations or enqueue more notifications. The runtime should process newly queued work consistently and should normally continue draining until the scope has no pending work.

The implementation must also account for reactive cycles or callbacks that continually produce more work. A work limit or equivalent cycle-handling policy may be needed to prevent a flush from running indefinitely.

### Consumer-owned scheduling

The separation of responsibilities is intentional:

- Signals own mutable values.
- Memos cache derived values.
- Effects perform side effects.
- Subscriptions receive explicit notifications.
- Scopes own resources and scope-local pending work.
- Scope events notify consumers about invalidation and pending work.
- Consumers decide when scopes run and flush.

The crate does not require a particular event loop, task executor, or UI rendering strategy.

## Integrating with an async runtime

An async application can use scope notifications to decide when to wake a task or schedule work on an executor. The signals crate does not need to spawn tasks or depend on a specific async runtime.

Conceptually, an adapter can:

1. Subscribe to scope events.
2. Translate invalidation or pending-work notifications into a wake-up or queued application task.
3. Call `run()` when the scope result needs reevaluation.
4. Call `flush()` when the scope has pending reactive work.
5. Coalesce redundant wake-ups and avoid reentrant processing.

The adapter owns the async scheduling policy. The signals crate owns reactive state, dependency tracking, invalidation, and scope-local pending work.

A consumer should not assume that every signal write directly wakes an async task, or that a scope is automatically rerun or flushed without consumer action.

## Batching

`batch()` groups writes into a single scheduling interval.

```rust
batch(|| {
    first.set(10);
    second.set(20);
});
```

Nested batches defer the release of batched work until the outermost batch completes. This prevents reactive work from being processed between every write in a group.

Batching does not imply that the consumer must flush immediately. Pending work remains subject to the scope's scheduling policy.

The runtime must define how batched invalidations propagate through memo chains and how redundant pending work is deduplicated.

## Disposal and subscriptions

A subscription can be explicitly removed by consuming its handle:

```rust
let subscription = count.subscribe(|value| {
    println!("{value}");
});

subscription.unsubscribe();
```

The runtime must consistently define resource lifetimes and disposal behavior, including:

- Removing obsolete dependency registrations.
- Preventing disposed effects from executing.
- Preventing queued notifications from invoking disposed subscriptions.
- Preventing stale handles from accessing recycled resource slots.
- Cleaning up resources no longer retained by an execution.
- Avoiding ownership cycles.

Dropping a subscription handle does not necessarily unsubscribe unless that behavior is explicitly implemented and documented. Call `unsubscribe()` when explicit removal is required.

## Runtime safety and errors

The runtime must preserve internal consistency when execution fails. Important cases include:

- Panics in scope, memo, effect, subscription, or update closures.
- Nested or reentrant `run()` and `flush()` calls.
- Reactive cycles and work-limit exhaustion.
- Borrow conflicts.
- Disposal while work is queued.
- Resource kind or type mismatches.
- Failed memo recomputation or dependency replacement.

Panic guards and recovery behavior will be implemented after the core runtime works. The goal is to restore internal state during unwinding and allow the original panic to propagate unless the API explicitly specifies otherwise.

## Design principles

1. **Consumer-owned scheduling:** the crate does not dictate when scopes execute.
2. **Scope-local work:** flushing a scope processes that scope's pending reactive work.
3. **Queued notifications:** callbacks are delivered as pending work rather than invoked immediately during invalidation.
4. **Work deduplication:** each pending work item has a stable identity and is queued at most once at a time.
5. **Explicit invalidation:** a stale scope result is reevaluated only when the consumer calls `run()`.
6. **Lazy derived values:** memos cache results and recompute when a dirty result is read.
7. **Observable memos:** memos are read-only but support subscriptions that are notified when their outputs change.
8. **Explicit dependency tracking:** tracked reads register dependencies; untracked reads do not.
9. **Persistent resource identity:** resources are reused across scope runs at corresponding positions.
10. **Runtime independence:** no dependency on a particular UI framework, event loop, or async executor.
11. **Deterministic processing:** notification coalescing, reentrancy, and work processing follow a defined contract.
12. **Panic-safe state:** failures must not leave the reactive graph or leave execution context corrupted.
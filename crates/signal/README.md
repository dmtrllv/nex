# Signals

A generic, single-threaded reactive state-management crate written in Rust.

The crate provides signals, memos, effects, persistent scopes, subscriptions, batching, and a deterministic scheduler. It is independent of any UI framework or renderer and is intended to provide the reactive foundation for UI rendering and other reactive systems.

The runtime separates state mutation from reactive execution. Signal writes update stored values and invalidate dependents. Pending reactive work is processed through `Scope::flush()`, while scopes are rerun explicitly through `Scope::run()`.

> **Development status:** The runtime is under implementation. This document describes the intended API, semantics, invariants, and implementation plan. Features should be considered implemented only when their code and tests are in place.

## Example

The repository includes a runnable example:

```sh
cargo run -p signals-example
```

The example demonstrates signals, memos, effects, persistent scopes, tracked and untracked reads, and deferred reactive work.

```rust
use signal::{effect, memo, scope, signal};

fn main() {
    let mut app = scope(|| {
        let price = signal(25.0);
        let quantity = signal(2);

        let total = memo(|| price.value() * quantity.value() as f64);

        effect(|| {
            println!("Price: €{:.2}", price.value());
        });

        println!("Cart total: €{:.2}", total);
        println!("Price without tracking: €{:.2}", price.inspect());

        (price, quantity)
    });

    let (price, quantity) = app.run();

    println!("--- Initial cart ---");
    app.flush();

    println!("--- Change quantity ---");
    quantity.set(3);
    app.flush();

    println!("--- Change price ---");
    price.set(30.0);
    app.flush();

    let (price, quantity) = app.run();

    println!("--- Read current state ---");
    println!("Price: €{:.2}", price.get());
    println!("Quantity: {}", quantity.get());
}
```

The memo retains its latest result internally and can recompute when its dependencies change. The returned value is not itself a signal or observable unless a signal is returned from the memo.

The cart total is printed by the scope closure. Consequently, it is printed when `app.run()` executes, not whenever `app.flush()` recomputes the memo. The price effect, by contrast, executes initially and again when its tracked price dependency changes.

The scope is persistent: calling `app.run()` again reuses resources at their corresponding positions instead of recreating them or resetting their current values.

## Public API

The crate exposes a small API based on free functions and resource handles.

| API | Responsibility |
|---|---|
| `signal(value)` | Creates or retrieves a signal in the active resource context. |
| `memo(f)` | Creates or retrieves a memo, returns its current result, and registers its output with the active observer when applicable. |
| `effect(f)` | Creates or retrieves an effect that tracks reactive reads. |
| `scope(f)` | Creates a deferred, persistent execution scope. |
| `get()` | Reads a signal and registers a dependency when tracking is active. |
| `value()` | Copies a signal value when `T: Copy`; behaves like `get()` for dependency tracking. |
| `inspect()` | Reads a signal without registering a dependency. |
| `inspect_value()` | Copies a signal value without registering a dependency when `T: Copy`. |
| `set(value)` | Replaces a signal's value and invalidates dependents. |
| `update(f)` | Computes a replacement value from the existing value and invalidates dependents. |
| `subscribe(f)` | Registers an explicit change-notification callback. |
| `run()` | Executes a scope and returns the result of its closure. |
| `flush()` | Processes queued reactive work. |
| `batch(f)` | Groups writes into a scheduling interval. |

Exact signatures, return types, disposal operations, and error behavior are defined by the implementation.

## Signals

A signal stores mutable state and acts as a source of reactive dependencies.

```rust
let count = signal(0);

count.set(1);
count.update(|value| value + 1);

assert_eq!(*count.inspect(), 2);
```

### Tracked and untracked reads

| Operation | Reads the value | Registers a dependency |
|---|---|---|
| `get()` | Yes | Yes, when a tracking context is active |
| `value()` | Yes, by copying | Yes, because it calls `get()` |
| `inspect()` | Yes | Never |
| `inspect_value()` | Yes, by copying | Never |

Dependency tracking depends on the currently active execution context. A signal read inside a memo computation establishes a dependency for that memo; the same read inside an effect establishes a dependency for that effect.

A read does not automatically register every enclosing scope or computation as a dependent. Nested tracking contexts must remain isolated.

### Writes

`set()` replaces the stored value. `update()` computes a replacement from the current value.

```rust
items.update(|mut items| {
    items.push(4);
    items
});
```

This ownership-based update pattern allows a collection to be modified without requiring a clone of the entire collection.

A successful write invalidates dependents according to the signal's change-detection policy. Queued work is not executed synchronously by the write itself.

An update should invalidate dependents at most once after the replacement has been committed. Mutating a value several times inside one update closure should not produce a separate notification for every mutation.

**Panic safety for `update()` is a hardening requirement.** The runtime must not lose the stored value or leave its storage in an invalid state if the update closure panics. See [Panic guards](#panic-guards).

### Subscriptions

A subscription registers an explicit callback for signal changes.

```rust
let count = signal(0);

let subscription = count.subscribe(|value| {
    println!("Count changed: {value}");
});

count.set(1);
```

Subscription callbacks follow the runtime's scheduling rules. A write must not unexpectedly execute queued callbacks synchronously.

The implementation must define when callbacks receive their values, how repeated writes are coalesced, and how subscriptions are removed. Unsubscription and disposal must be safe when notifications are already queued.

## Memos

A memo is a persistent reactive computation. The runtime retains its latest result, tracks its dependencies, and schedules recomputation when those dependencies change.

```rust
let count = signal(42);
let doubled = memo(|| count.value() * 2);

assert_eq!(doubled, 84);
```

The computation executes immediately when a memo is first created.

For an ordinary result type, `memo()` returns an ordinary Rust value. That value is a snapshot of the memo's result at the time of the call. It does not update itself when the memo recomputes.

### Dependency tracking

A memo tracks reactive sources read during its computation. Separately, calling `memo()` registers the memo's output with the currently active observer, when one exists.

These are distinct relationships:

- The memo depends on the signals and memos read by its computation.
- The active observer depends on the memo output when `memo()` is called in that observer's context.
- Capturing a previously returned ordinary value does not establish a dependency.
- When the memo recomputes, downstream observers are notified according to the memo's change-detection policy.

For example:

```rust
let doubled = memo(|| count.value() * 2);

effect(|| {
    println!("Doubled: {doubled}");
});
```

This effect captures an ordinary value. It does not subscribe to the memo output merely by capturing that value.

To establish an effect's dependency on a memo output under this API design, the memo must be called at a stable resource position during the effect's execution, or the output must be exposed through another supported reactive mechanism.

The updated example deliberately prints the cart total in the scope closure. It does not claim that the print statement is a reactive subscriber to the memo.

### Memo lifecycle

1. `memo()` creates or retrieves a memo in the active resource context.
2. On first creation, its computation executes immediately.
3. Reactive reads establish the memo's dependencies.
4. The runtime retains the latest computed result.
5. The memo output is registered with the active observer, when applicable.
6. An input change marks the memo dirty and queues recomputation.
7. `Scope::flush()` processes the dirty memo.
8. The memo compares its new result with the previous result according to its change policy.
9. If the output changed, downstream work is invalidated.
10. When a dependent executes again, it obtains the latest result through the supported API.

Dependencies must be replaced after recomputation. Signals that were read previously but are no longer read must no longer remain dependencies of the memo.

### Memos returning signals

A memo may return a signal to expose a derived value through a signal handle.

```rust
let count = signal(10);

let doubled = memo(|| signal(count.value() * 2));

effect(|| {
    println!("Doubled: {}", doubled.value());
});
```

This is a special runtime behavior: `doubled` is a signal handle, not an ordinary integer result.

The intended contract is that the memo maintains a stable, memo-owned output signal across recomputations. When the memo recomputes, it updates that persistent output's value rather than replacing its identity with a newly created candidate signal.

The runtime must ensure that:

- Clones of the output handle continue to refer to the same signal.
- Updating the output follows ordinary signal invalidation and scheduling rules.
- Candidate signals do not accidentally replace or adopt unrelated signal storage.
- Returning the memo's own output cannot create a recursive update loop.
- Input dependencies and output subscribers remain separate.
- Disposal of the memo or its owning scope cleans up the output consistently.

This behavior must be tested explicitly. It is not a general rule that every signal returned by every computation is automatically adopted or synchronized.

## Effects

An effect runs a closure for its side effects and tracks the reactive sources read during execution.

```rust
let app = scope(|| {
    let count = signal(0);

    let _handle = effect(|| {
        println!("Count: {}", count.value());
    });

    count
});

let count = app.run();

count.set(1);
app.flush();
```

An effect executes immediately when first created. Subsequent executions are deferred until pending work is processed by `Scope::flush()`.

### Effect lifecycle

1. `effect()` creates or retrieves an effect.
2. On first creation, the closure executes immediately.
3. Reads during the closure establish dependencies for the effect.
4. Changes to those dependencies invalidate and queue the effect.
5. The scheduler executes the queued effect during a flush.
6. The effect's dependencies are updated to match its latest execution.

A pending effect should not appear more than once in the queue. If an effect is invalidated again after it has executed, it can be queued for another run.

### Dynamic dependencies

Conditional reads are supported. An effect's dependencies must reflect the signals actually read during its most recent execution.

```rust
let _handle = effect(|| {
    if enabled.value() {
        println!("Count: {}", count.value());
    }
});
```

When `enabled` is false, the effect does not read `count`, so `count` should not remain a dependency of that execution. If `enabled` later becomes true, the next execution reads `count` and establishes the dependency again.

Dynamic dependencies do not remove the positional resource-ordering requirements for resource-creating calls inside an effect.

## Scopes

A scope is a deferred, persistent execution context for reactive resources.

```rust
let app = scope(|| {
    let count = signal(42);
    let doubled = memo(|| count.value() * 2);

    (count, doubled)
});

let (count, doubled) = app.run();
```

Creating a scope does not execute its closure. The first `run()` initializes the scope's resources, establishes dependencies, and returns the closure's result.

Subsequent `run()` calls execute the closure again while reusing resources at corresponding positions. The returned value comes from the latest execution.

### Positional resource reuse

The runtime associates resource-creating calls with positions in each execution context. The order must remain stable between executions.

For example, a later call to `signal(42)` at the same position retrieves the existing signal. It must not reset the signal's current value to `42`.

The same principle applies to memos and effects: rerunning the scope must not create duplicate resources when the corresponding resource already exists at that position.

Conditional resource creation that shifts the sequence is invalid:

```rust
let app = scope(|| {
    let count = signal(0);

    if count.inspect() > 10 {
        let extra = signal(1);
    }

    let last = signal(30);

    (count, last)
});
```

When the condition changes, `last` can occupy a different resource position. The runtime cannot reliably infer the intended identity of resources that have shifted.

Keep conditional reactive logic inside a memo or effect whose own resource position is stable. Resource-creating calls must still obey the ordering rules of their own execution context.

The runtime should validate resource kinds, types, and resource counts when reusing cached positions. Such checks can detect common mistakes, but they cannot detect every semantic swap between resources of the same type.

### Scope dependencies and reruns

A scope tracks dependencies relevant to its own execution result, including direct tracked reads and memo outputs registered while the scope is active.

A signal read inside a memo belongs to the memo's computation. It is not automatically attributed to the enclosing scope.

When a relevant dependency changes, the scope can be notified that its result may need refreshing. **That notification does not automatically rerun the scope closure.** The caller or UI runtime decides when to call `run()` again.

This separation lets a UI runtime control when a component or scope is reevaluated.

### Flushing

The runtime separates invalidation from execution:

- `set()` and `update()` change stored state and invalidate dependents.
- Dirty memos are queued for recomputation.
- Effects and subscription callbacks are queued for execution.
- `flush()` processes pending reactive work according to scheduler rules.
- A scope that has been notified can be rerun explicitly with `run()`.

Dirty memos must recompute before their output subscribers are notified about the new result. Work queued during a flush must follow a defined queue-draining policy.

## Batching

`batch()` groups multiple writes into one scheduling interval.

```rust
batch(|| {
    first.set(10);
    second.set(20);
});

app.flush();
```

Nested batches defer scheduling until the outermost batch completes. Multiple writes can therefore invalidate dependents without executing queued work between every write.

Batching does not change the immediate initial-execution behavior of newly created memos and effects.

The scheduler must deduplicate pending work, including notifications caused by repeated writes to the same source during a batch.

## Runtime architecture

The implementation should keep the following responsibilities distinct.

### Signal storage

Owns signal values and supports tracked reads, untracked reads, writes, invalidation, and subscriptions.

### Execution context

Maintains the active scope, memo, or effect context. It manages nested execution and dependency tracking.

### Scope runtime

Stores the scope closure, cached resources, positional resource cursor, dependency registrations, execution state, and scope subscriptions.

### Memo runtime

Stores the computation, latest result, dirty state, dependencies, change policy, and output subscribers. Signal-valued memos additionally retain a stable output signal slot.

### Effect runtime

Stores effect closures, dependencies, dirty and queued state, and disposal information.

### Dependency graph

Tracks relationships between signals, memo outputs, effects, scopes, and explicit subscribers. It removes obsolete dependency edges when computations rerun or resources are disposed.

### Scheduler

Queues invalidated computations and notification callbacks, deduplicates pending work, and processes queued work through `Scope::flush()`.

### Disposal

Deactivates resources, removes dependency registrations, prevents disposed work from executing, and ensures stale handles cannot access disposed or recycled resources.

## Disposal and ownership

Resource ownership is distinct from the lifetime of a Rust handle referring to a resource.

The runtime must define and consistently implement:

- Scope disposal and nested scope ownership.
- Effect and subscription lifetimes.
- Disposal of a dependent without disposing its sources.
- Disposal of a source with active dependents.
- Disposal while work is queued.
- Cleanup of resources no longer used by an execution.
- Memo-owned output signal lifetime.
- Stale handles and resource-slot reuse.

Disposal should be idempotent. Queued work belonging to disposed resources must not execute.

Ownership relationships must avoid reference cycles. `Rc`/`Weak` or centrally managed resource identifiers may be appropriate for a single-threaded runtime, depending on the implementation.

A disposed scope must not silently resume normal resource creation, and an old handle must not access a different resource merely because that resource reused the same slot.

## Change detection

Signals and memos have separate change-detection responsibilities.

### Signals

The default signal policy can treat every successful write as a change without requiring `PartialEq` for the stored value.

If equality-aware invalidation is needed, it should be introduced through an explicit comparator or a separate API.

### Memos

A memo's change policy determines whether its output subscribers are notified after recomputation. Potential policies include always notifying, comparing with `PartialEq`, or using an explicit comparator.

The implementation must define the chosen policy without relying on Rust specialization.

For floating-point values, ordinary `PartialEq` semantics mean that `NaN` compares unequal to itself. Equality-based memo comparison can therefore treat a recomputed `NaN` as changed even when the previous result was also `NaN`.

For signal-valued memos, the candidate's value is applied to the persistent output signal using the output signal's change policy. The output's identity remains stable.

## Error handling and runtime safety

The runtime must define behavior for:

- Panics during scope, memo, effect, subscriber, and update-closure execution.
- Nested or reentrant flush requests.
- Reactive cycles and work-limit exhaustion.
- `RefCell` borrow conflicts.
- Disposal during a flush.
- Resource kind, type, and count mismatches.
- Stale resource handles.
- Partial execution failures and dependency cleanup.
- A memo returning its own output signal.
- A memo returning a signal owned by another scope or computation.
- Failure while extracting or applying a candidate signal result.

A failure must not leave the runtime's active context, scheduler state, dependency graph, or stored values in an invalid state.

The runtime must define whether failed computations retain their last successful result, how they become eligible for retry, and how pending notifications are handled. A partially completed execution must not silently be treated as a successful one.

## Implementation plan

The implementation should proceed in phases so that the reactive contract can be tested before adding more complex safety mechanisms.

### Phase 1: Core storage and execution context

- Implement signal storage and stable signal handles.
- Implement tracked and untracked reads.
- Implement writes and invalidation.
- Implement active execution contexts and dependency registration.
- Establish resource ownership and positional resource reuse.
- Add focused tests for signal reads, writes, and dependency isolation.

### Phase 2: Memos

- Implement memo creation and positional reuse.
- Track and replace memo dependencies.
- Retain the latest computed result.
- Register memo outputs with the active observer.
- Implement memo change detection and downstream invalidation.
- Implement stable output identity for signal-valued memos.
- Test memo recomputation, dependency switching, output notification, and signal-valued memo behavior.

### Phase 3: Effects

- Implement immediate initial execution.
- Implement dependency tracking and replacement on rerun.
- Queue subsequent executions.
- Deduplicate pending effect work.
- Test conditional dependencies and deferred execution.

### Phase 4: Scopes

- Implement deferred scope creation and explicit `run()`.
- Implement positional resource reuse and resource-cursor reset.
- Implement scope
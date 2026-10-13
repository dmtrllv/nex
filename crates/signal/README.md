# Signals

A generic, single-threaded reactive state-management crate written in Rust.

The crate provides signals, lazy memos, effects, persistent scopes, subscriptions, batching, and a reactive scheduler. It is independent of any UI framework or renderer and is intended to provide the reactive foundation for UI rendering and other reactive systems.

The runtime separates state mutation, invalidation, computation, and side-effect execution. Signals store mutable state, memos cache derived values, effects react to tracked reads, and scopes manage persistent reactive resources.

> **Development status:** The runtime is under implementation. This document describes the intended API and semantics. A feature should be considered implemented only when its implementation and tests are in place.

## Example

Run the example from the workspace root:

```sh
cargo run -p signals-example
```

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

        effect(|| {
            println!("Cart total: €{:.2}", total.value());
        });

        println!("Price without tracking: €{:.2}", price.inspect());

        (price, quantity)
    });

    // Initial execution creates the resources.
    let (price, quantity) = app.run();

    println!("--- Initial cart ---");
    app.flush();

    println!("--- Change quantity ---");
    quantity.set(3);
    app.flush();

    println!("--- Change price ---");
    price.set(30.0);
    app.flush();

    // Re-execution reuses the existing resources.
    let (price, quantity) = app.run();

    println!("--- Read current state ---");
    println!("Price: €{:.2}", price.get());
    println!("Quantity: {}", quantity.get());
}
```

The example illustrates the main distinctions:

- `Signal<T>` stores mutable state.
- `Memo<T>` represents a cached derived value.
- `get()` and `value()` track reactive dependencies.
- `inspect()` and `inspect_value()` read without tracking.
- `Scope::run()` explicitly executes the scope closure.
- `Scope::flush()` processes queued reactive work.

The memo is read through its handle. It does not return a snapshot that callers must capture and replace manually.

## Public API

The crate exposes free functions that create or retrieve reactive resources in the active execution context.

| API | Responsibility |
|---|---|
| `signal(value)` | Creates or retrieves a mutable reactive signal. |
| `memo(f)` | Creates or retrieves a lazy, cached derived computation. |
| `effect(f)` | Creates or retrieves a reactive side effect. |
| `scope(f)` | Creates a deferred, persistent execution scope. |
| `get()` | Reads a value and tracks a dependency when tracking is active. |
| `inspect()` | Reads a value without registering a dependency. |
| `value()` | Copies a value when `T: Copy` and tracks a dependency. |
| `inspect_value()` | Copies a value when `T: Copy` without tracking. |
| `set(value)` | Replaces a signal's value. |
| `update(f)` | Computes a replacement signal value from its current value. |
| `subscribe(f)` | Registers an explicit signal change callback. |
| `run()` | Executes a scope closure and returns its result. |
| `flush()` | Processes queued reactive work. |
| `batch(f)` | Groups writes into a scheduling interval. |
| `unsubscribe()` | Removes an explicit subscription. |

`Signal<T>` and `Memo<T>` are handles to reactive values. They are intended to share a read-only interface, while mutation operations remain specific to signals.

Exact trait definitions, return types, disposal behavior, and scheduling details are governed by the implementation.

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
| `get()` | Yes | Yes, when tracking is active |
| `value()` | Yes, by copying | Yes |
| `inspect()` | Yes | No |
| `inspect_value()` | Yes, by copying | No |

Dependency tracking is determined by the active execution context. A read inside a memo computation establishes a dependency for that memo; a read inside an effect establishes a dependency for that effect.

Untracked reads do not create dependency edges. They are useful when a computation needs to inspect a value without reacting to subsequent changes.

### Writes

`set()` replaces the current value. `update()` computes a replacement from the current value.

```rust
items.update(|mut items| {
    items.push(4);
    items
});
```

This ownership-based update pattern allows collections to be modified without cloning the entire collection.

A successful write invalidates dependents according to the signal's change-detection policy. It does not directly execute queued effects or subscription callbacks.

An update should commit its replacement before notifying dependents. Multiple mutations performed inside one update closure should not produce separate notifications for each mutation.

**Panic safety is a requirement.** If an update closure panics, the runtime must not lose the stored value or leave signal storage in an invalid state. See [Panic guards](#panic-guards).

### Subscriptions

A subscription registers an explicit callback for signal changes.

```rust
let count = signal(0);

let subscription = count.subscribe(|value| {
    println!("Count changed: {value}");
});

count.set(1);

// Explicitly remove the subscription when it is no longer needed.
subscription.unsubscribe();
```

Subscription callbacks follow the runtime's scheduling rules. A write must not unexpectedly execute queued callbacks synchronously.

The runtime must define callback timing, repeated-write coalescing, and behavior when a subscription is removed while a notification is pending.

## Memos

A memo is a cached reactive computation. It retains the latest computed result and tracks the reactive sources read by its computation.

```rust
let count = signal(21);

let doubled = memo(|| count.value() * 2);

assert_eq!(*doubled.get(), 42);
```

Unlike a function that recalculates its result on every call, a memo reuses its cached result while it remains valid.

`memo()` returns a `Memo<T>` handle. It does not return the computed `T` directly.

### Lazy evaluation

A memo evaluates its computation when its result is first requested. If an input later changes, the memo becomes dirty. It recomputes the next time a tracked or untracked read requires its value.

Conceptually, the lifecycle is:

1. `memo(f)` creates or retrieves the memo resource.
2. The first `get()`, `inspect()`, `value()`, or `inspect_value()` evaluates the computation if no cached result exists.
3. Reactive reads performed by the computation establish its input dependencies.
4. The computed result is cached.
5. A dependency change invalidates the memo.
6. The next read recomputes the result if the memo is dirty.
7. The cache is updated, and downstream invalidation follows the memo's change-detection policy.

Reading a clean memo does not rerun its computation.

`get()` and `value()` register the active reactive context as a dependent of the memo. `inspect()` and `inspect_value()` deliberately do not.

### Tracked memo reads

A memo can be used as a dependency of another memo or effect.

```rust
let count = signal(10);

let doubled = memo(|| count.value() * 2);

effect(|| {
    println!("Doubled: {}", doubled.value());
});
```

The effect tracks the memo output because it calls `doubled.value()` while executing.

When `count` changes, the memo becomes dirty. The reactive runtime must ensure that the effect can obtain the current memo result when it executes, rather than reading a stale cached value.

The distinction between a tracked read and an untracked read is significant:

```rust
effect(|| {
    println!("{}", doubled.value());
});
```

This effect depends on `doubled`.

```rust
effect(|| {
    println!("{}", doubled.inspect_value());
});
```

This effect does not depend on `doubled` through that read. It may still depend on other sources read by its closure.

### `get()` versus `inspect()`

Both methods return the current memo result, computing it first if necessary.

- `get()` tracks the memo as a dependency of the active reactive context.
- `inspect()` performs the same lazy read without adding a dependency.

The same distinction applies to `value()` and `inspect_value()` for `T: Copy`.

The read operation determines whether a dependency is registered; the fact that a read causes recomputation does not make it a tracked read.

### Dependency replacement

A memo's dependencies must reflect the sources read during its latest successful computation.

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

Initially, `selected` depends on `use_first` and `first`. After `use_first` becomes false and the memo recomputes, it should depend on `use_first` and `second`. It must no longer react to changes in `first`.

Dependency replacement must not leave obsolete subscriptions behind. Failed computations must not be treated as successful dependency updates.

### Memo change detection

Invalidation and value change are different concepts.

An input can change while the memo's computed result remains the same. For example, a memo that computes `count.value() > 0` can recompute when `count` changes from `1` to `2` while still producing `true`.

The runtime should distinguish:

- **Invalidation:** an input changed, so the cached result may be stale.
- **Recomputation:** the memo evaluates its closure to obtain a current result.
- **Output change:** the new result differs according to the memo's comparison policy.
- **Downstream notification:** dependent work is scheduled according to the output-change policy.

The comparison policy must be defined by the implementation. It should not silently require `T: PartialEq` for every memo unless that is an intentional API constraint. An explicit comparator or a separate equality-aware API are alternatives.

For floating-point values, ordinary `PartialEq` semantics mean that `NaN != NaN`. An equality-based memo can therefore treat a newly computed `NaN` as a change even if the previous result was also `NaN`.

### Read-only semantics

A memo's value is derived from its computation. Callers should not be able to replace it through `set()` or `update()`.

If the value needs to change, its input signals should change. The memo will then become dirty and compute a new result when read.

`Signal<T>` and `Memo<T>` should share the common read interface, but signal mutation and memo evaluation remain distinct responsibilities.

## Effects

An effect runs a closure for its side effects and tracks the reactive values read during execution.

```rust
let count = signal(0);

effect(|| {
    println!("Count: {}", count.value());
});
```

An effect executes when first created. Later executions are scheduled when its tracked dependencies change.

### Effect lifecycle

1. `effect(f)` creates or retrieves an effect resource.
2. The initial execution runs the closure.
3. Reactive reads establish the effect's dependencies.
4. Changes to those dependencies invalidate and queue the effect.
5. `flush()` processes queued effect work.
6. Each successful rerun replaces the effect's dependencies with those observed during the latest execution.

Pending work should be deduplicated so repeated invalidations do not add duplicate queue entries for the same pending effect.

### Dynamic dependencies

Conditional reads are supported.

```rust
let enabled = signal(false);
let count = signal(0);

effect(|| {
    if enabled.value() {
        println!("Count: {}", count.value());
    }
});
```

When `enabled` is false, the effect does not read `count`. After that execution, `count` should not remain a dependency of the effect. If `enabled` becomes true, the next execution reads `count` and establishes the dependency again.

Dynamic dependencies do not remove the resource-ordering requirements for resource-creating calls.

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

Subsequent `run()` calls execute the closure again while reusing resources at corresponding positions. Existing signal values are preserved; rerunning the scope must not reset them to their initial values.

### Positional resource reuse

The runtime associates resource-creating calls with positions in each execution context. The order of these calls must remain stable between executions.

For example, a later call to `signal(42)` at the same position retrieves the existing signal rather than resetting its value to `42`. The same principle applies to memos and effects.

Conditional resource creation that shifts the sequence is invalid:

```rust
let app = scope(|| {
    let count = signal(0);

    if count.inspect_value() > 10 {
        let extra = signal(1);
    }

    let last = signal(30);

    (count, last)
});
```

When the condition changes, `last` may occupy a different resource position. The runtime cannot reliably infer the intended identity of resources that have shifted.

Keep conditional reactive logic inside a memo or effect whose own resource position is stable. Resource-creating calls must still obey the ordering rules of their execution context.

The runtime should validate resource kinds, types, and resource counts when reusing cached positions. These checks can detect common mistakes but cannot detect every semantic swap between resources of the same type.

### Scope dependencies

A scope tracks dependencies relevant to its execution result, including direct tracked reads and tracked memo reads performed while the scope is active.

A signal read inside a memo belongs to the memo's computation. It must not automatically be registered as a direct dependency of the enclosing scope.

When a relevant dependency changes, the scope can be marked as needing reevaluation. This notification does not automatically rerun the scope closure. The caller or UI runtime decides when to call `run()` again.

This separation lets a UI runtime decide when a component or scope should be reevaluated.

### Flushing

The runtime separates state changes from reactive execution:

- `set()` and `update()` mutate signal state and invalidate dependents.
- Invalidated memos become dirty.
- Effects and subscription callbacks are queued according to the scheduler's rules.
- A read of a dirty memo recomputes it.
- `flush()` processes queued reactive work.
- A scope can be rerun explicitly with `run()`.

The scheduler must define how it handles work queued during a flush and how it prevents stale memo results from being observed by dependents.

Lazy memo evaluation does not mean that effects are lazy in the same way. Effects execute according to their scheduling rules; when an effect reads a dirty memo, that read must obtain the current result.

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

Batching does not change the initial-execution behavior of newly created effects. It also does not require a dirty memo to recompute until a read needs its result.

The scheduler must deduplicate pending work and define how invalidations propagate through chains of memos and effects.

## Runtime architecture

The implementation should keep the following responsibilities distinct.

### Signal storage

Owns signal values and supports tracked reads, untracked reads, writes, invalidation, and subscriptions.

### Execution context

Maintains the active scope, memo, or effect context. It manages nested execution and dependency tracking.

### Scope runtime

Stores the scope closure, cached resources, positional resource cursor, dependency registrations, and execution state.

### Memo runtime

Stores the memo computation, cached result, dirty state, dependencies, comparison policy, and downstream subscribers. Reading a dirty memo evaluates its computation and updates its cache.

### Effect runtime

Stores effect closures, dependencies, dirty and queued state, and lifecycle information.

### Dependency graph

Tracks relationships between signals, memos, effects, scopes, and explicit subscribers. It removes obsolete dependency edges when computations rerun or resources are disposed.

### Scheduler

Queues invalidated effects and notification callbacks, deduplicates pending work, and processes queued work through `Scope::flush()`.

The scheduler and memo runtime must cooperate so a dependent does not consume a stale memo result.

### Shared read interface

`Signal<T>` and `Memo<T>` should implement a common read-only trait for operations that are genuinely shared. The trait should express tracked reads, untracked reads, and, where practical, the copy-based convenience methods.

Mutation operations remain specific to `Signal<T>`. The trait should not force a memo to expose operations that have no valid meaning for derived state.

The precise trait shape depends on the final read return types and storage model. In particular, returning `&T` from interior-mutable storage requires a sound lifetime strategy; the implementation must not return references that outlive a temporary borrow.

### Disposal

Deactivates resources, removes dependency registrations, prevents disposed work from executing, and ensures stale handles cannot access disposed or recycled resources.

## Disposal and ownership

Resource ownership is distinct from the lifetime of a Rust handle referring to a resource.

The runtime must define and consistently implement:

- Scope and nested-resource ownership.
- Effect and subscription lifetimes.
- Disposal of a dependent without disposing its sources.
- Disposal of a source with active dependents.
- Disposal while work is queued.
- Cleanup of resources no longer used by an execution.
- Stale handles and resource-slot reuse.

Disposal should be idempotent. Queued work belonging to disposed resources must not execute.

Ownership relationships must avoid reference cycles. `Rc`/`Weak` or centrally managed resource identifiers may be appropriate for a single-threaded runtime, depending on the implementation.

A disposed scope must not silently resume normal resource creation, and an old handle must not access a different resource merely because that resource reused the same slot.

## Error handling and runtime safety

The runtime must define behavior for:

- Panics during scope, memo, effect, subscription, and update-closure execution.
- Nested or reentrant flush requests.
- Reactive cycles and work-limit exhaustion.
- `RefCell` borrow conflicts.
- Disposal during a flush.
- Resource kind, type, and count mismatches.
- Stale resource handles.
- Partial execution failures and dependency cleanup.
- Memo computations that repeatedly invalidate their own dependency chain.
- A failed memo recomputation and its cached result.
- A panic during dependency replacement.
- A panic while processing queued work.

A failure must not leave the runtime's active context, scheduler state, dependency graph, or stored values in an invalid state.

The runtime must define whether failed computations retain their last successful result, how they become eligible for retry, and how pending notifications are handled. A partially completed execution must not silently be treated as successful.

## Implementation plan

The implementation should proceed in phases so the reactive contract can be tested before adding more complex safety mechanisms.

### Phase 1: Core storage and execution context

- Implement signal storage and stable signal handles.
- Implement tracked and untracked reads.
- Implement writes and invalidation.
- Implement active execution contexts and dependency registration.
- Establish resource ownership and positional resource reuse.
- Add focused tests for signal reads, writes, and dependency isolation.

### Phase 2: Lazy memos

- Implement `Memo<T>` creation and positional reuse.
- Make initial evaluation lazy.
- Implement cached results and dirty-state tracking.
- Implement tracked and untracked memo reads.
- Track and replace memo dependencies.
- Define memo output change detection and downstream invalidation.
- Test initial laziness, cached reads, invalidation, recomputation, and dynamic dependencies.

### Phase 3: Effects

- Implement immediate initial execution.
- Implement dependency tracking and replacement on rerun.
- Queue subsequent executions.
- Deduplicate pending effect work.
- Test conditional dependencies and deferred execution.

### Phase 4: Scopes

- Implement deferred scope creation and explicit `run()`.
- Implement positional resource reuse and resource-cursor reset.
- Implement scope dependency tracking and reevaluation notification.
- Test repeated runs, stable resource identity, and invalid resource ordering.

### Phase 5: Scheduler and batching

- Implement queueing and deduplication.
- Implement nested batching.
- Define ordering between invalidation, memo reads, effect execution, and subscription notifications.
- Test chained memos, multiple writes, nested batches, and work queued during a flush.

### Phase 6: Subscriptions and disposal

- Implement subscription creation and unsubscription.
- Define callback timing and repeated-write behavior.
- Implement resource cleanup and disposal.
- Test disposal with queued work and stale handles.

### Phase 7: Integration and behavior tests

- Verify the public API against the documented examples.
- Test the interactions between signals, memos, effects, scopes, and batching.
- Test dynamic dependency replacement and lazy memo recomputation.
- Check that ordinary reads and untracked reads have consistent semantics.
- Verify that the runtime preserves resource identity across scope runs.

### Phase 8: Panic guards

Implement panic-safety guards **after the core runtime is working**.

Use RAII guards to restore runtime state when unwinding through a panic. Areas to cover include:

- Active execution-context restoration.
- Scheduler and flush state.
- Signal update ownership transfer.
- Memo recomputation and cache commit.
- Dependency replacement.
- Scope execution state and resource cursors.
- Temporary borrows and ownership transfer.
- Queued work and disposal state.

Do not indiscriminately catch and suppress panics. The objective is to leave the runtime in a valid, recoverable state while allowing the original panic to propagate unless the API explicitly defines a different policy.

Add panic-injection tests to verify that a panic does not corrupt storage, leave a context active, lose dependencies, or prevent later independent work from executing.

### Phase 9: Public API and documentation review

- Finalize the common read-only trait for `Signal<T>` and `Memo<T>`.
- Verify that mutation methods are exposed only on signals.
- Review the lifetime and ownership semantics of reference-returning reads.
- Confirm the lazy evaluation contract and scheduler ordering.
- Ensure examples reflect tracked and untracked behavior.
- Update this README whenever implementation semantics change.

## Design principles

1. **Explicit reactivity.** Tracked reads establish dependencies; untracked reads do not.
2. **Lazy derived values.** Memos cache results and recompute when a dirty value is read.
3. **Stable resource identity.** Repeated scope executions reuse resources at corresponding positions.
4. **Separated responsibilities.** Signals own mutable state, memos derive values, effects perform side effects, and scopes manage persistent execution.
5. **Deterministic scheduling.** State mutation and queued reactive execution remain separate operations.
6. **Renderer independence.** The crate has no dependency on a particular UI framework or rendering backend.
7. **Panic-safe runtime state.** Failures must not leave the reactive graph or execution context corrupted.

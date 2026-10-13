# Signals

A generic, single-threaded reactive state-management crate written in Rust.

The crate provides signals, memos, effects, persistent scopes, subscriptions, and a deterministic scheduler. It is independent of any UI framework or renderer and can serve as the reactive foundation for component rendering, state management, and other reactive systems.

The runtime separates state mutation from reactive execution. Signal writes update values and invalidate dependents, while `Scope::flush()` processes pending reactive work. Scopes preserve resources across executions using positional hook ordering.

## Design goals

- Provide a small public API based on free functions.
- Track reactive dependencies automatically through `get()`.
- Support untracked reads through `inspect()`.
- Defer subsequent reactive work until `Scope::flush()` processes pending work.
- Recompute memos when their dependencies change.
- Notify dependents when a memo's output changes.
- Support memos that return signals by maintaining a stable, memo-owned output signal.
- Preserve signals, memos, and effects across repeated scope executions.
- Support conditional reactive logic without allowing conditional resource creation to corrupt positional resource slots.
- Manage reactive resources through scopes and deterministic disposal.
- Support nested scopes and computations without losing the active execution context.
- Support batching, subscriptions, and deterministic scheduling.
- Remain single-threaded and independent of a particular UI framework or renderer.

## Public API

Reactive resources are created through free functions. The active execution context determines resource ownership, resource reuse, and dependency tracking.

```rust
use signal::{effect, memo, scope, signal};

let count = signal(10);

let doubled = memo(|| count.get() * 2);

let handle = effect(|| {
    println!("Current value: {}", count.get());
});

let app = scope(|| {
    let local = signal(42);
    local
});

// The scope closure executes when run() is called.
let local = app.run();
```

The principal API is organized around the following operations:

| API | Responsibility |
|---|---|
| `signal(value)` | Creates or retrieves a signal in the active resource context. |
| `memo(f)` | Creates or retrieves a memo, returns its current computed result, and registers the memo output with the active observer when applicable. |
| `effect(f)` | Creates or retrieves a reactive side effect. |
| `scope(f)` | Creates a deferred, persistent scope. |
| `get()` | Reads a signal and registers a dependency when a tracking context is active. |
| `inspect()` | Reads a signal without registering a dependency. |
| `set(value)` | Replaces a signal's stored value and invalidates dependents. |
| `update(f)` | Mutates a signal's stored value and invalidates dependents. |
| `subscribe(f)` | Registers an explicit change-notification callback. |
| `run()` | Executes a scope and returns the result of its closure. |
| `flush()` | Processes queued reactive work through the scope API. |
| `batch(f)` | Groups writes into a scheduling interval. |

Resource handles and subscriptions may expose additional lifecycle operations, including disposal. The exact signatures and return types are defined by the crate's public API.

## Signals

A signal stores mutable state and acts as a source of reactive dependencies.

```rust
let count = signal(0);

assert_eq!(*count.inspect(), 0);

count.set(1);
count.update(|value| *value += 1);

assert_eq!(*count.inspect(), 2);
```

### Reads

Signals support tracked and untracked reads.

| Operation | Reads the value | Registers a dependency |
|---|---|---|
| `get()` | Yes | Yes, when a tracking context is active |
| `inspect()` | Yes | No |

`get()` reads the value and registers the active observer as a dependent of the signal when a tracking context exists. Outside a tracking context, it simply reads the value.

`inspect()` always performs an untracked read, regardless of the active execution context.

Dependency tracking is determined by the runtime's active execution context. The signal does not need to know whether it is being read by a scope, memo, or effect.

### Writes

`set(value)` replaces the stored value. `update(f)` mutates the stored value through a closure.

Both operations invalidate dependents and schedule the necessary reactive work. They do not synchronously execute queued effects, recompute dirty memos, or invoke queued subscription callbacks.

```rust
let app = scope(|| {
    let count = signal(0);
    count
});

let count = app.run();

count.set(10);

// The stored value is already 10.
// Dependent reactive work is pending.
app.flush();
```

Signal writes do not inherently require `PartialEq`. The signal's change policy determines whether a write invalidates dependents; the default policy can treat every successful write as a change.

An `update()` operation should invalidate dependents at most once for the completed mutation, rather than once for every mutation performed inside its closure.

## Memos

A memo is a persistent reactive computation. The runtime retains its latest result, tracks its dependencies, and schedules recomputation when those dependencies change.

```rust
let count = signal(42);

let doubled = memo(|| count.get() * 2);

assert_eq!(doubled, 84);
```

The memo computation executes immediately when the memo is first created. Reads through `get()` establish its dependencies.

For an ordinary return type, `memo()` returns the computed result as an ordinary Rust value, not as a public memo handle. The returned value is a snapshot; it does not change when the memo recomputes.

**Calling `memo()` also registers the memo's output with the currently active observer.** This allows a scope, memo, or effect to depend on the memo's output without exposing a `Memo::get()` or `Memo::observe()` API.

When the memo's output changes, its dependents are notified. A dependent scope can be marked as needing another execution, while a dependent effect or memo can be invalidated for reevaluation. When the dependent executes again, it calls `memo()` and receives the latest computed result.

### Memo lifecycle

1. `memo()` creates or retrieves the memo in the active resource context.
2. On first creation, the memo's computation executes immediately.
3. Reads through `signal.get()` and calls to other memos establish dependencies for the memo's computation.
4. The runtime retains the computed result internally.
5. The memo output is registered with the active observer, when one exists.
6. When an input dependency changes, the memo is marked dirty and queued for recomputation.
7. During `Scope::flush()`, the dirty memo recomputes.
8. The runtime compares the new result with the previous result according to the memo's change policy.
9. If the result changed, the runtime notifies output subscribers and invalidates downstream work.
10. If the result did not change, downstream subscribers are not notified solely because the memo recomputed.

A memo must replace its dependency registrations after recomputation. If its computation begins reading a different set of signals, obsolete dependencies must be removed and new ones established.

### Memo output dependencies

The memo's ordinary return value does not itself carry reactive behavior. Instead, calling `memo()` registers the memo's output with the currently active observer.

This allows a scope, effect, or other supported observer to depend on a memo output without requiring the caller to invoke a public memo-handle method.

For example, a memo called while a scope is executing can register its output as a dependency of that scope. When the memo's output changes, the scope can be notified that its result may need to be refreshed.

The same principle applies inside an effect: calling a memo at a stable resource position within the effect's execution can register that memo output as a dependency of the effect.

The distinction is important:

- The memo's computation tracks the signals and other reactive sources it reads.
- The active observer tracks the memo output when `memo()` is called.
- The ordinary value returned by `memo()` is a snapshot, not a subscription.
- Capturing a previously returned value in a closure does not establish a new dependency.
- When a memo output changes, its subscribers can be invalidated even though previously returned ordinary values remain unchanged.

For example, this effect captures an ordinary value and does not subscribe to the memo output merely by capturing it:

```rust
let doubled = memo(|| count.get() * 2);

let _handle = effect(|| {
    println!("Doubled: {doubled}");
});
```

If the effect needs to react to a memo output, the memo must be called in the effect's active execution context, or another supported dependency mechanism must establish that relationship. A previously captured snapshot is not automatically refreshed.

### Memo returning a signal

A memo may return a signal when the computation needs to expose its derived result through a signal handle.

```rust
let count = signal(10);

let doubled = memo(|| signal(count.get() * 2));

let _count_effect = effect(|| {
    println!("Count: {}", count.get());
});

let _doubled_effect = effect(|| {
    println!("Doubled: {}", doubled.get());
});
```

In this example, `doubled` is a `Signal<i32>`, not an ordinary `i32`. The memo has a special output contract because the returned value is a signal.

The runtime maintains a **stable, memo-owned output signal** across recomputations.

The intended behavior is:

1. The memo executes initially and creates its output signal with the value `20`.
2. The memo retains ownership of that output signal's storage slot.
3. The returned `doubled` handle refers to the retained slot.
4. When `count` changes, the memo becomes dirty.
5. During `Scope::flush()`, the memo recomputes and produces a candidate signal result.
6. The runtime extracts the candidate's value and updates the existing output slot rather than replacing it with the candidate's identity.
7. Existing clones of `doubled` continue to refer to the same signal.
8. If the output value changes according to the signal's change policy, its dependents are invalidated and queued through the normal scheduler.
9. If the output value is considered unchanged, the runtime does not notify dependents solely because the memo recomputed.

For example, after `count` changes from `10` to `15` and pending work is flushed, the existing `doubled` handle observes `30`. It does not become a handle to a different signal.

This behavior preserves signal identity while allowing a memo to compute the signal's value.

#### Output ownership rules

- The output signal is created during the memo's initial execution.
- The memo owns the persistent output slot.
- Subsequent candidate signals are temporary computation results; their identities do not replace the persistent output identity.
- The runtime must not adopt an arbitrary signal owned by another scope or computation as the memo's output.
- Returning the memo's own output signal must be handled explicitly to prevent self-aliasing and feedback problems.
- The persistent output is updated using the ordinary signal change-detection, invalidation, batching, and scheduling mechanisms.
- The memo's input dependency set must remain separate from the output signal's subscribers.
- Disposing the memo or its owning scope must clean up its output according to the ownership and lifetime rules.

A signal-valued memo is a specific runtime behavior, not a general rule that all signals returned by arbitrary computations are automatically adopted or synchronized.

#### Memo change detection

The memo's change policy determines whether subscribers are notified after recomputation.

Possible policies include:

- Treat every recomputation as a change.
- Compare the new and previous results using `PartialEq`.
- Use an explicit comparator.

The selected policy must be defined by the implementation rather than relying on Rust specialization.

For a signal-valued memo, the candidate's value is applied to the persistent output signal using that signal's change policy. The runtime must avoid duplicate or recursive notifications while updating the output.

If equality-based comparison uses ordinary floating-point `PartialEq`, `NaN` compares unequal to itself. Consequently, a recomputation yielding `NaN` may be considered a change even when the previous result was also `NaN`.

## Effects

An effect executes a closure for its side effects and tracks reactive dependencies read during execution.

```rust
let app = scope(|| {
    let count = signal(0);

    let _handle = effect(|| {
        println!("Count: {}", count.get());
    });

    count
});

let count = app.run();

count.set(1);
app.flush();
```

An effect executes immediately when first created. Subsequent executions are deferred until pending work is processed by `Scope::flush()`.

### Effect lifecycle

1. `effect()` creates or retrieves the effect in the active resource context.
2. On first creation, its closure executes immediately.
3. Reads through `get()` register dependencies with the effect.
4. When a dependency changes, the effect is marked dirty and queued.
5. During `Scope::flush()`, the effect executes again.
6. The effect's dependencies are replaced with those read during the latest execution.

Repeated invalidations should not create duplicate queue entries while the effect is already pending.

Effects must not execute synchronously from `set()` or `update()`. Initial execution is the exception: it occurs when the effect is first created during execution of its owning context.

### Dynamic dependencies and conditionals

Effects may contain conditionals because conditional logic does not inherently change the effect's own resource-creation sequence.

```rust
let app = scope(|| {
    let enabled = signal(false);
    let count = signal(0);

    let _handle = effect(|| {
        if enabled.get() {
            println!("Count: {}", count.get());
        }
    });

    (enabled, count)
});

let (enabled, count) = app.run();
```

When the effect reruns, its dependencies are updated to match the reads performed by that execution. If the condition is false, `count` is not read and is not a dependency of that execution. If the condition later becomes true, reading `count` establishes the dependency again.

The same principle applies to conditional computations inside memos.

Resource-creating calls inside a scope, memo, or effect must still follow the positional ordering rules described below.

## Scopes

A scope provides a persistent execution context for reactive resources. Unlike `memo()` and `effect()`, creating a scope does not execute its closure immediately.

```rust
let app = scope(|| {
    let count = signal(42);
    let doubled = memo(|| count.get() * 2);

    let _handle = effect(|| {
        println!("Count: {}", count.get());
    });

    assert_eq!(doubled, 84);

    count
});

// The scope has not executed yet.

let count = app.run();

// The scope has initialized and returned the signal.
```

The first `app.run()` executes the scope closure, initializes its resources, establishes dependencies, and returns the closure's result.

Subsequent calls to `app.run()` execute the closure again while reusing the resources associated with the corresponding positions in the resource sequence. Each call returns the result produced by that execution.

### Persistent resource reuse

Consider the following scope:

```rust
let app = scope(|| {
    let count = signal(42);
    let doubled = memo(|| count.get() * 2);

    let _handle = effect(|| {
        println!("Count: {}", count.get());
    });

    (count, doubled)
});
```

The first execution creates the signal, memo, and effect.

A subsequent execution reuses the resources at those positions:

- `signal(42)` returns the existing signal without resetting its current value to `42`.
- `memo(...)` retrieves the existing memo and returns its current result.
- `effect(...)` retrieves the existing effect rather than registering a duplicate effect.
- The scope closure produces a new return value from the current execution.

The resource initializer arguments and closures may be evaluated again, but the runtime resources themselves persist.

Each execution context maintains its own positional resource sequence. Its resource cursor is reset at the start of each execution, allowing resource-creating calls to retrieve their corresponding cached resources.

This model is similar to React hooks: **the order of resource-creating calls must remain stable between executions.**

### Hook ordering and conditional logic

Positional resource reuse requires a consistent sequence of resource-creating calls. The runtime cannot safely infer the intended identity of a resource if an earlier conditional call is skipped.

For example, this is invalid:

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

When the condition changes, the `last` signal can occupy a different resource position. The runtime may detect a resource-count or resource-type mismatch, but it cannot reliably infer that the developer intended to preserve the identity of `last`.

Instead, keep conditional logic inside a memo or effect:

```rust
let app = scope(|| {
    let count = signal(0);

    let extra = memo(|| {
        if count.get() > 10 {
            1
        } else {
            0
        }
    });

    (count, extra)
});
```

The memo is always created at the same position. Its internal computation can change behavior and dependencies without changing the parent scope's resource sequence.

The same rule applies to loops: do not create a variable number of resources in a scope's resource sequence. A loop inside a memo or effect can change which computation or side effect occurs, but resource-creating calls inside that loop must still obey the positional ordering rules of their own execution context.

The runtime should validate resource kind and type when reusing a slot and detect resource-count mismatches when an execution completes. These checks catch common mistakes but cannot detect every semantic swap between resources of the same kind and type.

### Scope dependencies

A scope tracks reactive dependencies relevant to its own execution result. These include signals read directly by the scope through `get()` and memo outputs registered with the scope when `memo()` is called during its execution.

Reads isolated inside a memo's or effect's computation belong to that computation's dependency context; they are not automatically attributed to the parent scope.

When a memo output changes, the scope can be notified and marked as needing another execution. A scope notification does not automatically rerun the scope. The caller decides when to call `run()` again.

The scope closure is rerun only through an explicit execution request. Changes to dependencies do not implicitly execute the scope closure.

### Scope subscriptions

A scope can expose an explicit subscription mechanism for notifications that its relevant reactive dependencies have changed.

```rust
let app = scope(|| {
    let count = signal(42);
    let doubled = memo(|| count.get() * 2);

    (count, doubled)
});

let (count, _) = app.run();

let _subscription = app.subscribe(|| {
    println!("A scope dependency changed.");
});

count.set(2);

// The signal value changes immediately.
// Queued reactive work remains pending.
app.flush();
```

The memo initially returns `84`. After `count.set(2)`, the memo is invalidated and recomputes to `4` during `app.flush()`. If the output changes, the scope is notified through its memo-output dependency.

The scope subscription callback runs as part of the scheduler's notification work. It does not automatically rerun the scope closure; a caller or UI runtime can decide when to execute the scope again.

Subscription callbacks are explicit notification mechanisms. Reads performed inside such callbacks do not automatically become dependencies of the subscription.

## Deferred execution and flushing

The runtime separates invalidation from execution.

Signal writes update stored state and schedule affected work. Memos, effects, and queued subscription callbacks execute later when `Scope::flush()` processes the scheduler queue.

The scheduler uses a deterministic queue order and must respect reactive dependencies. In particular, a dirty memo must recompute before subscribers are notified about its new output.

A typical update sequence is:

1. A signal is updated.
2. Dependent memos and effects are invalidated.
3. The scheduler queues the affected work.
4. `Scope::flush()` recomputes dirty memos.
5. Each memo compares its new result with its previous result.
6. Changed memo outputs invalidate their subscribers.
7. For signal-valued memos, the runtime updates the stable output signal and queues its dependents according to the normal signal scheduling rules.
8. Queued effects and notification callbacks execute according to scheduler ordering.
9. A notified scope can be executed later through an explicit `run()` call.

The exact ordering between unrelated effects and subscriptions should be defined by the scheduler implementation and covered by tests.

### Queue semantics

The scheduler must preserve these invariants:

- Signal writes do not synchronously execute queued work.
- Initial memo and effect executions are immediate.
- A pending work item is not duplicated in the queue.
- A work item invalidated again after execution can be queued again.
- Work enqueued during a flush is handled according to the scheduler's queue-draining rules.
- Disposed work is skipped.
- Nested flush requests do not start competing scheduler loops.
- Reactive cycles must not cause unbounded execution; a configured work limit can bound processing.

If a work limit is reached, pending work must remain available for a subsequent flush. The runtime must report the limit condition according to its defined error behavior.

## Batching

Batching groups multiple signal writes into one scheduling interval.

```rust
batch(|| {
    first.set(10);
    second.set(20);
});

// Process the resulting pending work.
app.flush();
```

Nested batches should defer scheduling until the outermost batch completes. Multiple writes in one batch can therefore invalidate several dependencies without triggering reactive execution between each write.

Batching does not change the immediate initial execution semantics of newly created memos and effects.

Memo-owned signal output updates must use the same batching mechanism as ordinary signal writes. Multiple changes during a batch should not cause duplicate pending notifications for the same subscriber.

## Dependency tracking and execution context

The runtime maintains an active execution context to determine where resources belong and where dependencies should be registered.

An execution context can represent a scope, memo computation, effect, or the absence of a tracking context.

Entering a nested context temporarily replaces the active context. When the nested execution completes, the previous context must be restored. This restoration must also occur when a user-provided closure panics.

### Dependency rules

- `get()` registers a dependency with the active observer when one exists.
- `inspect()` never registers a dependency.
- Memo computations track the reactive sources they read.
- Effects track the reactive sources they read.
- Scopes track sources relevant to their own execution result.
- Memo outputs are internal reactive sources maintained by the runtime.
- Calling `memo()` registers its output with the active observer when applicable.
- Repeated reads of the same source should not create duplicate dependency registrations within one observer.
- Memo and effect dependencies are updated after their computations rerun.
- Explicit subscription callbacks do not automatically track reads.
- Updating a memo-owned output signal must not accidentally add that output as an input dependency of the memo itself.

The runtime must keep nested dependency tracking isolated. A signal read inside a memo belongs to the memo's computation; a signal read inside an effect belongs to the effect. A parent scope does not inherit every nested read automatically.

## Runtime architecture

The implementation separates the following responsibilities.

### Signal storage

Stores signal values and supports tracked and untracked reads, mutation, invalidation, and subscriptions.

### Execution context

Maintains the active scope, memo, or effect context. It manages nested execution and restores the previous context when execution completes or panics.

### Scope runtime

Stores the scope closure, execution state, positional resource cursor, cached resources, dependency registrations, and scope subscriptions.

### Memo runtime

Stores the memo computation, latest internal result, dirty state, dependencies, change policy, and output subscribers.

For a memo returning `Signal<T>`, the runtime additionally retains the identity of the memo-owned output signal slot. Recalculation updates the slot's value rather than replacing the output signal identity.

The ordinary value returned by a memo is separate from its internal record. In the signal-valued case, the returned handle identifies the persistent output slot.

### Effect runtime

Stores effect closures, resource state, dependencies, dirty state, queued status, and disposal state.

### Dependency graph

Tracks relationships between signals, memo outputs, effects, scopes, and explicit subscribers. It must support removing obsolete dependency edges when computations rerun or resources are disposed.

### Scheduler

Queues invalidated computations and notification callbacks, deduplicates pending work, and processes queued work during `Scope::flush()`.

### Disposal

Deactivates resources, removes dependency registrations, prevents disposed work from executing, and ensures stale handles cannot access disposed or recycled resources.

## Resource ownership and disposal

Scopes own the reactive resources assigned to their resource sequence. Resource ownership is distinct from the lifetime of a Rust handle referring to a resource.

The runtime must define and consistently implement the behavior of:

- Explicit scope disposal.
- Dropping strong references to a scope.
- Nested scopes and parent-child ownership.
- Effects and subscriptions associated with a scope.
- Disposing a dependent without disposing its sources.
- Disposing a source that has active dependents.
- Disposing a resource while its work is queued.
- Cleaning up resources no longer used by a completed execution.
- Reusing resource positions without allowing stale handles to access new resources.
- Disposing a memo and its memo-owned output signal.
- Preventing candidate signal results from replacing or adopting unrelated signal slots.

Disposal should be idempotent. Queued callbacks belonging to disposed resources must not execute.

Ownership relationships should avoid reference cycles. Single-threaded ownership primitives such as `Rc` and `Weak`, or centrally managed resource identifiers, can be used where appropriate.

A disposed scope must not silently resume normal resource creation. Resource-slot reuse must also ensure that an old handle cannot accidentally access a new resource occupying the same slot.

## Change detection

Signals and memos have different change-detection responsibilities.

### Signals

The default signal policy can treat every successful write as a change without requiring `PartialEq` on the signal's value type.

If equality-aware invalidation is needed, it should be offered through an explicit comparator or a separate API.

### Memos

A memo retains its latest computed result and applies its configured change policy after recomputation.

Possible policies include:

- Always treat recomputation as a change.
- Compare results using `PartialEq`.
- Use a custom comparator.

The policy determines whether the memo's output subscribers are notified. A memo can recompute without notifying subscribers when its result is considered unchanged.

For a memo returning `Signal<T>`, the runtime compares and updates the output signal's value according to the output signal's change policy. The persistent signal identity remains unchanged even when the value changes.

The implementation must not require Rust specialization to support different comparison policies.

For floating-point values, ordinary `PartialEq` semantics mean that `NaN` compares unequal to itself. This behavior matters when equality-based memo comparison is used.

## Error handling and runtime safety

The runtime must define behavior for:

- Nested or reentrant flush requests.
- Reactive cycles and work-limit exhaustion.
- Panics during scope, memo, or effect execution.
- `RefCell` borrow conflicts.
- Disposal during a flush.
- Resource kind or type mismatches at cached positions.
- Resource-count mismatches between executions.
- Stale or invalid resource handles.
- Partial execution failure and dependency cleanup.
- A memo returning its own output signal.
- A memo returning a signal owned by another scope or computation.
- Failure while extracting or applying a candidate signal result.

A panic must not leave an incorrect active execution context or a scheduler permanently marked as flushing.

A scope execution that fails partway through must leave its cached resources and dependency registrations in a defined state. The runtime must not treat a partially completed execution as a successful run without an explicit policy for doing so.

A failed memo recomputation must not leave its output signal partially updated. The runtime should retain the last successfully committed result and define how the memo's dirty state and pending notifications are handled after failure.

## Testing strategy

Tests should cover each subsystem independently and then verify interactions between them.

### Signals

- Initial values and reads.
- `get()` tracks dependencies only when an appropriate observer is active.
- `inspect()` never tracks dependencies.
- `set()` and `update()` invalidate dependents without synchronously executing queued work.
- Explicit subscriptions do not invoke callbacks immediately.
- Batched writes do not execute queued work between writes.

### Memos

- Initial computation executes immediately.
- `memo()` returns an ordinary value for ordinary result types.
- The runtime retains the latest result internally.
- Calling `memo()` registers its output with the active observer.
- Dependency changes queue recomputation.
- Recomputation executes during `Scope::flush()`.
- Subscribers are notified only when the result changes under the configured policy.
- Dependencies are replaced after recomputation.
- Repeated invalidations are deduplicated.
- No public memo-handle read method is required.

### Signal-valued memos

- The initial execution creates a stable output signal.
- The returned handle observes the initial computed value.
- Recomputation updates the existing output slot rather than replacing it.
- Cloned output handles continue to refer to the same signal.
- Output changes notify dependents through the normal signal scheduling path.
- Unchanged output values do not cause unnecessary notifications under the configured change policy.
- Candidate signal handles do not replace the memo-owned output identity.
- Returning the same candidate handle repeatedly is handled safely.
- Returning an unrelated or externally owned signal does not cause it to be adopted.
- Returning the memo's own output signal cannot cause recursive update loops.
- Batching deduplicates pending output notifications.
- Disposing the memo or its owning scope cleans up the output correctly.
- A failed recomputation does not partially update the output.

### Effects

- Initial execution happens immediately.
- Subsequent execution is deferred until `Scope::flush()`.
- Dependencies are replaced after reruns.
- Conditional reads add and remove dependencies correctly.
- Repeated invalidations do not create duplicate pending entries.
- Disposed effects do not execute.

### Scopes

- Creating a scope does not execute its closure.
- The first `run()` initializes the scope and returns its result.
- Subsequent runs return fresh closure results while reusing signals, memos, and effects at their corresponding positions.
- Reusing a signal does not reset its current value.
- Resource cursors reset for each execution context.
- Resource kind, type, and count mismatches are detected.
- Conditional computation inside a memo or effect does not shift the parent scope's resource positions.
- Scope dependencies include memo outputs registered during scope execution.
- A changed memo output notifies the scope.
- Scope notification does not automatically rerun the scope.
- Scope subscriptions run during a flush.
- Obsolete resources are cleaned up safely.

### Scheduler

- Work is queued and executed in the documented order.
- Dirty memos recompute before their output subscribers are notified.
- Signal-valued memo outputs are updated before their dependents are notified.
- Work enqueued during a flush is handled correctly.
- Nested flush requests do not start competing scheduler loops.
- Work-limit exhaustion preserves pending work.
- Reactive cycles are bounded or diagnosed.
- Panic handling restores scheduler and tracking state.

### Disposal

- Disposal is idempotent.
- Disposing a dependent does not dispose its sources.
- Disposed resources are not executed.
- Queued callbacks for disposed resources are skipped.
- Nested scope ownership behaves consistently.
- Obsolete resources are disposed safely.
- Stale handles cannot access disposed or recycled resources.
- Memo-owned output signals are disposed according to the memo and scope ownership rules.

## Non-goals

The core crate is independent of a particular UI framework or renderer. Its initial design does not require:

- Multithreaded signal access.
- Async effects or async scheduling.
- A built-in UI renderer or component system.
- Automatic scope execution when a dependency changes.
- A public reactive memo handle.
- Automatic synchronization between an ordinary memo result and a signal initialized from that result.
- Automatic support for conditional resource creation in a scope's positional hook sequence.

These capabilities can be considered separately without coupling the reactive core to a particular UI framework.

## Implementation principles

The implementation should preserve these core invariants:

1. **Signals store state.** Writes update the value and invalidate dependents without synchronously running queued work.
2. **Tracked reads use `get()`.** A read registers a dependency only when a tracking context is active.
3. **Untracked reads use `inspect()`.** They never register a dependency.
4. **Memos retain their latest result internally.** Ordinary returned values are snapshots, not reactive handles.
5. **Memo calls establish output dependencies.** The active observer is registered as a dependent when `memo()` is called.
6. **Signal-valued memos retain stable output identity.** Recomputation updates the memo-owned output slot rather than replacing its signal identity.
7. **Effects track their own reads.** Their dependencies are updated after every execution.
8. **Scopes persist resources positionally.** Repeated executions reuse resources at corresponding positions rather than resetting or recreating them.
9. **Resource order is stable.** Conditional resource creation is not allowed to shift the parent execution context's resource positions; conditional reactive logic belongs inside memos and effects.
10. **Execution is deferred.** Subsequent reactive work is processed by `Scope::flush()`, while initial memo and effect execution remains immediate.
11. **Scopes rerun explicitly.** Dependency notifications signal that a scope may need another execution but do not automatically execute it.
12. **Ownership and disposal are deterministic.** Disposed resources cannot execute pending work or be accessed through stale handles.
13. **Nested contexts remain isolated.** Dependencies and resource cursors belong to their own execution contexts, and the previous context is restored after nested execution.

These invariants define the contract between the reactive runtime and any consumer.
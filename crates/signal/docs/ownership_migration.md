# TODO: Migrate the signal crate to scope-managed borrowing


## Target architecture
The scope owns resource storage and the borrow state associated with each resource. ``Signal<T>`` remains a Copy handle containing a `ResourceId`, reading returns a direct reference instead of an `Rc<T>` snapshot.
The public API should ultimately provide:
```rs
impl<T: 'static> `Signal<T>` {
    pub fn get(&self) -> &T;

    pub fn try_get(&self) -> Result<&T, BorrowError>;

    pub fn set(&mut self, value: impl Into<T>);

    pub fn try_set(
        &mut self,
        value: impl Into<T>,
    ) -> Result<(), BorrowError>;

    pub fn update(
        &mut self,
        closure: impl FnOnce(&mut T),
    );
}
```

This is the target shape, not code to paste yet. The precise updated return type and whether to expose try_set should be finalized during implementation.
The intended behavior is:
- ``get()`` tracks the active reactive dependency, registers a read borrow, and returns `&T`.
- `try_`get()`` does the same but reports a mutable-borrow conflict.
- `set()` and `update()` require exclusive access and must reject conflicting reads or writes.
- `Scope`, `memo`, and `effect` execution establish and release their borrow frames.
- Copied handles cannot bypass resource-level borrow tracking.
- `Rc` remains where useful for reactive metadata and scheduling; removing `Rc` from value ownership does not require removing it everywhere. (This is still open for discussion, if we implement our own way of tracking resources, dependenies and ref counting we wont need `Rc`)

## Phase 1 — Define the borrow-tracking contract
- [ ] Define BorrowError and the exact conflict conditions.
- [ ] Define the state tracked for each resource: reader count, active writer, and any execution-frame ownership needed for cleanup.
- [ ] Define whether nested frames may read a resource borrowed by their parent frame. Shared reads can be permitted, but a child must not release its parent's borrow.
- [ ] Define what happens when a write conflicts with a read: set() panics, try_set() returns an error, and update() follows the same exclusive-access rule.
- [ ] Define what happens when a scope frame exits normally or unwinds due to a panic.
- [ ] Define how a frame identifies its own borrows so cleanup cannot release borrows owned by another frame.
- [ ] Define the lifetime and stability invariant: while any returned reference is live, its target must remain allocated and must not be mutated through another access path.
- [ ] Resolve one API issue before implementation: current integration tests call `get()`, set(), and update() outside Scope::run(). Decide whether these operations are supported outside execution frames or whether all reference-based access must happen inside a frame. The implementation must enforce one consistent rule.
Acceptance criteria: the borrowing rules are written down and apply equally to copied handles, nested executions, and error paths.
Phase 2 — Replace the current resource storage representation
File: crates/signal/src/scope.rs
The current ResourceStore stores resources in Vec<Slot>, where each slot contains Box<dyn Any> holding an Rc<T>. resolve_resource() obtains an Rc<T> through the registry. That is the current ownership path to replace.
- [ ] Introduce a storage representation that owns signal values directly, rather than storing Rc<SignalInner<T>> as the ownership mechanism for values.
- [ ] Separate signal values from reactive metadata such as SourceNode, subscribers, and event queues.
- [ ] Add borrow state to each signal resource, or to a stable allocation associated with that resource.
- [ ] Preserve ResourceId { scope, index, generation }.
- [ ] Preserve validation of scope identity, resource kind, concrete type, and generation.
- [ ] Preserve resource reuse by execution position.
- [ ] Preserve generation increments when a removed resource slot is reused.
- [ ] Ensure growing or truncating resources: Vec<Slot> cannot invalidate a live reference to a signal value.
- [ ] Ensure replacing a value does not move or destroy the old value while any reference to it remains live.
- [ ] Keep memo and effect resource storage functional during the signal migration; do not change all resource kinds at once unless the shared representation requires it.
Important implementation constraint: the current Vec<Slot> can reallocate, and the current resolve_resource() returns an Rc<T> rather than a borrow into the store. Simply changing Rc<T> to T will not be sufficient. A direct-reference implementation needs stable value storage and a carefully encapsulated borrowing mechanism.
Acceptance criteria: the store remains the owner of signal values, and a resource ID resolves to the correct value without relying on an owned Rc<T> snapshot.
Phase 3 — Add the storage-level read/write mechanism
File: crates/signal/src/scope.rs
- [ ] Implement acquisition of a read borrow for a resource.
- [ ] Allow multiple readers when no writer exists.
- [ ] Reject read acquisition when a writer exists.
- [ ] Implement acquisition of an exclusive write borrow.
- [ ] Reject write acquisition when any reader or writer exists.
- [ ] Associate acquired borrows with the correct resource generation.
- [ ] Associate execution-scoped borrows with the current frame.
- [ ] Make cleanup idempotent so a borrow cannot be released twice.
- [ ] Handle panic unwinding without leaving the resource permanently marked as borrowed.
- [ ] Ensure stale handles cannot access the borrow state of a replacement resource.
- [ ] Encapsulate all unsafe operations, if needed, behind a small internal API with documented safety invariants.
The tracker must be authoritative. Every path that reads, mutates, replaces, or recalculates a stored value must obey it.
Acceptance criteria: two copied handles pointing to the same resource cannot acquire conflicting access, regardless of which handle initiated the borrow.
Phase 4 — Add execution-frame management
Files: crates/signal/src/runtime.rs, crates/signal/src/scope.rs
The current runtime already uses ObserverGuard to install and restore the active reactive observer. Borrow-frame management should follow the same general RAII pattern, while remaining separate from dependency tracking.
- [ ] Introduce an internal execution-frame guard.
- [ ] Record the frame when entering a scope, memo computation, or effect callback.
- [ ] Maintain a stack or equivalent mechanism for nested execution frames.
- [ ] Record each frame's acquired borrows.
- [ ] Restore the previous active frame when a nested execution exits.
- [ ] Release only the borrows acquired by the exiting frame.
- [ ] Ensure normal return and panic unwinding both restore the active frame.
- [ ] Ensure a nested memo or effect cannot accidentally release a borrow belonging to its caller.
- [ ] Ensure frame cleanup does not invalidate references still permitted by the API.
- [ ] Keep dependency tracking behavior unchanged: entering a borrow frame must not implicitly add reactive dependencies.
Acceptance criteria: every execution path has a well-defined entry and exit, and nested cleanup affects only its own frame.
Phase 5 — Integrate frames into Scope::run() and Scope::flush()
File: crates/signal/src/scope.rs
The current Scope::run() already enters ActiveScopeGuard and ObserverGuard while evaluating the closure. Scope::flush() executes queued work, including effects and memo recalculations.
- [ ] Enter the borrow frame alongside the active scope and observer in Scope::run().
- [ ] Exit the frame after the scope closure has finished evaluating.
- [ ] Preserve the current result caching and invalidation behavior.
- [ ] Ensure the resource cursor and resource truncation logic do not invalidate active references.
- [ ] Enter the appropriate frame for each unit of queued work processed by flush().
- [ ] Ensure each queued callback's borrows are released before unrelated queued work executes.
- [ ] Preserve pending-event dispatch order and batching semantics.
- [ ] Add panic-path tests to verify the active scope, observer, and borrow frame are all restored.
Acceptance criteria: ordinary scope evaluation and queued work cannot leave stale borrow registrations behind.
Phase 6 — Rewrite signal reads and writes
File: crates/signal/src/signal.rs
The current implementation stores value: RefCell<Option<Rc<T>>>, returns SignalReadGuard<T> snapshots, and uses Rc::get_mut() inside update().
- [ ] Remove SignalReadGuard<T> from the public read path.
- [ ] Remove Deref, Clone, Debug, and Display implementations that exist only to support SignalReadGuard<T>.
- [ ] Change `get()` to return &T.
- [ ] Make `get()` panic with a clear borrow-conflict message when a mutable borrow is active.
- [ ] Add try_`get()` returning Result<&T, BorrowError>.
- [ ] Keep dependency tracking in `get()` and try_`get()`.
- [ ] Add or preserve an untracked read API, such as `inspect() -> &T`, with the same borrow safety rules but without dependency tracking.
- [ ] Change `set()` to require `&mut self`.
- [ ] Add `try_set()` if non-panicking writes are part of the chosen public API.
- [ ] Change `update()` to require `&mut self` and mutate the stored value under exclusive access.
- [ ] Remove the `Rc::get_mut()` requirement.
- [ ] Invalidate dependents only after a successful mutation.
- [ ] Queue subscription notifications only after a successful mutation.
- [ ] Preserve the `Signal<T>`: Copy and Clone behavior.
- [ ] Preserve `value()` and `inspect_value()` for `T: Copy`.
- [ ] Update `Debug`, `PartialEq`, `Eq`, and `Hash` implementations to use the new read path or resource identity as appropriate.
- [ ] Update signal construction to initialize the new storage and borrow state.
Acceptance criteria: reads return direct references, writes require exclusive access, and reactive invalidation and subscription semantics remain unchanged.
Phase 7 — Migrate memo storage and recalculation
File: crates/signal/src/memo.rs
The current `MemoInner<T>` stores value: `RefCell<Option<Rc<T>>>`; `get()` and `inspect()` return snapshot guards. Memo recalculation runs in ensure_value() under an ObserverGuard.
- [ ] Decide whether memo cached values use the same storage and borrow-state abstraction as signal values.
- [ ] Remove `MemoReadGuard<T>` if memo reads also move to direct references.
- [ ] Add borrow-aware memo reads while preserving lazy evaluation.
- [ ] Ensure a memo is evaluated before acquiring the read borrow for its cached value.
- [ ] Ensure the memo's own cached value cannot be replaced while readers hold references to it.
- [ ] Enter an execution frame for the memo computation in ensure_value().
- [ ] Release the computation frame before exposing the newly calculated cached value.
- [ ] Preserve dependency clearing and rebuilding.
- [ ] Preserve equality checks and the rule that downstream observers are invalidated only when the memo value changes.
- [ ] Preserve subscription coalescing and notification behavior.
- [ ] Preserve Memo<T>: Copy if that remains a requirement.
- [ ] Preserve the existing PartialEq bound and test non-Copy memo values.
Acceptance criteria: lazy evaluation, caching, equality comparison, and dependency propagation continue working without Rc<T> snapshots.
Phase 8 — Migrate effect execution
File: crates/signal/src/effect.rs
Effects currently execute queued closures under an ObserverGuard and rebuild their dependencies before every run.
- [ ] Enter a borrow frame for each effect execution.
- [ ] Ensure reads inside the effect register with that frame.
- [ ] Release effect-owned borrows when the callback exits.
- [ ] Ensure nested memo calculations use their own nested frames.
- [ ] Preserve initial scheduling and invalidation scheduling.
- [ ] Preserve the scheduled flag and work coalescing.
- [ ] Preserve weak references used for scheduler ownership where they are still appropriate.
- [ ] Ensure panic unwinding restores both the observer and borrow frame.
- [ ] Test that an effect can read a signal, finish, and then allow the signal to be written.
- [ ] Test that conflicting access during effect execution is detected.
Acceptance criteria: effect execution participates in the same borrow protocol without changing its scheduling behavior.
Phase 9 — Migrate subscriptions and queued callbacks
Files: crates/signal/src/signal.rs, crates/signal/src/memo.rs, potentially crates/signal/src/runtime.rs
Current signal and memo subscription callbacks receive &T, but the queued work obtains that reference from an Rc<T> snapshot before invoking the callback.
- [ ] Remove the snapshot cloning from signal notification delivery.
- [ ] Ensure a notification callback runs within an execution frame.
- [ ] Acquire a valid read borrow before passing &T to the callback.
- [ ] Ensure that reference cannot outlive the callback execution.
- [ ] Decide what happens if a notification runs while the resource is mutably borrowed.
- [ ] Preserve cancellation through Subscription::unsubscribe().
- [ ] Preserve per-subscription pending flags and notification coalescing.
- [ ] Preserve the behavior where callbacks observe the latest value at delivery time.
- [ ] Apply equivalent changes to memo notifications if memo values move to direct references.
Acceptance criteria: callbacks receive valid references without retaining snapshots, and notification ordering and cancellation remain unchanged.
Phase 10 — Update public API and integration tests
Files: crates/signal/src/lib.rs, crates/signal/tests/signal.rs
- [ ] Export BorrowError if it is part of the public API.
- [ ] Remove public exports of obsolete read-guard types, if any exist.
- [ ] Update signal_stores_and_updates_its_value for mutable writes.
- [ ] Update set_replaces_the_value to use a mutable handle.
- [ ] Update update_mutates_the_value_and_returns_a_read_guard to match the new update() return type.
- [ ] Update tests that access values outside Scope::run() according to the chosen access rule.
- [ ] Keep tests proving that `Signal<T>` is Copy even when T is not Copy.
- [ ] Preserve tests for dependency tracking and scope invalidation.
- [ ] Preserve tests for untracked reads.
- [ ] Add tests for `get()` panicking during a mutable borrow.
- [ ] Add tests for try_`get()` returning an error during a mutable borrow.
- [ ] Add tests for multiple simultaneous readers.
- [ ] Add tests for write conflicts across copied handles.
- [ ] Add tests for stale resource IDs and generation reuse.
- [ ] Add tests for cleanup after a panic.
- [ ] Add tests for nested scope, memo, and effect execution.
- [ ] Add tests proving that reads do not create a dependency when using the untracked API.
- [ ] Verify that old references cannot be observed after a resource is replaced or its slot reused.
Acceptance criteria: all public tests describe the new semantics and cover both ordinary behavior and aliasing edge cases.
Phase 11 — Verify the migration
From the repository root:
- [ ] Run cargo test -p signal.
- [ ] Run cargo test --workspace.
- [ ] Run cargo clippy -p signal --all-targets --all-features -- -D warnings.
- [ ] Run cargo fmt --all -- --check.
- [ ] Search crates/signal/src for remaining SignalReadGuard, MemoReadGuard, and value-storage Rc<T> usage.
- [ ] Review every unsafe block introduced by the storage implementation.
- [ ] Check that all unsafe blocks have explicit safety comments explaining allocation stability, aliasing, mutation exclusion, and reference lifetime guarantees.
- [ ] Check panic paths to ensure no borrow state or active execution frame remains stuck.
- [] Confirm that runtime Rc usage for observers, source nodes, event state, and queued work remains where it is still needed.
Recommended implementation order
1. Borrow contract and storage design — Phase 1.
2. Storage and borrow tracker — Phases 2–3.
3. Execution frames and scope integration — Phases 4–5.
4. Signals and public tests — Phases 6 and 10, initially limited to signals.
5. Memo and effect execution — Phases 7–8.
6. Subscriptions and callback lifetimes — Phase 9.
7. Full verification and cleanup — Phase 11.
The first implementation step should be Phase 1, followed by the storage design in Phase 2. Do not change Signal::`get()` first: its direct-reference implementation depends on the storage's allocation stability and borrowing protocol.
1. Keep signal values in signal nodes. The value is conceptually part of the signal; you do not need a global registry solely to store values.
2. Let the runtime manage the dependency graph. It tracks which computations depend on each signal, invalidation, pending work, and cleanup.
3. Make local signals scope-owned. Disposing a scope removes its signals and computations from the active reactive graph.
4. Give shared signals explicit ownership semantics. A shared handle provides cross-thread access, but cloning it does not transfer ownership or prevent logical disposal.
5. Keep scheduling separate from storage. Shared writes invalidate dependents and queue work; the owning consumer processes that work.
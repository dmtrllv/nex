use std::{
    cell::{Ref, RefCell},
    rc::Rc,
};

use crate::{
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState, SourceNode},
    scope::memo_resource,
    Subscription,
};

// NOTE:
// One important caveat
// This is the first implementation layer, not the final memo implementation. 
// It establishes the evaluation path and connects invalidation to queued work. 
// The next step must add memo subscriptions and tests, then check edge cases 
// such as a memo being read after invalidation but before its queued work runs.

struct MemoInner<T> {
    compute: RefCell<Box<dyn Fn() -> T>>,
    value: RefCell<Option<T>>,
    source: Rc<SourceNode>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
}

/// A lazily evaluated, cached reactive computation.
pub struct Memo<T: PartialEq + 'static> {
    inner: Rc<MemoInner<T>>,
}

impl<T: PartialEq + 'static> Memo<T> {
    /// Lazily evaluates the memo and tracks the active observer as a
    /// dependent of its cached value.
    pub fn get(&self) -> Ref<'_, T> {
        ensure_value(&self.inner);
        runtime::track(&self.inner.source);

        Ref::map(self.inner.value.borrow(), |value| {
            value.as_ref().expect("memo value should be initialized")
        })
    }

    /// Lazily evaluates the memo without tracking a dependency.
    pub fn inspect(&self) -> Ref<'_, T> {
        ensure_value(&self.inner);

        Ref::map(self.inner.value.borrow(), |value| {
            value.as_ref().expect("memo value should be initialized")
        })
    }

    /// Subscribes to changes in this memo.
    ///
    /// Notifications will be queued when the memo's computed value changes.
    pub fn subscribe(&self, _closure: impl Fn(&T) + 'static) -> Subscription {
        todo!("implement memo subscriptions in the next step")
    }
}

impl<T: Copy + PartialEq + 'static> Memo<T> {
    /// Returns a copy of the memo's value and tracks the active observer.
    pub fn value(&self) -> T {
        *self.get()
    }

    /// Returns a copy of the memo's value without tracking a dependency.
    pub fn inspect_value(&self) -> T {
        *self.inspect()
    }
}

/// Creates or retrieves a memo resource in the active scope.
pub fn memo<T: PartialEq + 'static>(closure: impl Fn() -> T + 'static) -> Memo<T> {
    let inner = memo_resource(|events| MemoInner {
        compute: RefCell::new(Box::new(closure)),
        value: RefCell::new(None),
        source: SourceNode::new(),
        observer: ObserverNode::new(),
        events,
    });

    // The scheduled callback holds only a weak reference, so it cannot
    // keep the memo alive after its owning resources are released.
    let weak_inner = Rc::downgrade(&inner);
    inner.observer.set_memo_schedule(Rc::new(move || {
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };

        let weak_inner = Rc::downgrade(&inner);
        inner.events.queue_work(move || {
            let Some(inner) = weak_inner.upgrade() else {
                return;
            };

            ensure_value(&inner);
        });
    }));

    Memo { inner }
}

/// Ensures the memo has a current cached value.
///
/// Returns `true` if a previously cached value changed.
fn ensure_value<T: PartialEq + 'static>(inner: &Rc<MemoInner<T>>) -> bool {
    if inner.value.borrow().is_some() && !inner.observer.is_invalidated() {
        return false;
    }

    runtime::clear_dependencies(&inner.observer);

    // Mark valid before evaluation. If the computation causes one of its
    // dependencies to change while running, invalidation can mark it dirty
    // again and schedule another pass.
    inner.observer.mark_valid();

    let next = {
        let _observer_guard = ObserverGuard::enter(Rc::clone(&inner.observer));
        (inner.compute.borrow())()
    };

    let changed = {
        let mut cached = inner.value.borrow_mut();

        match cached.as_ref() {
            Some(previous) if previous == &next => false,
            Some(_) => {
                *cached = Some(next);
                true
            }
            None => {
                *cached = Some(next);
                false
            }
        }
    };

    if changed {
        runtime::invalidate(&inner.source);
    }

    changed
}
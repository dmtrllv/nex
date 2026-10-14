use std::{
    cell::{Cell, Ref, RefCell},
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState, SourceNode},
    scope::memo_resource,
};

struct MemoSubscriber<T> {
    active: Rc<Cell<bool>>,
    pending: Rc<Cell<bool>>,
    callback: Rc<dyn Fn(&T)>,
}

struct MemoInner<T> {
    compute: RefCell<Box<dyn Fn() -> T>>,
    value: RefCell<Option<T>>,
    source: Rc<SourceNode>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
    subscribers: RefCell<Vec<MemoSubscriber<T>>>,
}

/// A lazily evaluated, cached reactive computation.
#[derive(Clone)]
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
    /// Notifications are deferred until queued work is processed.
    /// The initial evaluation does not produce a notification.
    pub fn subscribe(&self, closure: impl Fn(&T) + 'static) -> Subscription {
        let (subscription, active) = Subscription::create();

        self.inner.subscribers.borrow_mut().push(MemoSubscriber {
            active,
            pending: Rc::new(Cell::new(false)),
            callback: Rc::new(closure),
        });

        subscription
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
    let mut compute: Option<Box<dyn Fn() -> T>> = Some(Box::new(closure));

    let inner = memo_resource(|events| MemoInner {
        compute: RefCell::new(
            compute
                .take()
                .expect("memo computation should be available when creating a resource"),
        ),
        value: RefCell::new(None),
        source: SourceNode::new(),
        observer: ObserverNode::new(),
        events,
        subscribers: RefCell::new(Vec::new()),
    });

    // When the resource already existed, its creation closure was not called.
    // Replace the old computation with the closure from this scope run.
    if let Some(compute) = compute {
        *inner.compute.borrow_mut() = compute;
        inner.observer.mark_invalidated();
    }

    // The scheduled callback holds only a weak reference, so it cannot
    // keep the memo alive after its owning resources are released.
    let weak_inner = Rc::downgrade(&inner);
    inner.observer.set_schedule(Rc::new(move || {
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

fn queue_notifications<T: PartialEq + 'static>(inner: &Rc<MemoInner<T>>) {
    let subscribers: Vec<_> = {
        let mut subscribers = inner.subscribers.borrow_mut();

        subscribers.retain(|subscriber| subscriber.active.get());

        subscribers
            .iter()
            .map(|subscriber| {
                (
                    Rc::clone(&subscriber.active),
                    Rc::clone(&subscriber.pending),
                    Rc::clone(&subscriber.callback),
                )
            })
            .collect()
    };

    for (active, pending, callback) in subscribers {
        if pending.replace(true) {
            continue;
        }

        let inner = Rc::clone(inner);
        let weak_inner = Rc::downgrade(&inner);

        inner.events.queue_work(move || {
            pending.set(false);

            if !active.get() {
                return;
            }

            let Some(inner) = weak_inner.upgrade() else {
                return;
            };

            // A memo can be invalidated again before this notification runs.
            ensure_value(&inner);

            let value = inner.value.borrow();
            let value = value.as_ref().expect("memo value should be initialized");

            callback(value);
        });
    }
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
        queue_notifications(inner);
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn memo_subscription_notifies_only_when_value_changes() {
        let signal_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&signal_handle);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(1);
            *handle.borrow_mut() = Some(source.clone());

            let input = source.clone();
            let derived = memo(move || input.value() % 2);

            (source, derived)
        });

        let (_, derived) = app.run().clone();

        // Initialize the lazy memo before subscribing.
        assert_eq!(derived.value(), 1);

        let received = Rc::new(RefCell::new(Vec::new()));
        let callback_received = Rc::clone(&received);

        let _subscription = derived.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        // 1 -> 3 leaves the computed value unchanged.
        signal_handle.borrow().as_ref().unwrap().set(3);
        app.flush();

        assert!(received.borrow().is_empty());

        // 3 -> 4 changes the computed value from 1 to 0.
        signal_handle.borrow().as_ref().unwrap().set(4);
        app.flush();

        assert_eq!(*received.borrow(), vec![0]);
    }

    #[test]
    fn reading_invalidated_memo_before_flush_recomputes_once() {
        let signal_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&signal_handle);
        let computations = Rc::new(Cell::new(0));
        let computation_count = Rc::clone(&computations);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(1);
            *handle.borrow_mut() = Some(source.clone());

            let input = source.clone();
            let cc = computation_count.clone();
            memo(move || {
                cc.set(cc.get() + 1);
                input.value() * 2
            })
        });

        let derived = app.run().clone();
        assert_eq!(derived.value(), 2);
        assert_eq!(computations.get(), 1);

        signal_handle.borrow().as_ref().unwrap().set(2);

        // This read recomputes immediately, before app.flush().
        assert_eq!(derived.value(), 4);
        assert_eq!(computations.get(), 2);

        // The previously queued recomputation must now be a no-op.
        app.flush();
        assert_eq!(derived.value(), 4);
        assert_eq!(computations.get(), 2);
    }

    #[test]
    fn memo_subscription_can_be_unsubscribed_before_delivery() {
        let signal_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&signal_handle);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(1);
            *handle.borrow_mut() = Some(source.clone());

            let input = source.clone();
            memo(move || input.value() * 2)
        });

        let derived = app.run().clone();

        // Initialize the lazy memo before subscribing.
        assert_eq!(derived.value(), 2);

        let calls = Rc::new(Cell::new(0));
        let callback_calls = Rc::clone(&calls);

        let subscription = derived.subscribe(move |_| {
            callback_calls.set(callback_calls.get() + 1);
        });

        signal_handle.borrow().as_ref().unwrap().set(2);
        app.flush();

        subscription.unsubscribe();

        // No additional calls after unsubscription.
        signal_handle.borrow().as_ref().unwrap().set(3);
        app.flush();

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn reused_memo_uses_computation_from_latest_scope_run() {
        let trigger_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&trigger_handle);
        let multiplier = Rc::new(Cell::new(2));
        let current_multiplier = Rc::clone(&multiplier);

        let mut app = crate::scope::scope(move || {
            let trigger = crate::signal::signal(0);
            let source = crate::signal::signal(10);

            *handle.borrow_mut() = Some(trigger.clone());

            // This dependency causes the scope to execute again.
            let _ = trigger.value();

            // Capture the current non-reactive value in the memo closure.
            let factor = current_multiplier.get();
            let input = source.clone();

            memo(move || input.value() * factor)
        });

        let derived = app.run().clone();
        assert_eq!(derived.value(), 20);

        // Change the captured value, then invalidate the scope.
        multiplier.set(3);
        trigger_handle.borrow().as_ref().unwrap().set(1);

        // The scope rerun must update the existing memo's closure.
        let reused = app.run().clone();
        assert_eq!(reused.value(), 30);

        // The memo resource itself must have been reused.
        assert!(Rc::ptr_eq(&derived.inner, &reused.inner));
    }
}

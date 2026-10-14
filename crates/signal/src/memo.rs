
use std::{
    cell::{Cell, RefCell},
    fmt::Debug,
    marker::PhantomData,
    ops::Deref,
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState, SourceNode},
    scope::{ResourceId, ResourceKind, memo_resource, resolve_resource},
};

struct MemoSubscriber<T> {
    active: Rc<Cell<bool>>,
    pending: Rc<Cell<bool>>,
    callback: Rc<dyn Fn(&T)>,
}

struct MemoInner<T> {
    compute: RefCell<Box<dyn Fn() -> T>>,
    value: RefCell<Option<Rc<T>>>,
    source: Rc<SourceNode>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
    subscribers: RefCell<Vec<MemoSubscriber<T>>>,
}

/// An owned snapshot of a memo's cached value.
pub struct MemoReadGuard<T> {
    value: Rc<T>,
}

impl<T> Deref for MemoReadGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref()
    }
}

impl<T> Clone for MemoReadGuard<T> {
    fn clone(&self) -> Self {
        Self {
            value: Rc::clone(&self.value),
        }
    }
}

/// A lightweight handle to a lazily evaluated, cached reactive computation.
pub struct Memo<T: PartialEq + 'static> {
    id: ResourceId,
    marker: PhantomData<fn() -> T>,
}

impl<T: PartialEq + 'static> Copy for Memo<T> {}

impl<T: PartialEq + 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: PartialEq + 'static> PartialEq for Memo<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: PartialEq + 'static> Eq for Memo<T> {}

impl<T: PartialEq + 'static> std::hash::Hash for Memo<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: PartialEq + 'static> Memo<T> {
    fn inner(&self) -> Rc<MemoInner<T>> {
        resolve_resource(self.id, ResourceKind::Memo)
    }

    /// Lazily evaluates the memo and tracks the active observer as a
    /// dependent of its cached value.
    pub fn get(&self) -> MemoReadGuard<T> {
        let inner = self.inner();

        ensure_value(&inner);
        runtime::track(&inner.source);

        let value = inner
            .value
            .borrow()
            .as_ref()
            .expect("memo value should be initialized")
            .clone();

        MemoReadGuard { value }
    }

    /// Lazily evaluates the memo without tracking a dependency.
    pub fn inspect(&self) -> MemoReadGuard<T> {
        let inner = self.inner();

        ensure_value(&inner);

        let value = inner
            .value
            .borrow()
            .as_ref()
            .expect("memo value should be initialized")
            .clone();

        MemoReadGuard { value }
    }

    /// Subscribes to changes in this memo.
    ///
    /// Notifications are deferred until queued work is processed.
    /// The initial evaluation does not produce a notification.
    pub fn subscribe(&self, closure: impl Fn(&T) + 'static) -> Subscription {
        let inner = self.inner();
        let (subscription, active) = Subscription::create();

        inner.subscribers.borrow_mut().push(MemoSubscriber {
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

impl<T: Debug + PartialEq + 'static> Debug for Memo<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Memo")
            .field(self.inspect().deref())
            .finish()
    }
}

/// Creates or retrieves a memo resource in the active scope.
pub fn memo<T: PartialEq + 'static>(closure: impl Fn() -> T + 'static) -> Memo<T> {
    let mut compute: Option<Box<dyn Fn() -> T>> = Some(Box::new(closure));

    let id = memo_resource(|events| MemoInner {
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

    let handle = Memo {
        id,
        marker: PhantomData,
    };

    let inner = handle.inner();

    // If the resource already existed, its creation closure was not called.
    // Update its computation to use the closure from the latest scope run.
    if let Some(compute) = compute {
        *inner.compute.borrow_mut() = compute;
        inner.observer.mark_invalidated();
    }

    // Scheduled work holds only a weak reference. The scope remains the
    // owner of the memo resource.
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

    // Creating a memo inside a scope establishes a dependency on its source.
    runtime::track(&inner.source);

    handle
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

        let weak_inner = Rc::downgrade(inner);
        let events = Rc::clone(&inner.events);

        events.queue_work(move || {
            pending.set(false);

            if !active.get() {
                return;
            }

            let Some(inner) = weak_inner.upgrade() else {
                return;
            };

            // The memo may have been invalidated again before this
            // notification runs. Ensure the callback sees the latest value.
            ensure_value(&inner);

            let value = inner
                .value
                .borrow()
                .as_ref()
                .expect("memo value should be initialized")
                .clone();

            callback(value.as_ref());
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

    // Mark valid before evaluation. If a dependency changes during evaluation,
    // invalidation can mark the memo dirty again and schedule another pass.
    inner.observer.mark_valid();

    let next = {
        let _observer_guard = ObserverGuard::enter(Rc::clone(&inner.observer));
        (inner.compute.borrow())()
    };

    let changed = {
        let mut cached = inner.value.borrow_mut();

        match cached.as_ref() {
            Some(previous) if previous.as_ref() == &next => false,
            Some(_) => {
                *cached = Some(Rc::new(next));
                true
            }
            None => {
                *cached = Some(Rc::new(next));
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
            *handle.borrow_mut() = Some(source);

            let input = source;
            let derived = memo(move || input.value() % 2);

            (source, derived)
        });

        let (_, derived) = *app.run();

        assert_eq!(derived.value(), 1);

        let received = Rc::new(RefCell::new(Vec::new()));
        let callback_received = Rc::clone(&received);

        let _subscription = derived.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        signal_handle.borrow().as_ref().unwrap().set(3);
        app.flush();

        assert!(received.borrow().is_empty());

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
            *handle.borrow_mut() = Some(source);

            let input = source;
            let cc = Rc::clone(&computation_count);

            memo(move || {
                cc.set(cc.get() + 1);
                input.value() * 2
            })
        });

        let derived = *app.run();

        assert_eq!(derived.value(), 2);
        assert_eq!(computations.get(), 1);

        signal_handle.borrow().as_ref().unwrap().set(2);

        assert_eq!(derived.value(), 4);
        assert_eq!(computations.get(), 2);

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
            *handle.borrow_mut() = Some(source);

            let input = source;
            memo(move || input.value() * 2)
        });

        let derived = *app.run();
        assert_eq!(derived.value(), 2);

        let calls = Rc::new(Cell::new(0));
        let callback_calls = Rc::clone(&calls);

        let subscription = derived.subscribe(move |_| {
            callback_calls.set(callback_calls.get() + 1);
        });

        signal_handle.borrow().as_ref().unwrap().set(2);
        app.flush();

        subscription.unsubscribe();

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

            *handle.borrow_mut() = Some(trigger);

            let _ = trigger.value();

            let factor = current_multiplier.get();
            let input = source;

            memo(move || input.value() * factor)
        });

        let derived = *app.run();
        assert_eq!(derived.value(), 20);

        multiplier.set(3);
        trigger_handle.borrow().as_ref().unwrap().set(1);

        let reused = *app.run();

        assert_eq!(reused.value(), 30);
        assert_eq!(derived, reused);
    }

    #[test]
    fn memo_handle_is_copy_without_t_copy() {
		#[derive(PartialEq)]
        struct NonCopy(String);

        let mut app = crate::scope::scope(|| {
            memo(|| NonCopy(String::from("value")))
        });

        let first = *app.run();
        let second = first;

        assert_eq!(first.inspect().0, "value");
        assert_eq!(second.inspect().0, "value");
    }

    #[test]
    fn memo_read_guard_keeps_old_snapshot_after_recomputation() {
        let source_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&source_handle);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(1);
            *handle.borrow_mut() = Some(source);

            let input = source;
            memo(move || format!("value-{}", input.value()))
        });

        let derived = *app.run();
        let old_snapshot = derived.get();

        source_handle.borrow().as_ref().unwrap().set(2);
        assert_eq!(&*derived.get(), "value-2");
        assert_eq!(&*old_snapshot, "value-1");
    }
}

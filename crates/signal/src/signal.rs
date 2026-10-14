
use std::{
    any::Any, cell::{Cell, RefCell}, fmt::{Debug, Display}, marker::PhantomData, ops::Deref, rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ScopeEventState, SourceNode},
    scope::{ResourceId, ResourceKind, resolve_resource, signal_resource},
};

struct SignalSubscriber<T> {
    active: Rc<Cell<bool>>,
    pending: Rc<Cell<bool>>,
    callback: Rc<dyn Fn(&T)>,
}

struct SignalInner<T> {
    value: RefCell<Option<Rc<T>>>,
    source: Rc<SourceNode>,
    events: Rc<ScopeEventState>,
    subscribers: RefCell<Vec<SignalSubscriber<T>>>,
}

/// An owned snapshot of a signal value.
///
/// The guard keeps the observed value alive even after the signal is updated.
pub struct SignalReadGuard<T> {
    value: Rc<T>,
}

impl<T> Deref for SignalReadGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value.as_ref()
    }
}

impl<T> Clone for SignalReadGuard<T> {
    fn clone(&self) -> Self {
        Self {
            value: Rc::clone(&self.value),
        }
    }
}

impl<T: Debug> Debug for SignalReadGuard<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SignalReadGuard").field("value", &self.value).finish()
	}
}

impl<T: Debug> Display for SignalReadGuard<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("SignalReadGuard").field("value", &self.value).finish()
	}
}

/// A lightweight handle to a reactive value.
///
/// The scope owns the signal resource. The handle stores only its identity.
pub struct Signal<T: 'static> {
    id: ResourceId,
    marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Signal<T> {
    fn inner(&self) -> Rc<SignalInner<T>> {
        resolve_resource(self.id, ResourceKind::Signal)
    }

    /// Reads the current value and tracks the active observer.
    ///
    /// The returned guard owns an `Rc<T>` snapshot. Subsequent writes do not
    /// change the value observed through an existing guard.
    pub fn get(&self) -> SignalReadGuard<T> {
        let inner = self.inner();
        runtime::track(&inner.source);

        let value = inner
            .value
            .borrow()
            .as_ref()
            .expect("signal value temporarily unavailable")
            .clone();

        SignalReadGuard { value }
    }

    /// Reads the current value without tracking a dependency.
    pub fn inspect(&self) -> SignalReadGuard<T> {
        let inner = self.inner();

        let value = inner
            .value
            .borrow()
            .as_ref()
            .expect("signal value temporarily unavailable")
            .clone();

        SignalReadGuard { value }
    }

    /// Replaces the current value and invalidates dependent observers.
    pub fn set(&self, value: impl Into<T>) {
        let inner = self.inner();

        *inner.value.borrow_mut() = Some(Rc::new(value.into()));

        runtime::invalidate(&inner.source);
        queue_notifications(&inner);
    }

    /// Mutates the current value in place.
    ///
    /// The stored `Rc<T>` is temporarily removed from the option so that
    /// exclusive mutable access can be obtained. This requires that no
    /// read guards or other strong references still hold the current
    /// snapshot when `update` is called.
    ///
    /// The replacement value is restored before dependents are invalidated
    /// and subscription notifications are queued.
    pub fn update(&self, closure: impl FnOnce(&mut T)) -> SignalReadGuard<T> {
        let inner = self.inner();

        let mut value = inner
            .value
            .borrow_mut()
            .take()
            .expect("signal value temporarily unavailable");

        closure(
            Rc::get_mut(&mut value)
                .expect("signal value is still shared during update"),
        );

        *inner.value.borrow_mut() = Some(value);

        runtime::invalidate(&inner.source);
        queue_notifications(&inner);

        self.inspect()
    }

    /// Subscribes to changes in this signal.
    ///
    /// Notifications are deferred until queued work is processed.
    pub fn subscribe(&self, closure: impl Fn(&T) + 'static) -> Subscription {
        let inner = self.inner();
        let (subscription, active) = Subscription::create();

        inner.subscribers.borrow_mut().push(SignalSubscriber {
            active,
            pending: Rc::new(Cell::new(false)),
            callback: Rc::new(closure),
        });

        subscription
    }
}

impl<T: Copy + 'static> Signal<T> {
    /// Returns a copy of the current value and tracks the active observer.
    pub fn value(&self) -> T {
        *self.get()
    }

    /// Returns a copy of the current value without tracking a dependency.
    pub fn inspect_value(&self) -> T {
        *self.inspect()
    }
}

impl<T: Debug + 'static> Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Signal")
            .field(self.inspect().deref())
            .finish()
    }
}

impl<T: 'static> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for Signal<T> {}

impl<T: 'static> std::hash::Hash for Signal<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Creates or retrieves a signal resource in the active scope.
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    let id = signal_resource(|events| SignalInner {
        value: RefCell::new(Some(Rc::new(value))),
        source: SourceNode::new(),
        events,
        subscribers: RefCell::new(Vec::new()),
    });

    let handle = Signal {
        id,
        marker: PhantomData,
    };

    // Creating a signal inside a scope establishes the scope's dependency
    // on that signal, even if its value is not explicitly read.
    let inner = handle.inner();
    runtime::track(&inner.source);

    handle
}

fn queue_notifications<T: 'static>(inner: &Rc<SignalInner<T>>) {
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
        let events = Rc::clone(&inner.events);

        events.queue_work(move || {
            pending.set(false);

            if !active.get() {
                return;
            }

            // Clone the current snapshot before invoking user code. This
            // avoids holding a RefCell borrow while running the callback.
            let value = inner
                .value
                .borrow()
                .as_ref()
                .expect("signal value temporarily unavailable")
                .clone();

            callback(value.as_ref());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn create_test_signal<T: 'static>(
        value: T,
    ) -> (crate::scope::Scope<Signal<T>>, Signal<T>) {
        let value = Rc::new(RefCell::new(Some(value)));
        let value_for_scope = Rc::clone(&value);

        let mut scope = crate::scope::scope(move || {
            signal(
                value_for_scope
                    .borrow_mut()
                    .take()
                    .expect("test signal should only be created once"),
            )
        });

        let handle = *scope.run();

        (scope, handle)
    }

    fn events<T: 'static>(signal: &Signal<T>) -> Rc<ScopeEventState> {
        Rc::clone(&signal.inner().events)
    }

    #[test]
    fn subscription_callback_is_deferred() {
        let (_scope, signal) = create_test_signal(10);
        let events = events(&signal);
        let received = Rc::new(RefCell::new(Vec::new()));

        let callback_received = Rc::clone(&received);
        let _subscription = signal.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        signal.set(20);

        assert!(received.borrow().is_empty());

        let work = events.pop_work().expect("expected queued notification");
        work();

        assert_eq!(*received.borrow(), vec![20]);
        assert!(events.pop_work().is_none());
    }

    #[test]
    fn repeated_writes_coalesce_while_notification_is_pending() {
        let (_scope, signal) = create_test_signal(1);
        let events = events(&signal);
        let received = Rc::new(RefCell::new(Vec::new()));

        let callback_received = Rc::clone(&received);
        let _subscription = signal.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        signal.set(2);
        signal.set(3);
        signal.set(4);

        let work = events.pop_work().expect("expected queued notification");
        assert!(events.pop_work().is_none());

        work();

        assert_eq!(*received.borrow(), vec![4]);
    }

    #[test]
    fn unsubscribe_cancels_queued_notification() {
        let (_scope, signal) = create_test_signal(10);
        let events = events(&signal);
        let received = Rc::new(RefCell::new(Vec::new()));

        let callback_received = Rc::clone(&received);
        let subscription = signal.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        signal.set(20);
        subscription.unsubscribe();

        let work = events.pop_work().expect("expected queued notification");
        work();

        assert!(received.borrow().is_empty());
    }

    #[test]
    fn independent_subscriptions_receive_independent_notifications() {
        let (_scope, signal) = create_test_signal(0);
        let events = events(&signal);
        let first_received = Rc::new(RefCell::new(Vec::new()));
        let second_received = Rc::new(RefCell::new(Vec::new()));

        let first_callback = Rc::clone(&first_received);
        let _first_subscription = signal.subscribe(move |value| {
            first_callback.borrow_mut().push(*value);
        });

        let second_callback = Rc::clone(&second_received);
        let _second_subscription = signal.subscribe(move |value| {
            second_callback.borrow_mut().push(*value);
        });

        signal.set(42);

        let first_work = events.pop_work().expect("expected first notification");
        let second_work = events.pop_work().expect("expected second notification");

        assert!(events.pop_work().is_none());

        first_work();
        second_work();

        assert_eq!(*first_received.borrow(), vec![42]);
        assert_eq!(*second_received.borrow(), vec![42]);
    }

    #[test]
    fn unsubscribed_subscription_does_not_receive_future_notifications() {
        let (_scope, signal) = create_test_signal(0);
        let events = events(&signal);
        let received = Rc::new(RefCell::new(Vec::new()));

        let callback_received = Rc::clone(&received);
        let subscription = signal.subscribe(move |value| {
            callback_received.borrow_mut().push(*value);
        });

        subscription.unsubscribe();
        signal.set(1);

        assert!(events.pop_work().is_none());
        assert!(received.borrow().is_empty());
    }

    #[test]
    fn each_subscription_coalesces_its_own_pending_notifications() {
        let (_scope, signal) = create_test_signal(0);
        let events = events(&signal);
        let first_received = Rc::new(RefCell::new(Vec::new()));
        let second_received = Rc::new(RefCell::new(Vec::new()));

        let first_callback = Rc::clone(&first_received);
        let _first_subscription = signal.subscribe(move |value| {
            first_callback.borrow_mut().push(*value);
        });

        let second_callback = Rc::clone(&second_received);
        let _second_subscription = signal.subscribe(move |value| {
            second_callback.borrow_mut().push(*value);
        });

        signal.set(1);
        signal.set(2);

        let first_work = events.pop_work().expect("expected first notification");
        let second_work = events.pop_work().expect("expected second notification");

        assert!(events.pop_work().is_none());

        first_work();
        second_work();

        assert_eq!(*first_received.borrow(), vec![2]);
        assert_eq!(*second_received.borrow(), vec![2]);
    }

    #[test]
    fn callback_can_be_called_again_after_its_notification_runs() {
        let (_scope, signal) = create_test_signal(0);
        let events = events(&signal);
        let calls = Rc::new(Cell::new(0));

        let callback_calls = Rc::clone(&calls);
        let _subscription = signal.subscribe(move |_| {
            callback_calls.set(callback_calls.get() + 1);
        });

        signal.set(1);
        events.pop_work().expect("expected first notification")();

        assert_eq!(calls.get(), 1);

        signal.set(2);
        events.pop_work().expect("expected second notification")();

        assert_eq!(calls.get(), 2);
        assert!(events.pop_work().is_none());
    }

    #[test]
    fn read_guard_keeps_its_snapshot_after_set() {
        let (_scope, signal) = create_test_signal(String::from("before"));

        let snapshot = signal.inspect();
        signal.set(String::from("after"));

        assert_eq!(&*snapshot, "before");
        assert_eq!(&*signal.inspect(), "after");
    }

    #[test]
    fn update_mutates_non_clone_value() {
        struct NonClone {
            value: String,
        }

        let (_scope, signal) = create_test_signal(NonClone {
            value: String::from("before"),
        });

        signal.update(|value| {
            value.value.push_str("-after");
        });

        assert_eq!(signal.inspect().value.value, "before-after");
    }

    #[test]
    fn update_returns_the_updated_value() {
        let (_scope, signal) = create_test_signal(10);

        let updated = signal.update(|value| {
            *value += 5;
        });

        assert_eq!(*updated, 15);
        assert_eq!(signal.value(), 15);
    }

    #[test]
    fn signal_handle_is_copy_without_t_copy() {
        struct NonCopy(String);

        let (_scope, signal) = create_test_signal(NonCopy(String::from("value")));

        let copied_handle = signal;
        let another_handle = signal;

        assert_eq!(copied_handle.inspect().0, "value");
        assert_eq!(another_handle.inspect().0, "value");
    }
}

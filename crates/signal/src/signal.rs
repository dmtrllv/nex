use std::{
    cell::{Cell, Ref, RefCell},
    fmt::Debug,
    ops::Deref,
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ScopeEventState, SourceNode},
    scope::signal_resource,
};

struct SignalSubscriber<T> {
    active: Rc<Cell<bool>>,
    pending: Rc<Cell<bool>>,
    callback: Rc<dyn Fn(&T)>,
}

struct SignalInner<T> {
    value: RefCell<Option<T>>,
    source: Rc<SourceNode>,
    events: Rc<ScopeEventState>,
    subscribers: RefCell<Vec<SignalSubscriber<T>>>,
}

/// A handle to a reactive value.
#[derive(Clone)]
pub struct Signal<T: 'static> {
    inner: Rc<SignalInner<T>>,
}

impl<T> Signal<T> {
    /// Reads the current value and tracks the active observer.
    pub fn get(&self) -> Ref<'_, T> {
        runtime::track(&self.inner.source);

        Ref::map(self.inner.value.borrow(), |value| {
            value
                .as_ref()
                .expect("signal value temporarily unavailable")
        })
    }

    /// Reads the current value without tracking a dependency.
    pub fn inspect(&self) -> Ref<'_, T> {
        Ref::map(self.inner.value.borrow(), |value| {
            value
                .as_ref()
                .expect("signal value temporarily unavailable")
        })
    }

    /// Replaces the current value and invalidates dependent observers.
    pub fn set(&self, value: impl Into<T>) {
        *self.inner.value.borrow_mut() = Some(value.into());

        runtime::invalidate(&self.inner.source);
        queue_notifications(&self.inner);
    }

    /// Replaces the current value with the result of `closure`.
    pub fn update(&self, closure: impl FnOnce(T) -> T) -> Ref<'_, T> {
        let current = self
            .inner
            .value
            .borrow_mut()
            .take()
            .expect("signal value temporarily unavailable");

        let next = closure(current);

        *self.inner.value.borrow_mut() = Some(next);

        runtime::invalidate(&self.inner.source);
        queue_notifications(&self.inner);
        self.inspect()
    }

    /// Subscribes to changes in this signal.
    pub fn subscribe(&self, closure: impl Fn(&T) + 'static) -> Subscription {
        let (subscription, active) = Subscription::create();

        self.inner.subscribers.borrow_mut().push(SignalSubscriber {
            active,
            pending: Rc::new(Cell::new(false)),
            callback: Rc::new(closure),
        });

        subscription
    }
}

impl<T: Copy> Signal<T> {
    /// Returns a copy of the current value and tracks the active observer.
    pub fn value(&self) -> T {
        *self.get()
    }

    /// Returns a copy of the current value without tracking a dependency.
    pub fn inspect_value(&self) -> T {
        *self.inspect()
    }
}

impl<T: Debug> Debug for Signal<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Signal")
            .field(self.inspect().deref())
            .finish()
    }
}

/// Creates or retrieves a signal resource in the active scope.
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    let inner = signal_resource(|events| SignalInner {
        value: RefCell::new(Some(value)),
        source: SourceNode::new(),
        events,
        subscribers: RefCell::new(Vec::new()),
    });

	runtime::track(&inner.source);

    Signal { inner }
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

            let value = inner.value.borrow();
            let value = value
                .as_ref()
                .expect("signal value temporarily unavailable");

            callback(value);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn create_test_signal<T: 'static>(value: T) -> Signal<T> {
        Signal {
            inner: Rc::new(SignalInner {
                value: RefCell::new(Some(value)),
                source: SourceNode::new(),
                events: Rc::new(ScopeEventState::default()),
                subscribers: RefCell::new(Vec::new()),
            }),
        }
    }

    #[test]
    fn subscription_callback_is_deferred() {
        let signal = create_test_signal(10);
        let received = Rc::new(RefCell::new(Vec::new()));

        let received_callback = Rc::clone(&received);
        let _subscription = signal.subscribe(move |value| {
            received_callback.borrow_mut().push(*value);
        });

        signal.set(20);

        // Writing the signal must not invoke the callback synchronously.
        assert!(received.borrow().is_empty());

        // Execute the queued notification.
        let work = signal
            .inner
            .events
            .pop_work()
            .expect("expected queued notification");

        work();

        assert_eq!(*received.borrow(), vec![20]);
        assert!(signal.inner.events.pop_work().is_none());
    }

    #[test]
    fn repeated_writes_coalesce_while_notification_is_pending() {
        let signal = create_test_signal(1);
        let received = Rc::new(RefCell::new(Vec::new()));

        let received_callback = Rc::clone(&received);
        let _subscription = signal.subscribe(move |value| {
            received_callback.borrow_mut().push(*value);
        });

        signal.set(2);
        signal.set(3);
        signal.set(4);

        // Only one notification should be queued for this subscription.
        let work = signal
            .inner
            .events
            .pop_work()
            .expect("expected queued notification");

        assert!(
            signal.inner.events.pop_work().is_none(),
            "repeated writes should not queue duplicate notifications"
        );

        // The callback should observe the latest value, not intermediate values.
        work();

        assert_eq!(*received.borrow(), vec![4]);
    }

    #[test]
    fn unsubscribe_cancels_queued_notification() {
        let signal = create_test_signal(10);
        let received = Rc::new(RefCell::new(Vec::new()));

        let received_callback = Rc::clone(&received);
        let subscription = signal.subscribe(move |value| {
            received_callback.borrow_mut().push(*value);
        });

        signal.set(20);

        // The notification is already queued when the subscription is removed.
        subscription.unsubscribe();

        let work = signal
            .inner
            .events
            .pop_work()
            .expect("expected queued notification");

        work();

        assert!(
            received.borrow().is_empty(),
            "an unsubscribed callback must not run, even if its notification was queued"
        );
    }

    #[test]
    fn independent_subscriptions_receive_independent_notifications() {
        let signal = create_test_signal(0);
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

        assert!(first_received.borrow().is_empty());
        assert!(second_received.borrow().is_empty());

        // Each subscription should have its own queued notification.
        let first_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected first subscription notification");

        let second_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected second subscription notification");

        assert!(
            signal.inner.events.pop_work().is_none(),
            "there should be exactly one queued notification per subscription"
        );

        first_work();
        second_work();

        assert_eq!(*first_received.borrow(), vec![42]);
        assert_eq!(*second_received.borrow(), vec![42]);
    }

    #[test]
    fn unsubscribed_subscription_does_not_receive_future_notifications() {
        let signal = create_test_signal(0);
        let received = Rc::new(RefCell::new(Vec::new()));

        let received_callback = Rc::clone(&received);
        let subscription = signal.subscribe(move |value| {
            received_callback.borrow_mut().push(*value);
        });

        subscription.unsubscribe();

        signal.set(1);

        assert!(
            signal.inner.events.pop_work().is_none(),
            "an inactive subscription should not queue a notification"
        );
        assert!(received.borrow().is_empty());
    }

    #[test]
    fn each_subscription_coalesces_its_own_pending_notifications() {
        let signal = create_test_signal(0);
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

        let first_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected first subscription notification");

        let second_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected second subscription notification");

        assert!(
            signal.inner.events.pop_work().is_none(),
            "each subscription should have at most one pending notification"
        );

        first_work();
        second_work();

        assert_eq!(*first_received.borrow(), vec![2]);
        assert_eq!(*second_received.borrow(), vec![2]);
    }

    #[test]
    fn callback_can_be_called_again_after_its_notification_runs() {
        let signal = create_test_signal(0);
        let call_count = Rc::new(Cell::new(0));

        let callback_count = Rc::clone(&call_count);
        let _subscription = signal.subscribe(move |_| {
            callback_count.set(callback_count.get() + 1);
        });

        signal.set(1);

        let first_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected first notification");
        first_work();

        assert_eq!(call_count.get(), 1);

        // Once the first notification has run, another write should queue work.
        signal.set(2);

        let second_work = signal
            .inner
            .events
            .pop_work()
            .expect("expected second notification");
        second_work();

        assert_eq!(call_count.get(), 2);
        assert!(signal.inner.events.pop_work().is_none());
    }
}

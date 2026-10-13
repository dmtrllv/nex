use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState},
};

enum Slot {
    Signal {
        type_id: TypeId,
        value: Box<dyn Any>,
    },
}

type ScopeSubscriberFn<T> = dyn Fn(&mut Scope<T>, ScopeEvent);

struct ScopeSubscriber<T> {
    active: Rc<Cell<bool>>,
    callback: Rc<ScopeSubscriberFn<T>>,
}

struct ResourceStore {
    resources: Vec<Slot>,
    cursor: usize,
    events: Rc<ScopeEventState>,
}

impl ResourceStore {
    fn new(events: Rc<ScopeEventState>) -> Self {
        Self {
            resources: Vec::new(),
            cursor: 0,
            events,
        }
    }
}

pub struct Scope<T> {
    closure: Box<dyn Fn() -> T>,
    resources: Rc<RefCell<ResourceStore>>,
    result: Option<T>,
    //work_queue: VecDeque<Work>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
    subscribers: Vec<ScopeSubscriber<T>>,
}

thread_local! {
    static ACTIVE_SCOPE: RefCell<Option<Rc<RefCell<ResourceStore>>>> =
        const { RefCell::new(None) };
}

struct ActiveScopeGuard {
    previous: Option<Rc<RefCell<ResourceStore>>>,
}

impl ActiveScopeGuard {
    fn enter(resources: Rc<RefCell<ResourceStore>>) -> Self {
        let previous = ACTIVE_SCOPE.with(|active| active.replace(Some(resources)));

        Self { previous }
    }
}

impl Drop for ActiveScopeGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();

        ACTIVE_SCOPE.with(|active| {
            active.replace(previous);
        });
    }
}

/// Retrieves a signal resource from the active scope, creating it if needed.
///
/// Resource identity is positional: the resource at the current cursor is
/// reused when its stored type matches `T`.
pub(crate) fn signal_resource<T: 'static>(create: impl FnOnce(Rc<ScopeEventState>) -> T) -> Rc<T> {
    ACTIVE_SCOPE.with(|active| {
        let resources = active
            .borrow()
            .as_ref()
            .cloned()
            .expect("signal() called outside an active scope");

        let mut store = resources.borrow_mut();
        let index = store.cursor;
        store.cursor += 1;

        if index < store.resources.len() {
            match &store.resources[index] {
                Slot::Signal { type_id, value } => {
                    assert_eq!(
                        *type_id,
                        TypeId::of::<T>(),
                        "signal resource type mismatch at slot {index}",
                    );

                    return value
                        .downcast_ref::<Rc<T>>()
                        .expect("signal resource storage type mismatch")
                        .clone();
                }
            }
        }

        let value = Rc::new(create(Rc::clone(&store.events)));

        store.resources.push(Slot::Signal {
            type_id: TypeId::of::<T>(),
            value: Box::new(Rc::clone(&value)),
        });

        value
    })
}

impl<T> Scope<T> {
    pub fn flush(&mut self) {
        loop {
            self.dispatch_events();

            let Some(work) = self.events.pop_work() else {
                if self.events.has_events() {
                    continue;
                }

                break;
            };

            work();
        }
    }

    /// Evaluates the scope if it has no cached result or was invalidated.
    ///
    /// Resources are reused by their positions in the scope's execution.
    pub fn run(&mut self) -> &T {
        if self.result.is_none() || self.observer.is_invalidated() {
            // reset the resource cursor
            {
                let mut store = self.resources.borrow_mut();
                store.cursor = 0;
            }

            let result = {
                self.observer.mark_valid();
                runtime::clear_dependencies(&self.observer);

                let _scope_guard = ActiveScopeGuard::enter(Rc::clone(&self.resources));
                let _observer_guard = ObserverGuard::enter(Rc::clone(&self.observer));

                (self.closure)()
            };

            {
                let mut store = self.resources.borrow_mut();
                let used_resources = store.cursor;
                store.resources.truncate(used_resources);
            }

            self.result = Some(result);
        }

        self.result
            .as_ref()
            .expect("scope result should exist after run")
    }

    pub fn subscribe(
        &mut self,
        callback: impl Fn(&mut Self, ScopeEvent) + 'static,
    ) -> Subscription {
        let (subscription, active) = Subscription::create();

        self.subscribers.push(ScopeSubscriber {
            active,
            callback: Rc::new(callback),
        });

        subscription
    }

    pub fn result(&self) -> Option<&T> {
        self.result.as_ref()
    }

    fn dispatch_events(&mut self) {
        // Take a snapshot so events produced by callbacks are deferred
        // until the next dispatch pass.
        let events: Vec<_> = std::iter::from_fn(|| self.events.pop()).collect();

        for event in events {
            let subscribers: Vec<_> = self
                .subscribers
                .iter()
                .map(|subscriber| {
                    (
                        Rc::clone(&subscriber.active),
                        Rc::clone(&subscriber.callback),
                    )
                })
                .collect();

            for (active, callback) in subscribers {
                if active.get() {
                    callback(self, event);
                }
            }
        }

        self.subscribers
            .retain(|subscriber| subscriber.active.get());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeEvent {
    Invalidated,
    Pending,
}

pub fn scope<T>(closure: impl Fn() -> T + 'static) -> Scope<T> {
    let events = Rc::new(ScopeEventState::default());
    let observer = ObserverNode::new_for_scope(&events);

    Scope {
        closure: Box::new(closure),
        resources: Rc::new(RefCell::new(ResourceStore::new(Rc::clone(&events)))),
        result: None,
        //work_queue: VecDeque::new(),
        observer,
        events,
        subscribers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[test]
    fn tracked_signal_change_invalidates_scope_and_queues_event() {
        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);

        let mut scope = scope(move || {
            let count = crate::signal::signal(0);
            let _ = count.value();

            *handle.borrow_mut() = Some(count);
        });

        scope.run();
        assert!(!scope.observer.is_invalidated());
        assert_eq!(scope.events.pop(), None);

        count_handle.borrow().as_ref().unwrap().set(1);

        assert!(scope.observer.is_invalidated());
        assert_eq!(scope.events.pop(), Some(ScopeEvent::Invalidated));
        assert_eq!(scope.events.pop(), None);

        scope.run();
        assert!(!scope.observer.is_invalidated());
        assert_eq!(count_handle.borrow().as_ref().unwrap().value(), 1);
    }

    #[test]
    fn scope_reuses_cached_result_until_invalidated() {
        let executions = Rc::new(Cell::new(0));
        let execution_count = Rc::clone(&executions);
        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);

        let mut scope = scope(move || {
            execution_count.set(execution_count.get() + 1);

            let count = crate::signal::signal(0);
            let value = count.value();

            *handle.borrow_mut() = Some(count);
            value
        });

        assert_eq!(*scope.run(), 0);
        assert_eq!(executions.get(), 1);

        // A valid scope returns its cached result.
        assert_eq!(*scope.run(), 0);
        assert_eq!(executions.get(), 1);

        // Updating the tracked signal invalidates the scope.
        count_handle.borrow().as_ref().unwrap().set(1);

        assert!(scope.observer.is_invalidated());
        assert_eq!(*scope.run(), 1);
        assert_eq!(executions.get(), 2);

        // The new result is cached too.
        assert_eq!(*scope.run(), 1);
        assert_eq!(executions.get(), 2);
    }

    #[test]
    fn tracked_signal_change_queues_scope_invalidation_event() {
        let mut scope = scope(|| {
            let count = crate::signal::signal(0);
            let _ = count.value();
            count
        });

        assert_eq!(scope.run().value(), 0);
        assert!(!scope.observer.is_invalidated());
        assert_eq!(scope.events.pop(), None);

        scope.run().set(1);

        assert!(scope.observer.is_invalidated());
        assert_eq!(scope.events.pop(), Some(ScopeEvent::Invalidated));
        assert_eq!(scope.events.pop(), None);

        assert_eq!(scope.run().value(), 1);
        assert!(!scope.observer.is_invalidated());
    }

    #[test]
    fn scope_subscription_receives_invalidation_event() {
        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_by_callback = Rc::clone(&received);

        let mut scope = scope(move || {
            let count = crate::signal::signal(0);
            let _ = count.value();
            *handle.borrow_mut() = Some(count);
        });

        scope.run();

        let _subscription = scope.subscribe(move |_, event| {
            received_by_callback.borrow_mut().push(event);
        });

        count_handle.borrow().as_ref().unwrap().set(1);

        assert!(received.borrow().is_empty());

        scope.dispatch_events();

        assert_eq!(*received.borrow(), vec![ScopeEvent::Invalidated]);
    }

    #[test]
    fn unsubscribed_scope_subscription_receives_no_events() {
        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);
        let calls = Rc::new(Cell::new(0));
        let calls_by_callback = Rc::clone(&calls);

        let mut scope = scope(move || {
            let count = crate::signal::signal(0);
            let _ = count.value();
            *handle.borrow_mut() = Some(count);
        });

        scope.run();

        let subscription = scope.subscribe(move |_, _| {
            calls_by_callback.set(calls_by_callback.get() + 1);
        });

        subscription.unsubscribe();
        count_handle.borrow().as_ref().unwrap().set(1);

        scope.dispatch_events();

        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn flush_executes_queued_signal_subscription() {
        let signal_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&signal_handle);
        let received = Rc::new(RefCell::new(Vec::new()));
        let received_callback = Rc::clone(&received);

        let mut scope = scope(move || {
            let signal = crate::signal::signal(0);
			let received_callback = received_callback.clone();
            let _subscription = signal.subscribe(move |value| {
                received_callback.borrow_mut().push(*value);
            });

            *handle.borrow_mut() = Some(signal);
        });

        scope.run();

        signal_handle.borrow().as_ref().unwrap().set(10);

        assert!(received.borrow().is_empty());

        scope.flush();

        assert_eq!(*received.borrow(), vec![10]);
    }

    #[test]
    fn flush_processes_work_queued_by_another_work_item() {
        let events = Rc::new(ScopeEventState::default());
        let calls = Rc::new(Cell::new(0));

        let mut scope = Scope {
            closure: Box::new(|| ()),
            resources: Rc::new(RefCell::new(ResourceStore::new(Rc::clone(&events)))),
            result: None,
            observer: ObserverNode::new_for_scope(&events),
            events: Rc::clone(&events),
            subscribers: Vec::new(),
        };

        let nested_events = Rc::clone(&events);
        let nested_calls = Rc::clone(&calls);

        events.queue_work(move || {
            nested_calls.set(nested_calls.get() + 1);

            let nested_calls = Rc::clone(&nested_calls);
            nested_events.queue_work(move || {
                nested_calls.set(nested_calls.get() + 1);
            });
        });

        scope.flush();

        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn flush_delivers_scope_events_without_queued_work() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let callback_calls = Rc::clone(&calls);

        let mut scope = scope(|| ());
        let _subscription = scope.subscribe(move |_, event| {
            callback_calls.borrow_mut().push(event);
        });

        scope.events.queue_invalidated();

        scope.flush();

        assert_eq!(*calls.borrow(), vec![ScopeEvent::Invalidated]);
    }
}

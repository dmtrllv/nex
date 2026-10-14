use std::{
    any::{Any, TypeId},
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResourceKind {
    Signal,
    Memo,
    Effect,
}

enum Slot {
    Signal {
        type_id: TypeId,
        value: Box<dyn Any>,
    },
    Memo {
        type_id: TypeId,
        value: Box<dyn Any>,
    },
    Effect {
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

    fn get_or_insert<T: 'static>(
        &mut self,
        kind: ResourceKind,
        create: impl FnOnce() -> T,
    ) -> Rc<T> {
        let index = self.cursor;
        self.cursor += 1;

        if index < self.resources.len() {
            let slot = &self.resources[index];

            let (type_id, value) = match (kind, slot) {
                (ResourceKind::Signal, Slot::Signal { type_id, value })
                | (ResourceKind::Effect, Slot::Effect { type_id, value })
                | (ResourceKind::Memo, Slot::Memo { type_id, value }) => (type_id, value),
                _ => {
                    panic!("resource kind changed at slot {index}");
                }
            };

            assert_eq!(
                *type_id,
                TypeId::of::<T>(),
                "resource type changed at slot {index}",
            );

            return Rc::clone(
                value
                    .downcast_ref::<Rc<T>>()
                    .expect("resource slot type metadata is inconsistent"),
            );
        }

        let value = Rc::new(create());

        let slot = match kind {
            ResourceKind::Signal => Slot::Signal {
                type_id: TypeId::of::<T>(),
                value: Box::new(Rc::clone(&value)),
            },
            ResourceKind::Memo => Slot::Memo {
                type_id: TypeId::of::<T>(),
                value: Box::new(Rc::clone(&value)),
            },
            ResourceKind::Effect => Slot::Effect {
                type_id: TypeId::of::<T>(),
                value: Box::new(Rc::clone(&value)),
            },
        };

        self.resources.push(slot);

        value
    }
}

struct FlushGuard(Rc<Cell<bool>>);

impl Drop for FlushGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

pub struct Scope<T> {
    closure: Box<dyn Fn() -> T>,
    resources: Rc<RefCell<ResourceStore>>,
    result: Option<T>,
    //work_queue: VecDeque<Work>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
    flushing: Rc<Cell<bool>>,
    subscribers: Vec<ScopeSubscriber<T>>,
}


impl<T> Scope<T> {
    /// Processes pending scope events and reactive work.
    ///
    /// A Pending event is dispatched before the queued work it announces.
    /// After the callback returns, this method processes the work before
    /// dispatching other scope events. Work queued during processing is
    /// handled by the same flush loop.
    pub fn flush(&mut self) {
        if self.flushing.replace(true) {
            return;
        }

        let _guard = FlushGuard(Rc::clone(&self.flushing));

        loop {
            // Announce pending work before executing it.
            if self.events.has_work() {
                if let Some(event) = self.events.pop_pending() {
                    self.dispatch_event(event);
                    continue;
                }

                if let Some(work) = self.events.pop_work() {
                    work();
                    continue;
                }
            }

            // With no work waiting, dispatch any remaining scope events.
            if let Some(event) = self.events.pop() {
                self.dispatch_event(event);
                continue;
            }

            break;
        }
    }

    /// Processes queued reactive work and evaluates the scope if needed.
    ///
    /// Events and work are processed before the cached result is checked.
    /// Resources are reused by their positions in the scope's execution.
    pub fn run(&mut self) -> &T {
        self.flush();

        if self.result.is_none() || self.observer.is_invalidated() {
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

    fn dispatch_event(&mut self, event: ScopeEvent) {
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

        self.subscribers
            .retain(|subscriber| subscriber.active.get());
    }
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
/// Resource identity is positional. The resource at the current cursor is
/// reused only when its kind and concrete type match.
pub(crate) fn signal_resource<T: 'static>(create: impl FnOnce(Rc<ScopeEventState>) -> T) -> Rc<T> {
    let store = ACTIVE_SCOPE.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .expect("signals must be created inside an active scope")
    });

    let events = Rc::clone(&store.borrow().events);

    store
        .borrow_mut()
        .get_or_insert(ResourceKind::Signal, || create(events))
}

/// Retrieves a memo resource from the active scope, creating it if needed.
///
/// Resource identity is positional. The resource at the current cursor is
/// reused only when its kind and concrete type match.
pub(crate) fn memo_resource<T: 'static>(create: impl FnOnce(Rc<ScopeEventState>) -> T) -> Rc<T> {
    let store = ACTIVE_SCOPE.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .expect("memos must be created inside an active scope")
    });

    let events = Rc::clone(&store.borrow().events);

    store
        .borrow_mut()
        .get_or_insert(ResourceKind::Memo, || create(events))
}

/// Retrieves an effect resource from the active scope, creating it if needed.
///
/// Resource identity is positional. The resource at the current cursor is
/// reused only when its kind and concrete type match.
pub(crate) fn effect_resource<T: 'static>(create: impl FnOnce(Rc<ScopeEventState>) -> T) -> Rc<T> {
    let store = ACTIVE_SCOPE.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .expect("effects must be created inside an active scope")
    });

    let events = Rc::clone(&store.borrow().events);

    store
        .borrow_mut()
        .get_or_insert(ResourceKind::Effect, || create(events))
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
        observer,
        events,
        flushing: Rc::new(Cell::new(false)),
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

        scope.flush();

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

        scope.flush();

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
            flushing: Rc::new(Cell::new(false)),
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

    #[test]
    fn run_dispatches_invalidation_and_allows_reentrant_run() {
        let signal_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&signal_handle);
        let invalidations = Rc::new(Cell::new(0));
        let callback_invalidations = Rc::clone(&invalidations);

        let mut app = scope(move || {
            let count = crate::signal::signal(0);
            let _ = count.value();
            *handle.borrow_mut() = Some(count.clone());
            count
        });

        app.run();

        let _subscription = app.subscribe(move |app, event| match event {
            ScopeEvent::Invalidated => {
                callback_invalidations.set(callback_invalidations.get() + 1);

                let count = app.run();
                assert_eq!(count.value(), 1);
            }
            ScopeEvent::Pending => {
                app.flush();
            }
        });

        signal_handle.borrow().as_ref().unwrap().set(1);

        // The write queues events; it does not synchronously invoke
        // the scope subscription.
        assert_eq!(invalidations.get(), 0);

        // run() is a processing boundary and dispatches the queued event.
        assert_eq!(app.run().value(), 1);
        assert_eq!(invalidations.get(), 1);
    }

    #[test]
    fn scope_tracks_created_signal_without_reading_it() {
        let mut app = scope(|| crate::signal::signal(0));

        let count = app.run().clone();

        assert!(!app.observer.is_invalidated());

        count.set(1);

        assert!(
            app.observer.is_invalidated(),
            "creating a signal inside the scope should make the scope depend on it"
        );

        assert_eq!(app.events.pop(), Some(ScopeEvent::Invalidated));
    }

    #[test]
    fn flush_processes_effects_before_scope_invalidation_event() {
        use crate::{effect, signal};

        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);

        let effect_values = Rc::new(RefCell::new(Vec::new()));
        let values_from_effect = Rc::clone(&effect_values);

        let mut app = scope(move || {
            let count = signal(0);

            // The scope directly depends on this signal.
            let _ = count.value();

            let effect_count = count.clone();
            let values = Rc::clone(&values_from_effect);

            effect(move || {
                values.borrow_mut().push(effect_count.value());
            });

            *handle.borrow_mut() = Some(count.clone());
            count
        });

        app.run();

        // Process the effect's initial execution.
        app.flush();
        assert_eq!(*effect_values.borrow(), vec![0]);

        let events = Rc::new(RefCell::new(Vec::new()));
        let received_events = Rc::clone(&events);

        let _subscription = app.subscribe(move |_, event| {
            received_events.borrow_mut().push(event);
        });

        count_handle.borrow().as_ref().unwrap().set(1);
        app.flush();

        assert_eq!(*effect_values.borrow(), vec![0, 1]);

        assert_eq!(
            *events.borrow(),
            vec![ScopeEvent::Pending, ScopeEvent::Invalidated]
        );
    }

    #[test]
    fn flush_runs_effect_before_invalidation_callback() {
        use crate::{effect, signal};

        let count_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&count_handle);

        let effect_value = Rc::new(Cell::new(-1));
        let effect_value_for_closure = Rc::clone(&effect_value);

        let mut app = scope(move || {
            let count = signal(0);
            let _ = count.value();
            let effect_value_for_closure = effect_value_for_closure.clone();
            let effect_count = count.clone();
            effect(move || {
                effect_value_for_closure.set(effect_count.value());
            });

            *handle.borrow_mut() = Some(count.clone());
            count
        });

        app.run();
        app.flush();

        let observed_effect_value = Rc::new(Cell::new(-1));
        let observed_by_callback = Rc::clone(&observed_effect_value);
        let effect_value_for_callback = Rc::clone(&effect_value);

        let _subscription = app.subscribe(move |_, event| {
            if event == ScopeEvent::Invalidated {
                observed_by_callback.set(effect_value_for_callback.get());
            }
        });

        count_handle.borrow().as_ref().unwrap().set(1);
        app.flush();

        assert_eq!(effect_value.get(), 1);
        assert_eq!(
            observed_effect_value.get(),
            1,
            "the effect must run before the invalidation callback"
        );
    }

    #[test]
    fn pending_event_is_dispatched_before_queued_work() {
        let events = Rc::new(ScopeEventState::default());
        let mut app = Scope {
            closure: Box::new(|| ()),
            resources: Rc::new(RefCell::new(ResourceStore::new(Rc::clone(&events)))),
            result: None,
            observer: ObserverNode::new_for_scope(&events),
            events: Rc::clone(&events),
            flushing: Rc::new(Cell::new(false)),
            subscribers: Vec::new(),
        };

        let order = Rc::new(RefCell::new(Vec::new()));

        let callback_order = Rc::clone(&order);
        let callback_events = Rc::clone(&events);
        let _subscription = app.subscribe(move |_, event| {
            if event == ScopeEvent::Pending {
                assert!(
                    callback_events.has_work(),
                    "work should still be queued when Pending is delivered"
                );
                callback_order.borrow_mut().push("pending");
            }
        });

        let work_order = Rc::clone(&order);
        events.queue_work(move || {
            work_order.borrow_mut().push("work");
        });

        app.flush();

        assert_eq!(*order.borrow(), vec!["pending", "work"]);
    }
}

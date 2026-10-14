use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
};

use crate::ScopeEvent;

pub(crate) struct SourceNode {
    dependants: RefCell<Vec<Weak<ObserverNode>>>,
}

impl SourceNode {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            dependants: RefCell::new(Vec::new()),
        })
    }
}

pub(crate) struct ObserverNode {
    dependencies: RefCell<Vec<Weak<SourceNode>>>,
    invalidated: Cell<bool>,
    scope_events: Option<Weak<ScopeEventState>>,
    schedule: RefCell<Option<Rc<dyn Fn()>>>,
}

impl ObserverNode {
    /// This is only for testing
    #[allow(unused)]
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            dependencies: RefCell::new(Vec::new()),
            invalidated: Cell::new(false),
            scope_events: None,
            schedule: RefCell::new(None),
        })
    }

    pub(crate) fn new_for_scope(events: &Rc<ScopeEventState>) -> Rc<Self> {
        Rc::new(Self {
            dependencies: RefCell::new(Vec::new()),
            invalidated: Cell::new(false),
            scope_events: Some(Rc::downgrade(events)),
            schedule: RefCell::new(None),
        })
    }

    pub(crate) fn is_invalidated(&self) -> bool {
        self.invalidated.get()
    }

    pub(crate) fn mark_valid(&self) {
        self.invalidated.set(false);
    }

    pub(crate) fn set_schedule(&self, schedule: Rc<dyn Fn()>) {
        *self.schedule.borrow_mut() = Some(schedule);
    }

    fn schedule(&self) {
        let schedule = self.schedule.borrow().clone();

        if let Some(schedule) = schedule {
            schedule();
        }
    }

    pub(crate) fn mark_invalidated(&self) {
        self.invalidated.set(true);
    }
}

thread_local! {
    static ACTIVE_OBSERVER: RefCell<Option<Rc<ObserverNode>>> =
        const { RefCell::new(None) };
}

pub(crate) struct ObserverGuard {
    previous: Option<Rc<ObserverNode>>,
}

impl ObserverGuard {
    pub(crate) fn enter(observer: Rc<ObserverNode>) -> Self {
        let previous = ACTIVE_OBSERVER.with(|active| active.replace(Some(observer)));

        Self { previous }
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();

        ACTIVE_OBSERVER.with(|active| {
            active.replace(previous);
        });
    }
}

#[derive(Default)]
pub(crate) struct ScopeEventState {
    queue: RefCell<VecDeque<ScopeEvent>>,
    invalidated_queued: Cell<bool>,
    pending_queued: Cell<bool>,
    work_queue: RefCell<VecDeque<Work>>,
    work_pending: Cell<bool>,
}

impl ScopeEventState {
    pub(crate) fn queue_invalidated(&self) {
        if !self.invalidated_queued.replace(true) {
            self.queue.borrow_mut().push_back(ScopeEvent::Invalidated);
        }
    }

    pub(crate) fn queue_pending(&self) {
        if !self.pending_queued.replace(true) {
            self.queue.borrow_mut().push_back(ScopeEvent::Pending);
        }
    }

    pub(crate) fn pop(&self) -> Option<ScopeEvent> {
        let event = self.queue.borrow_mut().pop_front()?;

        match event {
            ScopeEvent::Invalidated => self.invalidated_queued.set(false),
            ScopeEvent::Pending => self.pending_queued.set(false),
        }

        Some(event)
    }

    pub(crate) fn queue_work(&self, work: impl FnOnce() + 'static) {
        self.work_queue.borrow_mut().push_back(Box::new(work));

        // Emit Pending only when work transitions from absent to present.
        if !self.work_pending.replace(true) {
            self.queue_pending();
        }
    }

    pub(crate) fn has_work(&self) -> bool {
        !self.work_queue.borrow().is_empty()
    }

    /// Removes a queued Pending event even if another event was queued first.
    ///
    /// Used to notify the consumer about pending work before processing it.
    pub(crate) fn pop_pending(&self) -> Option<ScopeEvent> {
        let mut queue = self.queue.borrow_mut();
        let index = queue
            .iter()
            .position(|event| *event == ScopeEvent::Pending)?;

        let event = queue.remove(index);

        if event.is_some() {
            self.pending_queued.set(false);
        }

        event
    }

    pub(crate) fn pop_work(&self) -> Option<Work> {
        let work = self.work_queue.borrow_mut().pop_front();

        if work.is_some() && self.work_queue.borrow().is_empty() {
            self.work_pending.set(false);
        }

        work
    }
}

type Work = Box<dyn FnOnce()>;

/// Registers the active observer as a dependent of `source`.
pub(crate) fn track(source: &Rc<SourceNode>) {
    ACTIVE_OBSERVER.with(|active| {
        let observer = match active.borrow().as_ref().cloned() {
            Some(observer) => observer,
            None => return,
        };

        let inserted = {
            let mut dependencies = observer.dependencies.borrow_mut();
            dependencies.retain(|dependency| dependency.strong_count() > 0);

            if dependencies
                .iter()
                .filter_map(Weak::upgrade)
                .any(|existing| Rc::ptr_eq(&existing, source))
            {
                false
            } else {
                dependencies.push(Rc::downgrade(source));
                true
            }
        };

        if inserted {
            let mut dependents = source.dependants.borrow_mut();
            dependents.retain(|dependent| dependent.strong_count() > 0);

            if !dependents
                .iter()
                .filter_map(Weak::upgrade)
                .any(|existing| Rc::ptr_eq(&existing, &observer))
            {
                dependents.push(Rc::downgrade(&observer));
            }
        }
    });
}

/// Removes the observer's previous dependency edges.
///
/// Call before executing the observer again so its new reads can establish
/// the current dependency set.
pub(crate) fn clear_dependencies(observer: &Rc<ObserverNode>) {
    let previous = {
        let mut dependencies = observer.dependencies.borrow_mut();
        std::mem::take(&mut *dependencies)
    };

    for dependency in previous {
        let Some(source) = dependency.upgrade() else {
            continue;
        };

        source.dependants.borrow_mut().retain(|dependent| {
            dependent
                .upgrade()
                .is_some_and(|existing| !Rc::ptr_eq(&existing, observer))
        });
    }
}

pub(crate) fn invalidate(source: &Rc<SourceNode>) {
    let mut dependants = source.dependants.borrow_mut();

    dependants.retain(|dependant| {
        let Some(observer) = dependant.upgrade() else {
            return false;
        };

        let was_invalidated = observer.invalidated.replace(true);

        if !was_invalidated {
            if let Some(events) = observer.scope_events.as_ref().and_then(Weak::upgrade) {
                events.queue_invalidated();
            }

            observer.schedule();
        }

        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_deduplicates_dependencies() {
        let observer = ObserverNode::new();
        let source = SourceNode::new();

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&source);
            track(&source);
        }

        assert_eq!(observer.dependencies.borrow().len(), 1);
        assert_eq!(source.dependants.borrow().len(), 1);
    }

    #[test]
    fn clearing_dependencies_removes_reverse_edges() {
        let observer = ObserverNode::new();
        let first = SourceNode::new();
        let second = SourceNode::new();

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&first);
        }

        clear_dependencies(&observer);

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&second);
        }

        assert!(
            observer
                .dependencies
                .borrow()
                .iter()
                .filter_map(Weak::upgrade)
                .any(|source| Rc::ptr_eq(&source, &second))
        );

        assert!(first.dependants.borrow().is_empty());
        assert_eq!(second.dependants.borrow().len(), 1);
    }

    #[test]
    fn invalidation_marks_dependent_observer() {
        let source = SourceNode::new();
        let observer = ObserverNode::new();

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&source);
        }

        assert!(!observer.is_invalidated());

        invalidate(&source);

        assert!(observer.is_invalidated());
    }

    #[test]
    fn invalidation_does_not_mark_unrelated_observer() {
        let source = SourceNode::new();
        let observer = ObserverNode::new();

        invalidate(&source);

        assert!(!observer.is_invalidated());
    }

    #[test]
    fn invalidation_removes_dropped_observers() {
        let source = SourceNode::new();
        let observer = ObserverNode::new();

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&source);
        }

        assert_eq!(source.dependants.borrow().len(), 1);

        drop(observer);
        invalidate(&source);

        assert!(source.dependants.borrow().is_empty());
    }

    #[test]
    fn scope_invalidation_queues_one_event() {
        let events = Rc::new(ScopeEventState::default());
        let observer = ObserverNode::new_for_scope(&events);
        let source = SourceNode::new();

        {
            let _guard = ObserverGuard::enter(Rc::clone(&observer));
            track(&source);
        }

        invalidate(&source);
        invalidate(&source);

        assert_eq!(events.pop(), Some(ScopeEvent::Invalidated));
        assert_eq!(events.pop(), None);
    }

    #[test]
    fn pending_and_invalidated_events_are_independent() {
        let events = ScopeEventState::default();

        events.queue_pending();
        events.queue_invalidated();
        events.queue_pending();

        assert_eq!(events.pop(), Some(ScopeEvent::Pending));
        assert_eq!(events.pop(), Some(ScopeEvent::Invalidated));
        assert_eq!(events.pop(), None);
    }

    #[test]
    fn queued_work_is_not_executed_immediately() {
        let events = ScopeEventState::default();
        let calls = Rc::new(Cell::new(0));
        let calls_by_work = Rc::clone(&calls);

        events.queue_work(move || {
            calls_by_work.set(calls_by_work.get() + 1);
        });

        assert_eq!(calls.get(), 0);
        assert_eq!(events.pop(), Some(ScopeEvent::Pending));

        let work = events.pop_work().expect("work should be queued");
        assert_eq!(calls.get(), 0);

        work();

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn queued_work_is_retrieved_in_fifo_order() {
        let events = ScopeEventState::default();
        let order = Rc::new(RefCell::new(Vec::new()));

        for value in [1, 2, 3] {
            let order_by_work = Rc::clone(&order);

            events.queue_work(move || {
                order_by_work.borrow_mut().push(value);
            });
        }

        while let Some(work) = events.pop_work() {
            work();
        }

        assert_eq!(*order.borrow(), vec![1, 2, 3]);
    }

    #[test]
    fn work_queued_during_processing_is_available_later() {
        let events = Rc::new(ScopeEventState::default());
        let order = Rc::new(RefCell::new(Vec::new()));

        let events_by_work = Rc::clone(&events);
        let order_by_work = Rc::clone(&order);

        events.queue_work(move || {
            order_by_work.borrow_mut().push(1);

            let order_by_next_work = Rc::clone(&order_by_work);
            events_by_work.queue_work(move || {
                order_by_next_work.borrow_mut().push(2);
            });
        });

        let first = events.pop_work().expect("first work should be queued");
        first();

        assert_eq!(*order.borrow(), vec![1]);

        let second = events.pop_work().expect("new work should be queued");
        second();

        assert_eq!(*order.borrow(), vec![1, 2]);
    }

    #[test]
    fn scopes_have_independent_work_queues() {
        let first = ScopeEventState::default();
        let second = ScopeEventState::default();
        let calls = Rc::new(Cell::new(0));
        let calls_by_work = Rc::clone(&calls);

        first.queue_work(move || {
            calls_by_work.set(calls_by_work.get() + 1);
        });

        assert!(second.pop_work().is_none());
        assert_eq!(calls.get(), 0);

        let work = first.pop_work().expect("first scope should own its work");
        work();

        assert_eq!(calls.get(), 1);
        assert!(second.pop_work().is_none());
    }
}

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState},
    scope::effect_resource,
};

struct EffectInner {
    compute: RefCell<Box<dyn Fn()>>,
    observer: Rc<ObserverNode>,
    events: Rc<ScopeEventState>,
    scheduled: Cell<bool>,
}

/// Creates or retrieves an effect resource in the active scope.
///
/// The effect runs initially through the scope's work queue and reruns when
/// a tracked dependency is invalidated. Work is processed by `Scope::flush`.
pub fn effect(closure: impl Fn() + 'static) {
    let mut compute: Option<Box<dyn Fn()>> = Some(Box::new(closure));

    let inner = effect_resource(|events| EffectInner {
        compute: RefCell::new(
            compute
                .take()
                .expect("effect computation should be available when creating a resource"),
        ),
        observer: ObserverNode::new(),
        events,
        scheduled: Cell::new(false),
    });

    // Refresh the closure when this resource is reused by a scope rerun.
    if let Some(compute) = compute {
        *inner.compute.borrow_mut() = compute;
    }

    let weak_inner = Rc::downgrade(&inner);

    inner.observer.set_schedule(Rc::new(move || {
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };

        schedule_effect(&inner);
    }));

    // Schedule initial execution too. The scheduled flag coalesces this
    // with any invalidation that occurs before the effect first runs.
    schedule_effect(&inner);
}

fn schedule_effect(inner: &Rc<EffectInner>) {
    if inner.scheduled.replace(true) {
        return;
    }

    let weak_inner = Rc::downgrade(inner);

    inner.events.queue_work(move || {
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };

        inner.scheduled.set(false);

        runtime::clear_dependencies(&inner.observer);
        inner.observer.mark_valid();

        let _observer_guard = ObserverGuard::enter(Rc::clone(&inner.observer));
        (inner.compute.borrow())();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn effect_runs_initially_when_scope_is_flushed() {
        let calls = Rc::new(Cell::new(0));
        let effect_calls = Rc::clone(&calls);

        let mut app = crate::scope::scope(move || {
            effect({
                let calls = Rc::clone(&effect_calls);
                move || calls.set(calls.get() + 1)
            });
        });

        app.run();
        assert_eq!(calls.get(), 0);

        app.flush();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn effect_reruns_after_dependency_changes() {
        let source_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&source_handle);
        let values = Rc::new(RefCell::new(Vec::new()));
        let effect_values = Rc::clone(&values);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(1);
            *handle.borrow_mut() = Some(source.clone());

            let source_for_effect = source.clone();
            let values_for_effect = Rc::clone(&effect_values);

            effect(move || {
                values_for_effect
                    .borrow_mut()
                    .push(source_for_effect.value());
            });
        });

        app.run();
        app.flush();
        assert_eq!(*values.borrow(), vec![1]);

        source_handle.borrow().as_ref().unwrap().set(2);

        // Effect work is deferred until flush.
        assert_eq!(*values.borrow(), vec![1]);

        app.flush();
        assert_eq!(*values.borrow(), vec![1, 2]);
    }

    #[test]
    fn repeated_invalidations_coalesce_while_effect_is_queued() {
        let source_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&source_handle);
        let values = Rc::new(RefCell::new(Vec::new()));
        let effect_values = Rc::clone(&values);

        let mut app = crate::scope::scope(move || {
            let source = crate::signal::signal(0);
            *handle.borrow_mut() = Some(source.clone());

            let source_for_effect = source.clone();
            let values_for_effect = Rc::clone(&effect_values);

            effect(move || {
                values_for_effect
                    .borrow_mut()
                    .push(source_for_effect.value());
            });
        });

        app.run();
        app.flush();
        assert_eq!(*values.borrow(), vec![0]);

        let source = source_handle.borrow().as_ref().unwrap().clone();
        source.set(1);
        source.set(2);
        source.set(3);

        app.flush();

        // One rerun observes the latest value.
        assert_eq!(*values.borrow(), vec![0, 3]);
    }
}

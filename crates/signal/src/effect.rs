
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use crate::{
    runtime::{self, ObserverGuard, ObserverNode, ScopeEventState},
    scope::{ResourceId, ResourceKind, effect_resource, resolve_resource},
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

    let id: ResourceId = effect_resource(|events| EffectInner {
        compute: RefCell::new(
            compute
                .take()
                .expect("effect computation should be available when creating a resource"),
        ),
        observer: ObserverNode::new(),
        events,
        scheduled: Cell::new(false),
    });

    let inner: Rc<EffectInner> = resolve_resource(id, ResourceKind::Effect);

    // If the resource already existed, refresh its closure to use captures
    // from the latest scope execution.
    if let Some(compute) = compute {
        *inner.compute.borrow_mut() = compute;
    }

    // The observer's scheduler holds only a weak reference. The scope remains
    // the owner of the effect resource.
    let weak_inner = Rc::downgrade(&inner);

    inner.observer.set_schedule(Rc::new(move || {
        let Some(inner) = weak_inner.upgrade() else {
            return;
        };

        schedule_effect(&inner);
    }));

    // Schedule initial execution too. The scheduled flag coalesces this with
    // invalidations that occur before the effect first runs.
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
            *handle.borrow_mut() = Some(source);

            let source_for_effect = source;
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
            *handle.borrow_mut() = Some(source);

            let source_for_effect = source;
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

        let source = *source_handle.borrow().as_ref().unwrap();
        source.set(1);
        source.set(2);
        source.set(3);

        app.flush();

        // One rerun observes the latest value.
        assert_eq!(*values.borrow(), vec![0, 3]);
    }

    #[test]
    fn reused_effect_uses_closure_from_latest_scope_run() {
        let trigger_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&trigger_handle);
        let multiplier = Rc::new(Cell::new(2));
        let current_multiplier = Rc::clone(&multiplier);
        let results = Rc::new(RefCell::new(Vec::new()));
        let effect_results = Rc::clone(&results);

        let mut app = crate::scope::scope(move || {
            let trigger = crate::signal::signal(0);
            *handle.borrow_mut() = Some(trigger);

            // This dependency causes the scope to rerun.
            let _ = trigger.value();

            let factor = current_multiplier.get();
            let results = Rc::clone(&effect_results);

            effect(move || {
                results.borrow_mut().push(factor);
            });
        });

        app.run();
        app.flush();

        assert_eq!(*results.borrow(), vec![2]);

        multiplier.set(3);
        trigger_handle.borrow().as_ref().unwrap().set(1);

        app.run();
        app.flush();

        // The existing effect resource uses the latest captured factor.
        assert_eq!(*results.borrow(), vec![2, 3]);
    }

    #[test]
    fn repeated_scope_runs_do_not_duplicate_initial_effect_work() {
        let trigger_handle = Rc::new(RefCell::new(None));
        let handle = Rc::clone(&trigger_handle);
        let calls = Rc::new(Cell::new(0));
        let effect_calls = Rc::clone(&calls);

        let mut app = crate::scope::scope(move || {
            let trigger = crate::signal::signal(0);
            *handle.borrow_mut() = Some(trigger);

            let _ = trigger.value();

            let calls = Rc::clone(&effect_calls);
            effect(move || {
                calls.set(calls.get() + 1);
            });
        });

        app.run();
        app.run();

        app.flush();
        assert_eq!(calls.get(), 1);

        trigger_handle.borrow().as_ref().unwrap().set(1);

        app.run();
        app.flush();

        assert_eq!(calls.get(), 2);
    }
}

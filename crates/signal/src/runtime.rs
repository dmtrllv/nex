use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

pub(crate) struct SourceNode {
    dependants: RefCell<Vec<Weak<ObserverNode>>>,
}

pub(crate) struct ObserverNode {
    dependencies: RefCell<Vec<Weak<SourceNode>>>,
    invalidated: Cell<bool>,
}

impl SourceNode {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            dependants: RefCell::new(Vec::new()),
        })
    }
}

impl ObserverNode {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            dependencies: RefCell::new(Vec::new()),
            invalidated: Cell::new(false),
        })
    }

    pub(crate) fn is_invalidated(&self) -> bool {
        self.invalidated.get()
    }

    pub(crate) fn mark_valid(&self) {
        self.invalidated.set(false);
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
        if let Some(observer) = dependant.upgrade() {
            observer.invalidated.set(true);
            true
        } else {
            false
        }
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
}

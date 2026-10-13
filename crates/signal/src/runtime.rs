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

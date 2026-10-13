use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
};

use crate::Subscription;

enum Slot {
    Signal {
        type_id: TypeId,
        value: Box<dyn Any>,
    },
}

struct Work {}

#[derive(Default)]
struct ResourceStore {
    resources: Vec<Slot>,
    cursor: usize,
}

#[allow(unused)]
pub struct Scope<T> {
    closure: Box<dyn Fn() -> T>,
    resources: Rc<RefCell<ResourceStore>>,
    result: Option<T>,
    invalidated: bool,
    work_queue: VecDeque<Work>,
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
pub(crate) fn signal_resource<T: 'static>(create: impl FnOnce() -> T) -> Rc<T> {
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

        let value = Rc::new(create());

        store.resources.push(Slot::Signal {
            type_id: TypeId::of::<T>(),
            value: Box::new(Rc::clone(&value)),
        });

        value
    })
}

impl<T> Scope<T> {
    pub fn flush(&mut self) {
        todo!()
    }

    /// Evaluates the scope if it has no cached result or was invalidated.
    ///
    /// Resources are reused by their positions in the scope's execution.
    pub fn run(&mut self) -> &T {
        if self.result.is_none() || self.invalidated {
			// reset the resource cursor
            {
                let mut store = self.resources.borrow_mut();
                store.cursor = 0;
            }

            let result = {
                let _guard = ActiveScopeGuard::enter(Rc::clone(&self.resources));

                (self.closure)()
            };

            {
                let mut store = self.resources.borrow_mut();
                let used_resources = store.cursor;
                store.resources.truncate(used_resources);
            }

            self.result = Some(result);
            self.invalidated = false;
        }

        self.result
            .as_ref()
            .expect("scope result should exist after run")
    }

    pub fn subscribe(&self, _closure: impl Fn(&mut Self, ScopeEvent)) -> Subscription {
        todo!()
    }

    pub fn result(&self) -> Option<&T> {
        self.result.as_ref()
    }
}

pub enum ScopeEvent {
    Invalidated,
    Pending,
}

pub fn scope<T>(closure: impl Fn() -> T + 'static) -> Scope<T> {
    Scope {
        closure: Box::new(closure),
        resources: Rc::new(RefCell::new(ResourceStore::default())),
        result: None,
        invalidated: true,
        work_queue: VecDeque::new(),
    }
}

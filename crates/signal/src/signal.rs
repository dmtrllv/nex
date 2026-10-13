
use std::{
    cell::{Ref, RefCell},
    fmt::Debug,
    ops::Deref,
    rc::Rc,
};

use crate::{scope::signal_resource, Subscription};

/// A handle to a reactive value.
pub struct Signal<T> {
    value: Rc<RefCell<T>>,
}

impl<T> Signal<T> {
    /// Reads the current value.
    ///
    /// Dependency tracking will be added in a later implementation step.
    pub fn get(&self) -> Ref<'_, T> {
        self.value.borrow()
    }

    /// Reads the current value without registering a dependency.
    ///
    /// Dependency tracking will be added in a later implementation step.
    pub fn inspect(&self) -> Ref<'_, T> {
        self.value.borrow()
    }

    /// Replaces the current value.
    pub fn set(&self, value: impl Into<T>) {
        *self.value.borrow_mut() = value.into();
    }

    /// Replaces the current value with the result of `closure`.
    ///
    /// Requires `T: Clone` because the current value is retained in the
    /// signal while a copy is passed to the closure.
    pub fn update(&self, closure: impl Fn(T) -> T) -> Ref<'_, T>
    where
        T: Clone,
    {
        let current = self.value.borrow().clone();
        let next = closure(current);

        *self.value.borrow_mut() = next;

        self.value.borrow()
    }

    /// Subscribes to changes in this signal.
    pub fn subscribe(&self, _closure: impl Fn(&T)) -> Subscription {
        todo!()
    }
}

impl<T: Copy> Signal<T> {
    /// Returns a copy of the current value.
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
    let value = signal_resource::<RefCell<T>>(|| RefCell::new(value));

    Signal { value }
}

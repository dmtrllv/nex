use std::{
    cell::{Ref, RefCell},
    fmt::Debug,
    ops::Deref,
    rc::Rc,
};

use crate::{
    Subscription,
    runtime::{self, ScopeEventState, SourceNode},
    scope::signal_resource,
};

struct SignalInner<T> {
    value: RefCell<Option<T>>,
    source: Rc<SourceNode>,
    _events: Rc<ScopeEventState>,
}

/// A handle to a reactive value.
pub struct Signal<T> {
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

        self.inspect()
    }

    /// Subscribes to changes in this signal.
    pub fn subscribe(&self, _closure: impl Fn(&T)) -> Subscription {
        todo!()
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
        _events: events,
    });

    Signal { inner }
}

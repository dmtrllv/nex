use std::marker::PhantomData;

use crate::Subscription;

/// A handle to a reactive value.
///
/// Reads through [`get`](Self::get) register the active reactive context as a
/// dependency. Writes notify dependents according to the runtime's scheduling
/// and change-detection rules.
pub struct Signal<T> {
    _data: PhantomData<T>,
}

impl<T> Signal<T> {
    /// Reads the current value and registers the active reactive context as a
    /// dependency, if one exists.
    ///
    /// The returned reference is tied to the lifetime of this handle's borrow.
    pub fn get(&self) -> &T {
        todo!()
    }

    /// Reads the current value without registering a reactive dependency.
    ///
    /// Use this when reading state should not cause the current reactive
    /// context to rerun when the value changes.
    pub fn inspect(&self) -> &T {
        todo!()
    }

    /// Replaces the current value.
    ///
    /// The supplied value is converted into `T`. Dependents are invalidated
    /// according to the signal's change-detection policy; queued reactive
    /// work is processed by the runtime's scheduler.
    pub fn set(&self, _value: impl Into<T>) {
        todo!()
    }

    /// Updates the current value using a closure and returns a reference to
    /// the resulting value.
    ///
    /// Unlike [`set`](Self::set), this operation can derive the new value from
    /// the current one. Dependents are invalidated according to the signal's
    /// change-detection policy.
    pub fn update(&self, _closure: impl Fn(T) -> T) -> &T {
        todo!()
    }

    /// Subscribes to changes in this signal.
    ///
    /// The callback receives a reference to the value when a change
    /// notification is delivered. Returns a [`Subscription`] that can be
    /// consumed to unsubscribe.
    pub fn subscribe(&self, _closure: impl Fn(&T)) -> Subscription {
        todo!()
    }
}

impl<T: Copy> Signal<T> {
    /// Returns a copy of the current value and tracks the active reactive
    /// context as a dependency, if one exists.
    ///
    /// This is equivalent to calling [`get`](Self::get) and copying the
    /// result. Use [`inspect_value`](Self::inspect_value) to read a copy
    /// without registering a dependency.
    pub fn value(&self) -> T {
        *self.get()
    }

    /// Returns a copy of the current value without registering a reactive
    /// dependency.
    ///
    /// This is equivalent to calling [`inspect`](Self::inspect) and copying
    /// the result.
    pub fn inspect_value(&self) -> T {
        *self.inspect()
    }
}

/// Creates a reactive signal initialized with `value`.
///
/// The signal stores mutable state that can be read through tracked or
/// untracked reads and updated through its handle.
pub fn signal<T>(_value: T) -> Signal<T> {
    todo!()
}
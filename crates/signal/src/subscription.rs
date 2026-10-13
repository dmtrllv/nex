use std::{cell::Cell, rc::Rc};

/// A handle to an active subscription.
///
/// Dropping this value does not unsubscribe. Call
/// [`unsubscribe`](Self::unsubscribe) to deactivate the subscription.
pub struct Subscription {
    active: Rc<Cell<bool>>,
}

impl Subscription {
    pub(crate) fn create() -> (Self, Rc<Cell<bool>>) {
        let active = Rc::new(Cell::new(true));

        (
            Self {
                active: Rc::clone(&active),
            },
            active,
        )
    }

    /// Removes the subscription so it no longer receives notifications.
    pub fn unsubscribe(self) {
        self.active.set(false);
    }
}

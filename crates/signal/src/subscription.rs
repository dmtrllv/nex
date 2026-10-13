/// A handle to an active signal subscription.
///
/// Dropping this value does not necessarily unsubscribe automatically;
/// call [`unsubscribe`](Self::unsubscribe) to explicitly remove the
/// subscription.
pub struct Subscription;

impl Subscription {
    /// Removes the subscription so its callback no longer receives change
    /// notifications.
    ///
    /// Consumes the handle to prevent the same subscription from being
    /// explicitly unsubscribed through it more than once.
    pub fn unsubscribe(self) {
        todo!()
    }
}

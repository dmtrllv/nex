
use crate::runtime;

struct BatchGuard;

impl Drop for BatchGuard {
    fn drop(&mut self) {
        runtime::end_batch();
    }
}

/// Defers eligible reactive work until the outermost batch completes.
///
/// Signal and memo values are updated immediately. This function does not
/// automatically flush queued work when the batch ends.
pub fn batch<T>(closure: impl FnOnce() -> T) -> T {
    runtime::begin_batch();

    let _guard = BatchGuard;

    closure()
}

#[cfg(test)]
mod tests {
    use signal::{signal, scope};

    #[test]
    fn signal_stores_and_updates_its_value() {
        let mut app = scope(|| signal(10));

        let count = app.run();

        assert_eq!(count.value(), 10);

        count.set(20);
        assert_eq!(count.value(), 20);

        let updated = count.update(|value| value + 5);
        assert_eq!(*updated, 25);
        assert_eq!(count.value(), 25);
    }

    #[test]
    fn signal_resource_persists_across_scope_runs() {
        let mut app = scope(|| signal(10));

        app.run().set(20);

        // The scope remains valid, so run() returns its cached result.
        assert_eq!(app.run().value(), 20);
    }

    #[test]
    #[should_panic(expected = "signal() called outside an active scope")]
    fn signal_requires_an_active_scope() {
        let _count = signal(10);
    }
}
use signal::{scope, signal};

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
#[should_panic(expected = "signals must be created inside an active scope")]
fn signal_requires_an_active_scope() {
    let _count = signal(10);
}

#[test]
fn signal_stores_and_reads_its_value() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    assert_eq!(count.value(), 10);
    assert_eq!(*count.get(), 10);
    assert_eq!(*count.inspect(), 10);
    assert_eq!(count.inspect_value(), 10);
}

#[test]
fn set_replaces_the_value() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    count.set(20);

    assert_eq!(count.value(), 20);
}

#[test]
fn update_replaces_the_value_and_returns_a_reference() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    let updated = count.update(|value| value + 5);

    assert_eq!(*updated, 15);
    assert_eq!(count.value(), 15);
}

#[test]
fn update_works_without_clone() {
    struct NonClone {
        value: String,
    }

    let mut app = scope(|| {
        signal(NonClone {
            value: String::from("hello"),
        })
    });

    let state = app.run();

    let updated = state.update(|mut current| {
        current.value.push_str(" world");
        current
    });

    assert_eq!(updated.value, "hello world");
    assert_eq!(state.inspect().value, "hello world");
}

#[test]
fn update_can_consume_the_current_value() {
    struct NonClone(String);

    let mut app = scope(|| signal(NonClone(String::from("before"))));
    let state = app.run();

    state.update(|current| {
        let NonClone(value) = current;
        NonClone(format!("{value}-after"))
    });

    assert_eq!(state.inspect().0, "before-after");
}

#[test]
fn tracked_signal_write_invalidates_scope() {
    let mut app = scope(|| {
        let count = signal(1);
        let observed = count.value();

        (count, observed)
    });

    let (count, observed) = app.run();
    assert_eq!(*observed, 1);

    count.set(2);

    let (_, observed) = app.run();
    assert_eq!(*observed, 2);
}

#[test]
fn update_invalidates_scope() {
    let mut app = scope(|| {
        let count = signal(1);
        let observed = count.value();

        (count, observed)
    });

    let (count, observed) = app.run();
    assert_eq!(*observed, 1);

    count.update(|value| value + 1);

    let (_, observed) = app.run();
    assert_eq!(*observed, 2);
}

#[test]
fn untracked_read_does_not_invalidate_scope() {
    use std::cell::Cell;
    use std::rc::Rc;

    let executions = Rc::new(Cell::new(0));
    let executions_for_scope = Rc::clone(&executions);

    let mut app = scope(move || {
        executions_for_scope.set(executions_for_scope.get() + 1);

        let count = signal(1);
        let observed = count.inspect_value();

        (count, observed)
    });

    let (count, observed) = app.run();
    assert_eq!(*observed, 1);
    assert_eq!(executions.get(), 1);

    count.set(2);

    let (_, observed) = app.run();

    // The untracked read created no dependency, so the cached result remains.
    assert_eq!(*observed, 1);
    assert_eq!(executions.get(), 1);
}

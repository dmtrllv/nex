use signal::{scope, signal};

#[test]
fn signal_stores_and_updates_its_value() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    assert_eq!(*count.get(), 10);

    count.set(20);
    assert_eq!(*count.get(), 20);

    let updated = count.update(|value| *value += 5);
    assert_eq!(*updated, 25);
    assert_eq!(*count.get(), 25);
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

    assert_eq!(*count.get(), 10);
    assert_eq!(*count.inspect(), 10);
}

#[test]
fn set_replaces_the_value() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    count.set(20);

    assert_eq!(*count.get(), 20);
}

#[test]
fn update_mutates_the_value_and_returns_a_read_guard() {
    let mut app = scope(|| signal(10));
    let count = app.run();

    let updated = count.update(|value| *value += 5);

    assert_eq!(*updated, 15);
    assert_eq!(*count.inspect(), 15);
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

    let updated = state.update(|current| {
        current.value.push_str(" world");
    });

    assert_eq!(updated.value, "hello world");
    assert_eq!(state.inspect().value, "hello world");
}

#[test]
fn update_can_mutate_a_non_clone_value_in_place() {
    struct NonClone(String);

    let mut app = scope(|| signal(NonClone(String::from("before"))));
    let state = app.run();

    state.update(|current| {
        current.0.push_str("-after");
    });

    assert_eq!(state.inspect().0, "before-after");
}

#[test]
fn tracked_signal_read_invalidates_scope_after_write() {
    let mut app = scope(|| {
        let count = signal(1);
        let observed = *count.get();

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
        let observed = *count.get();

        (count, observed)
    });

    let (count, observed) = app.run();
    assert_eq!(*observed, 1);

    count.update(|value| *value += 1);

    let (_, observed) = app.run();
    assert_eq!(*observed, 2);
}

#[test]
fn untracked_read_does_not_create_a_dependency() {
    use std::cell::Cell;
    use std::rc::Rc;

    let executions = Rc::new(Cell::new(0));
    let executions_for_scope = Rc::clone(&executions);

    let mut app = scope(move || {
        executions_for_scope.set(executions_for_scope.get() + 1);

        let count = signal(1);
        let observed = *count.inspect();

        (count, observed)
    });

    let (count, observed) = app.run();
    assert_eq!(*observed, 1);
    assert_eq!(executions.get(), 1);

    count.set(2);

    // Explicitly rerunning the scope recomputes its result.
    let (_, observed) = app.run();

    assert_eq!(*observed, 2);
    assert_eq!(executions.get(), 2);
}
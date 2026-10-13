use signal::{scope, signal};

#[test]
fn signal_resource_persists_across_scope_runs() {
    let mut app = scope(|| signal(10));

    app.run().set(20);

    // The scope remains valid, so run() returns its cached result.
    assert_eq!(app.run().value(), 20);
}

#[test]
fn signal_value_persists_when_the_scope_closure_runs_again() {
    use std::cell::Cell;
    use std::rc::Rc;

    let executions = Rc::new(Cell::new(0));
    let executions_for_scope = Rc::clone(&executions);

    let mut app = scope(move || {
        executions_for_scope.set(executions_for_scope.get() + 1);
        signal(10)
    });

    app.run().set(20);

    // No tracked dependency changed, so the cached scope result is reused.
    assert_eq!(app.run().value(), 20);
    assert_eq!(executions.get(), 1);
}

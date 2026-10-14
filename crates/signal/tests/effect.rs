use signal::{effect, scope, signal};
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn effect_runs_initially_only_after_flush() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let calls_for_scope = Rc::clone(&calls);

    let mut app = scope(move || {
        let calls_for_effect = Rc::clone(&calls_for_scope);

        effect(move || {
            calls_for_effect.borrow_mut().push("run");
        });
    });

    app.run();

    // Creating the effect schedules its initial execution.
    assert!(calls.borrow().is_empty());

    app.flush();

    assert_eq!(*calls.borrow(), vec!["run"]);
}

#[test]
fn effect_reruns_when_a_tracked_signal_changes() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let values_for_scope = Rc::clone(&values);

    let mut app = scope(move || {
        let count = signal(1);

        let count_for_effect = count.clone();
        let values_for_effect = Rc::clone(&values_for_scope);

        effect(move || {
            values_for_effect
                .borrow_mut()
                .push(count_for_effect.value());
        });

        count
    });

    let count = app.run().clone();

    app.flush();
    assert_eq!(*values.borrow(), vec![1]);

    count.set(2);

    // Setting the signal must not execute the effect synchronously.
    assert_eq!(*values.borrow(), vec![1]);

    app.flush();

    assert_eq!(*values.borrow(), vec![1, 2]);
}

#[test]
fn repeated_invalidations_coalesce_into_one_rerun() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let values_for_scope = Rc::clone(&values);

    let mut app = scope(move || {
        let count = signal(0);

        let count_for_effect = count.clone();
        let values_for_effect = Rc::clone(&values_for_scope);

        effect(move || {
            values_for_effect
                .borrow_mut()
                .push(count_for_effect.value());
        });

        count
    });

    let count = app.run().clone();

    app.flush();
    assert_eq!(*values.borrow(), vec![0]);

    count.set(1);
    count.set(2);
    count.set(3);

    // The queued effect has not run yet.
    assert_eq!(*values.borrow(), vec![0]);

    app.flush();

    // The effect runs once and observes the latest value.
    assert_eq!(*values.borrow(), vec![0, 3]);
}

#[test]
fn effect_does_not_rerun_when_an_untracked_signal_changes() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let values_for_scope = Rc::clone(&values);

    let mut app = scope(move || {
        let tracked = signal(10);
        let unrelated = signal(100);

        let tracked_for_effect = tracked.clone();
        let values_for_effect = Rc::clone(&values_for_scope);

        effect(move || {
            values_for_effect
                .borrow_mut()
                .push(tracked_for_effect.value());
        });

        (tracked, unrelated)
    });

    let (tracked, unrelated) = app.run().clone();

    app.flush();
    assert_eq!(*values.borrow(), vec![10]);

    unrelated.set(200);
    app.flush();

    assert_eq!(
        *values.borrow(),
        vec![10],
        "changing an unread signal must not rerun the effect"
    );

    tracked.set(20);
    app.flush();

    assert_eq!(*values.borrow(), vec![10, 20]);
}

#[test]
fn effect_updates_its_dependencies_after_each_execution() {
    let values = Rc::new(RefCell::new(Vec::new()));
    let values_for_scope = Rc::clone(&values);

    let mut app = scope(move || {
        let use_first = signal(true);
        let first = signal(10);
        let second = signal(100);

        let use_first_for_effect = use_first.clone();
        let first_for_effect = first.clone();
        let second_for_effect = second.clone();
        let values_for_effect = Rc::clone(&values_for_scope);

        effect(move || {
            let value = if use_first_for_effect.value() {
                first_for_effect.value()
            } else {
                second_for_effect.value()
            };

            values_for_effect.borrow_mut().push(value);
        });

        (use_first, first, second)
    });

    let (use_first, first, second) = app.run().clone();

    app.flush();
    assert_eq!(*values.borrow(), vec![10]);

    // Switch the effect to reading the second signal.
    use_first.set(false);
    app.flush();

    assert_eq!(*values.borrow(), vec![10, 100]);

    // The effect should no longer depend on the first signal.
    first.set(20);
    app.flush();

    assert_eq!(
        *values.borrow(),
        vec![10, 100],
        "the previous dependency should be removed after reevaluation"
    );

    // The new dependency should continue to trigger the effect.
    second.set(200);
    app.flush();

    assert_eq!(*values.borrow(), vec![10, 100, 200]);
}

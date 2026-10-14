use signal::{batch, effect, scope, signal};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

#[test]
fn batch_defers_effect_execution_until_explicit_flush() {
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

    // Process the effect's initial execution.
    app.flush();
    assert_eq!(*values.borrow(), vec![0]);

    batch(|| {
        count.set(1);
        count.set(2);

        // Flushing inside a batch must not execute queued work.
        app.flush();

        assert_eq!(*values.borrow(), vec![0]);
    });

    // Ending the batch must not automatically execute queued work.
    assert_eq!(*values.borrow(), vec![0]);

    app.flush();

    // The effect runs once and observes the latest value.
    assert_eq!(*values.borrow(), vec![0, 2]);
}

#[test]
fn nested_batch_defers_work_until_outermost_batch_ends() {
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

    batch(|| {
        count.set(1);

        batch(|| {
            count.set(2);
            app.flush();

            // The inner batch must not release the deferred work.
            assert_eq!(*values.borrow(), vec![0]);
        });

        // The outer batch is still active.
        app.flush();
        assert_eq!(*values.borrow(), vec![0]);
    });

    // Work remains queued after the outermost batch ends.
    assert_eq!(*values.borrow(), vec![0]);

    app.flush();
    assert_eq!(*values.borrow(), vec![0, 2]);
}

#[test]
fn batch_with_no_changes_does_not_rerun_effect() {
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

    let _count = app.run().clone();

    app.flush();
    assert_eq!(*values.borrow(), vec![0]);

    batch(|| {});

    app.flush();

    assert_eq!(*values.borrow(), vec![0]);
}

#[test]
fn batch_restores_nesting_depth_when_closure_panics() {
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

    let result = catch_unwind(AssertUnwindSafe(|| {
        batch(|| {
            count.set(1);
            panic!("test panic");
        });
    }));

    assert!(result.is_err());

    // The batch guard must restore the batch depth during unwinding.
    // Work must still be processable after the panic is caught.
    app.flush();

    assert_eq!(*values.borrow(), vec![0, 1]);
}

#[test]
fn multiple_updates_in_separate_batches_produce_separate_runs() {
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

    batch(|| count.set(1));
    app.flush();

    assert_eq!(*values.borrow(), vec![0, 1]);

    batch(|| count.set(2));
    app.flush();

    assert_eq!(*values.borrow(), vec![0, 1, 2]);
}

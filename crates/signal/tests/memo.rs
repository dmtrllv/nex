use signal::{batch, effect, memo, scope, signal};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[test]
fn memo_is_evaluated_lazily_and_caches_its_value() {
    let computations = Rc::new(Cell::new(0));
    let computation_count = Rc::clone(&computations);

    let mut app = scope(move || {
        let count = signal(10);
        let input = count.clone();
        let computations = Rc::clone(&computation_count);

        let doubled = memo(move || {
            computations.set(computations.get() + 1);
            input.value() * 2
        });

        (count, doubled)
    });

    let (count, doubled) = app.run().clone();

    assert_eq!(computations.get(), 0);

    assert_eq!(doubled.value(), 20);
    assert_eq!(computations.get(), 1);

    assert_eq!(doubled.value(), 20);
    assert_eq!(doubled.inspect_value(), 20);
    assert_eq!(computations.get(), 1);

    assert_eq!(count.value(), 10);
}

#[test]
fn memo_recomputes_after_a_dependency_changes() {
    let computations = Rc::new(Cell::new(0));
    let computation_count = Rc::clone(&computations);

    let mut app = scope(move || {
        let count = signal(2);
        let input = count.clone();
        let computations = Rc::clone(&computation_count);

        let squared = memo(move || {
            computations.set(computations.get() + 1);
            input.value() * input.value()
        });

        (count, squared)
    });

    let (count, squared) = app.run().clone();

    assert_eq!(squared.value(), 4);
    assert_eq!(computations.get(), 1);

    count.set(3);

    // Updating a dependency invalidates the memo, but does not
    // synchronously execute its computation.
    assert_eq!(computations.get(), 1);

    assert_eq!(squared.value(), 9);
    assert_eq!(computations.get(), 2);

    // Flushing must not recompute a memo that is already up to date.
    app.flush();

    assert_eq!(squared.value(), 9);
    assert_eq!(computations.get(), 2);
}

#[test]
fn memo_tracks_dependencies_read_during_computation() {
    let computations = Rc::new(Cell::new(0));
    let computation_count = Rc::clone(&computations);

    let mut app = scope(move || {
        let left = signal(10);
        let right = signal(20);

        let left_input = left.clone();
        let right_input = right.clone();
        let computations = Rc::clone(&computation_count);

        let total = memo(move || {
            computations.set(computations.get() + 1);
            left_input.value() + right_input.value()
        });

        (left, right, total)
    });

    let (left, right, total) = app.run().clone();

    assert_eq!(total.value(), 30);
    assert_eq!(computations.get(), 1);

    left.set(15);
    app.flush();

    assert_eq!(total.value(), 35);
    assert_eq!(computations.get(), 2);

    right.set(25);
    app.flush();

    assert_eq!(total.value(), 40);
    assert_eq!(computations.get(), 3);
}

#[test]
fn memo_does_not_recompute_when_an_untracked_signal_changes() {
    let computations = Rc::new(Cell::new(0));
    let computation_count = Rc::clone(&computations);

    let mut app = scope(move || {
        let tracked = signal(5);
        let unrelated = signal(100);

        let input = tracked.clone();
        let computations = Rc::clone(&computation_count);

        let doubled = memo(move || {
            computations.set(computations.get() + 1);
            input.value() * 2
        });

        (tracked, unrelated, doubled)
    });

    let (tracked, unrelated, doubled) = app.run().clone();

    assert_eq!(doubled.value(), 10);
    assert_eq!(computations.get(), 1);

    unrelated.set(200);
    app.flush();

    assert_eq!(doubled.value(), 10);
    assert_eq!(computations.get(), 1);

    tracked.set(6);
    app.flush();

    assert_eq!(doubled.value(), 12);
    assert_eq!(computations.get(), 2);
}

#[test]
fn memo_updates_its_dependencies_when_the_computation_branch_changes() {
    let computations = Rc::new(Cell::new(0));
    let computation_count = Rc::clone(&computations);

    let mut app = scope(move || {
        let use_left = signal(true);
        let left = signal(10);
        let right = signal(20);

        let condition = use_left.clone();
        let left_input = left.clone();
        let right_input = right.clone();
        let computations = Rc::clone(&computation_count);

        let selected = memo(move || {
            computations.set(computations.get() + 1);

            if condition.value() {
                left_input.value()
            } else {
                right_input.value()
            }
        });

        (use_left, left, right, selected)
    });

    let (use_left, left, right, selected) = app.run().clone();

    assert_eq!(selected.value(), 10);
    assert_eq!(computations.get(), 1);

    use_left.set(false);
    app.flush();

    assert_eq!(selected.value(), 20);
    assert_eq!(computations.get(), 2);

    // The memo no longer depends on `left`.
    left.set(11);
    app.flush();

    assert_eq!(selected.value(), 20);
    assert_eq!(computations.get(), 2);

    // It now depends on `right`.
    right.set(21);
    app.flush();

    assert_eq!(selected.value(), 21);
    assert_eq!(computations.get(), 3);
}

#[test]
fn memo_does_not_invalidate_dependents_when_its_value_is_unchanged() {
    let effect_runs = Rc::new(Cell::new(0));
    let effect_run_count = Rc::clone(&effect_runs);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let parity = memo(move || input.value() % 2);

        let observed = parity.clone();
        let effect_runs = Rc::clone(&effect_run_count);

        effect(move || {
            let _ = observed.value();
            effect_runs.set(effect_runs.get() + 1);
        });

        (source, parity)
    });

    let (source, parity) = app.run().clone();

    assert_eq!(parity.value(), 1);

    app.flush();
    assert_eq!(effect_runs.get(), 1);

    // The input changes, but its parity remains 1.
    source.set(3);
    app.flush();

    assert_eq!(parity.value(), 1);
    assert_eq!(
        effect_runs.get(),
        1,
        "a dependent effect should not rerun when the memo value is unchanged"
    );

    source.set(4);
    app.flush();

    assert_eq!(parity.value(), 0);
    assert_eq!(effect_runs.get(), 2);
}

#[test]
fn reading_memo_tracks_its_output_in_an_effect() {
    let observed_values = Rc::new(RefCell::new(Vec::new()));
    let values_from_effect = Rc::clone(&observed_values);

    let mut app = scope(move || {
        let source = signal(2);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);

        let observed = doubled.clone();
        let values = Rc::clone(&values_from_effect);

        effect(move || {
            values.borrow_mut().push(observed.value());
        });

        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    app.flush();
    assert_eq!(*observed_values.borrow(), vec![4]);

    source.set(3);

    // The effect is deferred until the scope processes its work.
    assert_eq!(*observed_values.borrow(), vec![4]);

    app.flush();

    assert_eq!(doubled.value(), 6);
    assert_eq!(*observed_values.borrow(), vec![4, 6]);
}

#[test]
fn inspect_does_not_track_a_memo_dependency() {
    let effect_runs = Rc::new(Cell::new(0));
    let effect_run_count = Rc::clone(&effect_runs);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);

        // Initialize the memo before the effect uses its untracked read.
        let _ = doubled.value();

        let observed = doubled.clone();
        let effect_runs = Rc::clone(&effect_run_count);

        effect(move || {
            let _ = observed.inspect_value();
            effect_runs.set(effect_runs.get() + 1);
        });

        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    app.flush();
    assert_eq!(effect_runs.get(), 1);

    source.set(2);
    app.flush();

    assert_eq!(doubled.value(), 4);
    assert_eq!(
        effect_runs.get(),
        1,
        "inspect_value should not establish a dependency for the effect"
    );
}

#[test]
fn nested_memos_recompute_from_inner_to_outer() {
    let inner_computations = Rc::new(Cell::new(0));
    let inner_count = Rc::clone(&inner_computations);

    let outer_computations = Rc::new(Cell::new(0));
    let outer_count = Rc::clone(&outer_computations);

    let mut app = scope(move || {
        let source = signal(2);
        let input = source.clone();
        let inner_count = Rc::clone(&inner_count);

        let inner = memo(move || {
            inner_count.set(inner_count.get() + 1);
            input.value() * 3
        });

        let inner_input = inner.clone();
        let outer_count = Rc::clone(&outer_count);

        let outer = memo(move || {
            outer_count.set(outer_count.get() + 1);
            inner_input.value() + 1
        });

        (source, inner, outer)
    });

    let (source, inner, outer) = app.run().clone();

    assert_eq!(outer.value(), 7);
    assert_eq!(inner_computations.get(), 1);
    assert_eq!(outer_computations.get(), 1);

    source.set(4);
    app.flush();

    assert_eq!(outer.value(), 13);
    assert_eq!(inner_computations.get(), 2);
    assert_eq!(outer_computations.get(), 2);

    // Reading a current cached value must not recompute either memo.
    assert_eq!(inner.value(), 12);
    assert_eq!(outer.value(), 13);
    assert_eq!(inner_computations.get(), 2);
    assert_eq!(outer_computations.get(), 2);
}

#[test]
fn memo_subscription_does_not_notify_initial_value() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let received_by_callback = Rc::clone(&received);

    let mut app = scope(move || {
        let source = signal(5);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);
        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    // Initialize the memo before subscribing.
    assert_eq!(doubled.value(), 10);

    let received_callback = Rc::clone(&received_by_callback);
    let _subscription = doubled.subscribe(move |value| {
        received_callback.borrow_mut().push(*value);
    });

    app.flush();

    assert!(
        received.borrow().is_empty(),
        "subscribing should not notify the initial value"
    );

    source.set(6);

    assert!(received.borrow().is_empty());

    app.flush();

    assert_eq!(*received.borrow(), vec![12]);
}

#[test]
fn memo_subscription_notifies_only_when_the_value_changes() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let received_by_callback = Rc::clone(&received);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let parity = memo(move || input.value() % 2);
        (source, parity)
    });

    let (source, parity) = app.run().clone();

    assert_eq!(parity.value(), 1);

    let received_callback = Rc::clone(&received_by_callback);
    let _subscription = parity.subscribe(move |value| {
        received_callback.borrow_mut().push(*value);
    });

    source.set(3);
    app.flush();

    assert!(received.borrow().is_empty());

    source.set(4);
    app.flush();

    assert_eq!(*received.borrow(), vec![0]);
}

#[test]
fn memo_subscription_coalesces_repeated_invalidations() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let received_by_callback = Rc::clone(&received);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);
        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    assert_eq!(doubled.value(), 2);

    let received_callback = Rc::clone(&received_by_callback);
    let _subscription = doubled.subscribe(move |value| {
        received_callback.borrow_mut().push(*value);
    });

    source.set(2);
    source.set(3);
    source.set(4);

    assert!(received.borrow().is_empty());

    app.flush();

    assert_eq!(*received.borrow(), vec![8]);
}

#[test]
fn memo_subscription_can_be_unsubscribed() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let received_by_callback = Rc::clone(&received);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);
        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    assert_eq!(doubled.value(), 2);

    let received_callback = Rc::clone(&received_by_callback);
    let subscription = doubled.subscribe(move |value| {
        received_callback.borrow_mut().push(*value);
    });

    source.set(2);
    app.flush();

    assert_eq!(*received.borrow(), vec![4]);

    subscription.unsubscribe();

    source.set(3);
    app.flush();

    assert_eq!(
        *received.borrow(),
        vec![4],
        "an unsubscribed memo subscription should not receive further notifications"
    );
}

#[test]
fn batch_coalesces_memo_subscription_notifications() {
    let received = Rc::new(RefCell::new(Vec::new()));
    let received_by_callback = Rc::clone(&received);

    let mut app = scope(move || {
        let source = signal(1);
        let input = source.clone();

        let doubled = memo(move || input.value() * 2);
        (source, doubled)
    });

    let (source, doubled) = app.run().clone();

    assert_eq!(doubled.value(), 2);

    let received_callback = Rc::clone(&received_by_callback);
    let _subscription = doubled.subscribe(move |value| {
        received_callback.borrow_mut().push(*value);
    });

    batch(|| {
        source.set(2);
        source.set(3);
        source.set(4);

        assert!(received.borrow().is_empty());
    });

    // Ending a batch makes work eligible; it does not flush the scope.
    assert!(received.borrow().is_empty());

    app.flush();

    assert_eq!(doubled.value(), 8);
    assert_eq!(*received.borrow(), vec![8]);
}

#[test]
fn reused_memo_uses_computation_from_latest_scope_run() {
    let trigger_handle = Rc::new(RefCell::new(None));
    let handle = Rc::clone(&trigger_handle);

    let multiplier = Rc::new(Cell::new(2));
    let current_multiplier = Rc::clone(&multiplier);

    let mut app = scope(move || {
        let trigger = signal(0);
        let source = signal(10);

        *handle.borrow_mut() = Some(trigger.clone());

        // The scope depends on this value, so changing it causes a rerun.
        let _ = trigger.value();

        let factor = current_multiplier.get();
        let input = source.clone();

        let scaled = memo(move || input.value() * factor);

        (trigger, source, scaled)
    });

    let (_, _, scaled) = app.run().clone();

    assert_eq!(scaled.value(), 20);

    multiplier.set(3);
    trigger_handle.borrow().as_ref().unwrap().set(1);

    let (_, _, reused) = app.run().clone();

    assert_eq!(
        reused.value(),
        30,
        "the memo should use the computation from the latest scope run"
    );
}

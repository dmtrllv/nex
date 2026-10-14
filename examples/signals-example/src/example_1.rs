use signal::{ScopeEvent, effect, memo, scope, signal};

#[allow(unused)]
pub fn example_1() {
    let mut app = scope(|| {
        let price = signal(25.0);
        let quantity = signal(2);

        let total = memo(move || price.value() * quantity.value() as f64);

        effect(move || {
            println!("Price: €{:.2}", price.value());
        });

        effect(move || {
            println!("Cart total: €{:.2}", total.value());
        });

        println!("Inspecting price: €{:.2}", price.inspect());

        (price, quantity)
    });

    let _subscription = app.subscribe(|app, event| match event {
        ScopeEvent::Invalidated => {
            println!("Got invalidated scope");
            let (price, quantity) = app.run();

            println!("--- Read current state ---");
            println!("Price: €{:.2}", price.get());
            println!("Quantity: {}", quantity.get());
        }
        ScopeEvent::Pending => {
            println!("Got pending tasks");
            app.flush();
        }
    });

    let (price, quantity) = app.run();
    println!(
        "Initial price: {}, quantity: {}",
        price.value(),
        quantity.value()
    );

    quantity.set(3);
    price.set(30.0);

    let result = app.run();
    println!("Latest result: {result:?}");
}

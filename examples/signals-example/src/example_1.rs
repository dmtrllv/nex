use signal::{ScopeEvent, effect, memo, scope, signal};

#[allow(unused)]
pub fn example_1() {
    let mut app = scope(|| {
        let price = signal(25.0);
        let quantity = signal(2);

        let p = price.clone();
        let q = quantity.clone();

        let total = memo(move || p.value() * q.value() as f64);

        let p = price.clone();
        effect(move || {
            println!("Price: €{:.2}", p.value());
        });

        effect(move || {
            println!("Cart total: €{:.2}", total.value());
        });

        let p = price.clone();
        println!("Inspecting price: €{:.2}", p.inspect());

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

use signal::{effect, memo, scope, signal};

fn main() {
    let mut app = scope(|| {
        let price = signal(25.0);
        let quantity = signal(2);

        let total = memo(|| price.value() * quantity.value() as f64);

        effect(|| {
            println!("Price: €{:.2}", price.value());
        });

        effect(|| {
            println!("Cart total: €{:.2}", total);
        });

        println!("Inspecing Price: €{:.2}", price.inspect());

        (price, quantity)
    });

    // Initial execution creates the resources and returns their handles.
    let (price, quantity) = app.run();

    println!("--- Initial cart ---");
    app.flush();

    println!("--- Change quantity ---");
    quantity.set(3);
    app.flush();

    println!("--- Change price ---");
    price.set(30.0);
    app.flush();

    // Running the scope again reuses its resources.
    let (price, quantity) = app.run();

    println!("--- Read current state ---");
    // If not in a reactive context just return the value like .inspect()
    println!("Price: €{:.2}", price.get());
    println!("Quantity: {}", quantity.get());
}

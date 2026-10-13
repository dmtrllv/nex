//use signal::{scope, signal};

use crate::example_1::example_1;

mod example_1;

fn main() {
	example_1();
    //let mut app = scope(|| signal(0));

    //let count = app.run();

    //assert_eq!(count.inspect_value(), 0);

    //count.set(10);

    //assert_eq!(count.inspect_value(), 10);
}

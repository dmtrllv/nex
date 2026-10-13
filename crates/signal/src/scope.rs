#[allow(unused)]
pub struct Scope<T> {
    closure: Box<dyn Fn() -> T>,
}

impl<T> Scope<T> {
    pub fn flush(&mut self) {
        // should it take a mut self?
		todo!()
    }

    pub fn run(&mut self) -> T {
        // should it take a mut self?
		todo!()
    }
}

pub fn scope<T>(_closure: impl Fn() -> T) -> Scope<T> {
    todo!()
}

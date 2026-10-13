use crate::{Subscription, subscription};

#[allow(unused)]
pub struct Scope<T> {
    closure: Box<dyn Fn() -> T>,
}

impl<T> Scope<T> {
    pub fn flush(&mut self) {
		todo!()
    }

	// should only run when its state is invalidated
	// so multiple calls will just return its cached value
    pub fn run(&mut self) -> &T {
		todo!()
    }

	pub fn subscribe(&self, _closure: impl Fn(&mut Self, ScopeEvent)) -> Subscription {
		todo!()
	}

	pub fn result(&self) -> Option<&T> {
        todo!()
    }
}

pub enum ScopeEvent {
	Invalidated,
	Pending
}

pub fn scope<T>(_closure: impl Fn() -> T) -> Scope<T> {
    todo!()
}

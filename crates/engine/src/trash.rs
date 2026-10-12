//! What a module's job leaves to free, freed once the modules waiting on
//! it have started: on the same thread, as a free from another thread
//! costs the allocator far more, but off the path of what comes next.

use std::any::Any;
use std::cell::RefCell;

thread_local! {
    static TRASH: RefCell<Vec<Box<dyn Any>>> = RefCell::default();
}

/// Frees `value` at the next [`empty`] on this thread.
pub fn defer(value: impl Any) {
    TRASH.with_borrow_mut(|trash| trash.push(Box::new(value)));
}

/// Frees what this thread deferred.
pub fn empty() {
    let trash = TRASH.with_borrow_mut(std::mem::take);
    drop(trash);
}

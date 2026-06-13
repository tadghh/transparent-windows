//! UI-thread window lifetime management.
//!
//! With a single persistent Slint event loop (see `main`), windows are opened
//! with `show()` instead of the blocking `run()`. A shown window stays open only
//! while a strong [`slint::ComponentHandle`] to it is alive, so we stash those
//! handles in a thread-local owned by the UI (event-loop) thread.
//!
//! All access happens on the event-loop thread — either from a window callback
//! or from inside a `slint::invoke_from_event_loop` closure.

use std::{any::Any, cell::RefCell, collections::HashMap};

thread_local! {
    static KEEPALIVE: RefCell<HashMap<&'static str, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Keep a window handle alive under `key`. Reopening the same `key` drops the
/// previous window of that kind, closing it — so at most one of each kind is
/// retained. Windows are closed by hiding them; the stale handle is released on
/// the next open (never mid-callback, which would drop a live component).
pub fn keep_alive(key: &'static str, handle: impl Any + 'static) {
    KEEPALIVE.with(|map| {
        map.borrow_mut().insert(key, Box::new(handle));
    });
}

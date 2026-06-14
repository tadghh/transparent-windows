//! UI-thread window lifetime management.
//!
//! With a single persistent Slint event loop (see `main`), windows are opened
//! with `show()` instead of the blocking `run()`. A shown window stays open only
//! while a strong [`slint::ComponentHandle`] to it is alive, so we stash those
//! handles in a thread-local owned by the UI (event-loop) thread.
//!
//! All access happens on the event-loop thread — either from a window callback
//! or from inside a `slint::invoke_from_event_loop` closure.

use crate::ErrorWindow;
use slint::ComponentHandle;
use std::{
    any::{Any, TypeId},
    cell::RefCell,
    collections::HashMap,
};
use tracing::error;

thread_local! {
    static KEEPALIVE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Window-lifecycle conveniences on every Slint window. All generated components
/// implement [`slint::ComponentHandle`] (which already provides `window()`,
/// `show()`, `hide()`, …), so this blanket-impl extension trait adds the
/// app-specific dance on top — letting callers write `window.show_keep_alive()`
/// instead of repeating show-or-log-then-retain.
pub trait WindowExt: ComponentHandle + 'static {
    /// Show the window and retain it (see [`keep_alive`]); on failure the error is
    /// logged and the window dropped (closing it).
    fn show_keep_alive(self)
    where
        Self: Sized,
    {
        if let Err(e) = self.show() {
            error!(
                error = %e,
                window = std::any::type_name::<Self>(),
                "failed to show window"
            );
            return;
        }
        keep_alive(self);
    }
}

impl<T: ComponentHandle + 'static> WindowExt for T {}

/// Surface a failure to the user in a small dismissable window. Safe to call
/// from any thread — the work is marshalled onto the Slint event loop — so
/// background tasks (config persistence, rule mutations) can report errors that
/// would otherwise be invisible in release builds, where tracing is compiled out.
pub fn report_error(message: impl Into<String>) {
    let message = message.into();
    let _ = slint::invoke_from_event_loop(move || match ErrorWindow::new() {
        Ok(window) => {
            window.set_message(message.into());
            let handle = window.as_weak();
            window.on_dismiss(move || {
                if let Some(window) = handle.upgrade() {
                    let _ = window.hide();
                }
            });
            window.show_keep_alive();
        }
        Err(e) => error!(error = %e, "failed to show error window"),
    });
}

/// Keep a window handle alive, keyed by its concrete type. Each window kind is a
/// distinct type, so the type *is* the identity — reopening a window of the same
/// kind drops the previous one (closing it), keeping at most one of each kind.
/// Windows are closed by hiding them; the stale handle is released on the next
/// open (never mid-callback, which would drop a live component).
pub fn keep_alive<W: Any>(handle: W) {
    KEEPALIVE.with(|map| {
        map.borrow_mut().insert(TypeId::of::<W>(), Box::new(handle));
    });
}

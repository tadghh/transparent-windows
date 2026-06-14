// #![windows_subsystem = "windows"]
#![cfg_attr(test, feature(test))]
//! WinAlpha sets per-window transparency from the system tray, on Windows and
//! Linux. The user picks a window and chooses an opacity; the app stores that as a
//! rule and keeps the window — and future windows of the same class — at that
//! opacity until the rule is removed.
//!
//! The code is split into layers:
//! - [`platform`] — the per-OS window backends behind the [`platform::WindowManager`]
//!   trait (Win32, X11, and KWin/Wayland), plus host-OS integration via
//!   [`platform::OperatingSystem`] (autostart, opening paths, pid lookup).
//! - [`transparency`] — the opacity-rule model, the monitor loop that enforces
//!   rules on polling backends, and the picker / rule-editing UI flows.
//! - [`app`] — the shared runtime [`app::state::AppState`], its persisted
//!   [`app::config`], the [`app::tray`] menu, and Slint window plumbing ([`app::ui`]).
//! - [`identity`] — the app's name and slug, used wherever it announces itself.
//!
//! `main` wires these together: select the window backend ([`platform::init`]),
//! build a Tokio runtime, create the shared [`app::state::AppState`], spawn the
//! window monitor and an initial rule sync, start the [`app::tray`], and run the
//! Slint event loop until the user quits.

#[cfg(test)]
extern crate test;
use anyhow::Result;
use app::{state::AppState, tray};
use std::sync::Arc;
use transparency::monitor::monitor_windows;
mod app;
mod identity;
mod platform;
mod transparency;

slint::include_modules!();

fn main() -> Result<()> {
    platform::init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let app_state = Arc::new(AppState::new(runtime.handle().clone())?);

    runtime.spawn(monitor_windows(app_state.clone()));

    let startup_state = app_state.clone();
    runtime.spawn(async move { startup_state.sync_rules_now().await });

    tray::run(app_state.clone(), runtime.handle().clone());
    slint::run_event_loop_until_quit()?;

    Ok(())
}

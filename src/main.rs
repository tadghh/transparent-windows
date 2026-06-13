// #![windows_subsystem = "windows"]
#![cfg_attr(test, feature(test))]
#[cfg(test)]
extern crate test;
use anyhow::Result;
use app_state::AppState;
use monitor::monitor_windows;
use platform::wm;
use std::sync::Arc;
use tokio::{
    runtime::Handle,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};
use tray::{STARTUP_ID, setup_tray};
use util::{Message, load_config, show_config_error_window};
mod app_state;
mod monitor;
mod platform;
mod transparency;
mod tray;
mod ui;
mod util;
mod window_config;

slint::include_modules!();

fn main() -> Result<()> {
    platform::init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let (config, config_path, config_invalid) = load_config();
    if config_invalid {
        show_config_error_window(config_path.clone());
    }

    let (tx, rx): (UnboundedSender<Message>, UnboundedReceiver<Message>) =
        mpsc::unbounded_channel();

    let app_state = Arc::new(AppState::new(
        config,
        config_path.clone(),
        runtime.handle().clone(),
    ));

    runtime.spawn(monitor_windows(app_state.clone()));

    {
        let startup_state = app_state.clone();
        runtime.spawn(async move { startup_state.sync_rules_now().await });
    }

    let loop_state = app_state.clone();
    let loop_handle = runtime.handle().clone();

    std::thread::spawn(move || message_loop(rx, tx, loop_state, loop_handle));
    slint::run_event_loop_until_quit()?;

    Ok(())
}

/// Receives tray messages and dispatches them. UI work is marshalled onto the
/// event-loop thread with `invoke_from_event_loop`; async work is driven on the
/// background runtime via `handle`.
fn message_loop(
    mut rx: UnboundedReceiver<Message>,
    tx: UnboundedSender<Message>,
    app_state: Arc<AppState>,
    handle: Handle,
) {
    let mut tray = match setup_tray(tx) {
        Ok(tray) => tray,
        Err(e) => {
            eprintln!("Failed to set up tray: {e}");
            let _ = slint::quit_event_loop();
            return;
        }
    };

    while let Some(event) = rx.blocking_recv() {
        match event {
            Message::Quit => {
                let _ = wm().sync_rules(&[]);
                app_state.shutdown.notify_waiters();
                let _ = slint::quit_event_loop();
                break;
            }
            Message::Rules => {
                let rules = handle.block_on(app_state.get_window_rules());
                let state = app_state.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    transparency::show_rules_window(rules, state);
                });
            }
            Message::Add => transparency::start_add_flow(app_state.clone()),
            Message::Enable => handle.block_on(app_state.enabled()),
            Message::Disable => handle.block_on(app_state.disable()),
            Message::Startup => {
                _ = wm().set_autostart(!wm().get_autostart_state());
                let state_string = format!("Startup - {}", wm().get_autostart_state());
                if let Err(e) = tray
                    .inner_mut()
                    .set_menu_item_label(&state_string, STARTUP_ID)
                {
                    eprintln!("Failed to update startup label: {e}");
                }
            }
        }
    }
}

//! Transparency feature: the per-window opacity [`rules`] model, the [`monitor`]
//! loop that applies it on polling backends, and the picker/rule UI flows below.

pub mod monitor;
pub mod rules;

use self::rules::WindowRule;
use crate::{
    MouseInfo, PercentageInput, PercentageWindow, RulesStorage, RulesWindow, TransparencyRule,
    app::{
        state::AppState,
        ui::{WindowExt, keep_alive},
    },
    platform::{PickHover, WindowInfo, percent_to_alpha, wm},
};
use slint::{ComponentHandle, Model, PhysicalPosition, SharedString, VecModel};
use std::{
    rc::Rc,
    sync::{Arc, mpsc::channel},
    thread,
};

// Aligns the cursor overlay with the mouse (window scaling will break this).
const MOUSE_OFFSET: i32 = 15;

/// Drives the "Add" flow: let the user pick a window, then prompt for a percentage.
/// Runs without blocking the event loop — the picker reports its result through a
/// callback, which opens the percentage window back on the UI thread.
pub fn start_add_flow(app_state: Arc<AppState>) {
    pick_window(move |window_info| {
        if window_info.process_name.is_empty() {
            return;
        }
        let _ = slint::invoke_from_event_loop(move || {
            show_percentage_window(window_info, app_state);
        });
    });
}

/// Opens the percentage prompt for a just-picked window, pre-filled with its
/// process and class. Must be called on the Slint event-loop thread.
pub fn show_percentage_window(window_info: WindowInfo, app_state: Arc<AppState>) {
    let window = match PercentageWindow::new() {
        Ok(window) => window,
        Err(e) => {
            eprintln!("Failed to create percentage window: {e}");
            return;
        }
    };
    let window_handle = window.as_weak();
    let submit_handle = window_handle.clone();

    {
        let globals = window.global::<PercentageInput>();
        globals.set_name(window_info.process_name.clone().into());
        globals.set_classname(window_info.class_name.clone().into());
    }

    window.on_submit(move |value: SharedString| {
        if value.is_empty() {
            return;
        }

        if let Ok(number) = value.parse::<u8>() {
            app_state.spawn_update_rule(WindowRule::new(
                &window_info,
                percent_to_alpha(number.into()),
            ));

            if let Some(window) = submit_handle.upgrade() {
                let _ = window.hide();
            }
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            let _ = window.hide();
        }
    });

    window.show_keep_alive();
}

/// Opens the rules window, listing the configured rules so the user can review and
/// edit them. Must be called on the Slint event-loop thread; `rules` is fetched
/// ahead of time and passed in.
pub fn show_rules_window(mut rules: Vec<TransparencyRule>, app_state: Arc<AppState>) {
    let window = match RulesWindow::new() {
        Ok(window) => window,
        Err(e) => {
            eprintln!("Failed to create rules window: {e}");
            return;
        }
    };
    let window_handle = window.as_weak();

    rules.sort_by_key(|rule| rule.process_name.clone());
    let items_model = Rc::new(VecModel::from(rules));

    window
        .global::<RulesStorage>()
        .set_items(items_model.clone().into());

    let app_clone = app_state.clone();
    window.on_submit(move |value: TransparencyRule| {
        app_clone.spawn_update_rule(value.clone().into());
    });

    let items_model_weak = window.as_weak();
    window.on_force(move |value: TransparencyRule| {
        app_state.spawn_force_rule(value.clone().into());
        let app_state_clone = app_state.clone();

        let handle = items_model_weak.clone();
        app_state.runtime().spawn(async move {
            let current_items = app_state_clone.get_window_rules().await;

            handle.upgrade_in_event_loop(move |window| {
                if let Some(items_vec) = window
                    .global::<RulesStorage>()
                    .get_items()
                    .as_any()
                    .downcast_ref::<VecModel<TransparencyRule>>()
                    && let Some(idx) = (0..items_vec.row_count()).find(|&i| {
                        items_vec.row_data(i).unwrap().process_name == value.process_name
                    })
                    && let Some(item) = current_items.get(idx)
                {
                    items_vec.set_row_data(idx, item.clone());
                }
            })?;
            Ok::<(), anyhow::Error>(())
        });
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            let _ = window.hide();
        }
    });

    window.show_keep_alive();
}

/// Lets the user click on a window; the info about the clicked window is delivered
/// to `on_picked`. Non-blocking: a fixed info panel is shown on the event-loop
/// thread, and a worker thread drives the backend's `WindowManager::pick_window`,
/// which blocks until the user clicks while reporting the hovered window back for
/// the panel.
///
/// This is pure UI orchestration (it only drives Slint windows). All OS interaction
/// — cursor polling, the native selector, elevation checks — lives in the platform
/// layer behind the picker, so this code is backend-agnostic.
///
/// Picking is most reliable on a window's border: clicking inside can land on a
/// nested child window rather than the top-level one.
pub fn pick_window(on_picked: impl FnOnce(WindowInfo) + Send + 'static) {
    let (weak_tx, weak_rx) = channel::<slint::Weak<MouseInfo>>();

    let _ = slint::invoke_from_event_loop(move || match MouseInfo::new() {
        Ok(window) => {
            window.set_process_name("Click a window…".into());
            window.set_opacity_error(0);
            let weak = window.as_weak();
            if let Err(e) = window.show() {
                eprintln!("Failed to show picker panel: {e}");
                return;
            }
            let _ = weak_tx.send(weak);
            keep_alive(window);
        }
        Err(e) => eprintln!("Failed to create picker panel: {e}"),
    });

    thread::spawn(move || {
        let panel = weak_rx.recv().ok();
        let on_hover = |hover: PickHover| {
            if let Some(panel) = &panel {
                let _ = panel.upgrade_in_event_loop(move |handle| {
                    handle.set_class_name(hover.info.class_name.into());
                    handle.set_process_name(hover.info.process_name.into());

                    if hover.blocked {
                        handle.set_opacity_error(1);
                        handle.set_error_string(
                            "Administrator rights are required to adjust this window.".into(),
                        );
                    } else {
                        handle.set_opacity_error(0);
                    }

                    if let Some(cursor) = hover.cursor {
                        handle.window().set_position(PhysicalPosition {
                            x: cursor.x + MOUSE_OFFSET,
                            y: cursor.y + MOUSE_OFFSET,
                        });
                    }
                });
            }
        };

        let result = wm().pick_window(&on_hover);

        if let Some(panel) = &panel {
            let _ = panel.upgrade_in_event_loop(|handle| {
                let _ = handle.window().hide();
            });
        }

        match result {
            Ok(Some(info)) => on_picked(info),
            Ok(None) => {}
            Err(e) => eprintln!("Window pick failed: {e}"),
        }
    });
}

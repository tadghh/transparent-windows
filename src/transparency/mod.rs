use crate::{
    app_state::AppState,
    platform::{convert_to_full, wm, CursorPoint, WindowInfo},
    ui::keep_alive,
    window_config::WindowConfig,
    MouseInfo, PercentageInput, PercentageWindow, RulesStorage, RulesWindow, TransparencyRule,
};
use slint::{ComponentHandle, Model, PhysicalPosition, SharedString, VecModel};
use std::{
    rc::Rc,
    sync::{mpsc::channel, Arc},
    thread::{self, sleep},
    time::{Duration, Instant},
};

// Aligns the cursor overlay with the mouse (window scaling will break this).
const MOUSE_OFFSET: i32 = 15;

/*
  Drives the "Add" flow: let the user pick a window, then prompt for a percentage.
  Runs without blocking the event loop — the picker reports its result through a
  callback, which opens the percentage window back on the UI thread.
*/
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

/*
  Creates the process selection window, this is created after the user selected the frame of a window.
  Must be called on the Slint event-loop thread.
*/
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
            app_state.spawn_update_config(WindowConfig::new(
                &window_info,
                convert_to_full(number.into()),
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

    if let Err(e) = window.show() {
        eprintln!("Failed to show percentage window: {e}");
        return;
    }
    keep_alive("percentage", window);
}

/*
  Creates the rules window, this is so the user can see what rules are currently active.
  There is hardcoded minimum of 30%. Must be called on the Slint event-loop thread;
  the current rules are fetched ahead of time and passed in.
*/
pub fn show_rules_window(mut rules: Vec<TransparencyRule>, app_state: Arc<AppState>) {
    let window = match RulesWindow::new() {
        Ok(window) => window,
        Err(e) => {
            eprintln!("Failed to create rules window: {e}");
            return;
        }
    };
    let window_handle = window.as_weak();

    // Create the model once
    rules.sort_by_key(|rule| rule.process_name.clone());
    let items_model = Rc::new(VecModel::from(rules));

    // Set initial items
    window
        .global::<RulesStorage>()
        .set_items(items_model.clone().into());

    // Handle submit events
    let app_clone = app_state.clone();
    window.on_submit(move |value: TransparencyRule| {
        app_clone.spawn_update_config(value.clone().into());
    });

    let items_model_weak = window.as_weak();
    // Handle force events
    window.on_force(move |value: TransparencyRule| {
        app_state.spawn_force_config(value.clone().into());

        let handle = items_model_weak.clone();
        let app_state_clone = app_state.clone();
        app_state.runtime().spawn(async move {
            let current_items = app_state_clone.get_window_rules().await;

            handle.upgrade_in_event_loop(move |window| {
                if let Some(items_vec) = window
                    .global::<RulesStorage>()
                    .get_items()
                    .as_any()
                    .downcast_ref::<VecModel<TransparencyRule>>()
                {
                    if let Some(idx) = (0..items_vec.row_count()).find(|&i| {
                        items_vec.row_data(i).unwrap().process_name == value.process_name
                    }) {
                        items_vec.set_row_data(idx, current_items.get(idx).unwrap().clone());
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        });
    });

    // Handle cancel events
    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            let _ = window.hide();
        }
    });

    if let Err(e) = window.show() {
        eprintln!("Failed to show rules window: {e}");
        return;
    }
    keep_alive("rules", window);
}

/*
  Lets the user click on a window; the info about the clicked window is delivered
  to `on_picked`. Non-blocking: the cursor overlay is shown on the event-loop
  thread and a worker thread polls the OS through the platform primitives.

  This is UI orchestration (it drives Slint windows), so it lives here rather than
  in the platform layer, which exposes only the OS primitives it builds on.

  Note: Should really try to click on the border of the window,
   clicking inside can cause issues since programs have windows inside of other windows (that are not modal).
*/
pub fn pick_window(on_picked: impl FnOnce(WindowInfo) + Send + 'static) {
    // Wayland backends select with a native, compositor-driven picker. We can't
    // make a window follow the cursor (Wayland forbids client self-positioning),
    // so we show a fixed info panel that updates live with the window under the
    // cursor while the native picker handles the click.
    if wm().supports_native_pick() {
        let (weak_tx, weak_rx) = channel::<slint::Weak<MouseInfo>>();

        let _ = slint::invoke_from_event_loop(move || match MouseInfo::new() {
            Ok(window) => {
                window.set_class_name(Default::default());
                window.set_process_name("Click a window…".into());
                window.set_opacity_error(0);
                let weak = window.as_weak();
                if let Err(e) = window.show() {
                    eprintln!("Failed to show picker panel: {e}");
                    return;
                }
                let _ = weak_tx.send(weak);
                keep_alive("picker", window);
            }
            Err(e) => eprintln!("Failed to create picker panel: {e}"),
        });

        thread::spawn(move || {
            let panel = weak_rx.recv().ok();

            let on_hover = |info: WindowInfo| {
                if let Some(panel) = &panel {
                    let _ = panel.upgrade_in_event_loop(move |handle| {
                        handle.set_class_name(info.class_name.into());
                        handle.set_process_name(info.process_name.into());
                    });
                }
            };

            let result = wm().native_pick_with_hover(&on_hover);

            if let Some(panel) = &panel {
                let _ = panel.upgrade_in_event_loop(|handle| {
                    let _ = handle.window().hide();
                });
            }

            match result {
                Ok(Some(info)) => on_picked(info),
                Ok(None) => {}
                Err(e) => eprintln!("Native window pick failed: {e}"),
            }
        });
        return;
    }

    // The overlay window must be created on the event-loop thread; hand its weak
    // handle back to the polling worker.
    let (weak_tx, weak_rx) = channel::<slint::Weak<MouseInfo>>();

    let _ = slint::invoke_from_event_loop(move || match MouseInfo::new() {
        Ok(window) => {
            let weak = window.as_weak();
            if let Err(e) = window.show() {
                eprintln!("Failed to show picker overlay: {e}");
                return;
            }
            let _ = weak_tx.send(weak);
            keep_alive("picker", window);
        }
        Err(e) => eprintln!("Failed to create picker overlay: {e}"),
    });

    thread::spawn(move || {
        let handle_weak = match weak_rx.recv() {
            Ok(weak) => weak,
            Err(_) => return,
        };

        let mut click_point = CursorPoint::default();
        let mut click_point_old = CursorPoint::default();
        let mut last_window_check = Instant::now();
        let mut window_info_old = WindowInfo::default();

        let window_check_interval = Duration::from_millis(25);
        let is_admin = wm().is_running_as_admin();

        loop {
            let now = Instant::now();

            if let Ok(pos) = wm().get_cursor_pos() {
                click_point = pos;
                if pos != click_point_old {
                    click_point_old = pos;
                    let _ = handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.window().set_position(PhysicalPosition {
                            x: pos.x + MOUSE_OFFSET,
                            y: pos.y + MOUSE_OFFSET,
                        });
                    });
                }
            }

            if now.duration_since(last_window_check) >= window_check_interval {
                last_window_check = now;

                if let Ok(window_info) = wm().get_window_info_at(click_point)
                    && window_info_old != window_info
                {
                    window_info_old = window_info.clone();
                    let point = click_point;

                    let _ = handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.set_class_name(window_info.class_name.into());
                        handle.set_process_name(window_info.process_name.into());

                        if wm().is_elevated_at(point) && !is_admin {
                            handle.set_opacity_error(1);
                            handle.set_error_string(
                                "Administrator rights are required to adjust this window.".into(),
                            );
                        } else {
                            handle.set_opacity_error(0);
                        }
                    });
                }
            }

            if wm().is_left_click() {
                let window_io = wm().get_window_info_at(click_point).unwrap_or_default();

                // Hide the overlay and hand the result back.
                let _ = handle_weak.upgrade_in_event_loop(|handle| {
                    let _ = handle.window().hide();
                });
                on_picked(window_io);

                // Back to the caller we go!
                break;
            }

            // Yield between polls; small enough to feel responsive without busy-spinning.
            sleep(Duration::from_millis(8));
        }
    });
}

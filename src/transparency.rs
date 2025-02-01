use std::{rc::Rc, sync::Arc};

use crate::{app_state::AppState, RulesStorage, RulesWindow, TransparencyRule};

use slint::{ComponentHandle, Model, VecModel};

/*
  Creates the rules window, this is so the user can see what rules are currently active.
  There is hardcoded minimum of 30%
*/
pub async fn create_rules_window(app_state: Arc<AppState>) -> Result<(), core::fmt::Error> {
    let window = RulesWindow::new().unwrap();
    let window_handle = window.as_weak();

    // Create the model once
    let mut window_info = app_state.get_window_rules().await;
    window_info.sort_by_key(|rule| rule.process_name.clone());
    let items_model = Rc::new(VecModel::from(window_info));

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
        tokio::spawn(async move {
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
            window.hide().unwrap();
        }
    });

    window.run().unwrap();
    Ok(())
}

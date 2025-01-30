use std::{rc::Rc, sync::Arc};

use crate::{app_state::AppState, RulesStorage, RulesWindow, TransparencyRule};

use slint::{ComponentHandle, VecModel};

/*
  Creates the rules window, this is so the user can see what rules are currently active.
  There is hardcoded minimum of 30%
*/
pub async fn create_rules_window(app_state: Arc<AppState>) -> Result<(), core::fmt::Error> {
    let window = RulesWindow::new().unwrap();
    let window_handle = window.as_weak();

    let mut window_info = app_state.get_window_rules().await;

    // Oh boo hoo its sorted every time the rules window is opened 😢
    window_info.sort_by_key(|rule| rule.process_name.clone());

    window
        .global::<RulesStorage>()
        .set_items(Rc::new(VecModel::from(window_info)).into());

    window.on_submit(move |value: TransparencyRule| {
        app_state.spawn_update_config(value.into());
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
        }
    });

    window.run().unwrap();
    Ok(())
}

use windows::Win32::{
    Foundation::{BOOL, COLORREF, LPARAM},
    UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, IsWindowVisible, SetLayeredWindowAttributes, SetWindowLongW,
        GWL_EXSTYLE, LAYERED_WINDOW_ATTRIBUTES_FLAGS, WS_EX_LAYERED,
    },
};

use super::*;

pub fn monitor_windows(app_state: Arc<AppState>) {
    loop {
        let config = app_state.config.lock().unwrap();

        for (_, window_config) in &config.windows {
            // Update
            update_window_transparency(window_config);
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn update_window_transparency(config: &WindowConfig) {
    unsafe {
        match EnumWindows(Some(enum_window_proc), LPARAM(config as *const _ as isize)) {
            Ok(()) => (),
            Err(err) => print!("{:?}", err),
        }
    }
}

unsafe extern "system" fn enum_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let config = &*(lparam.0 as *const WindowConfig);

    if !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }

    let mut class_name = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut class_name);
    let window_class = String::from_utf16_lossy(&class_name[..len as usize]);

    if window_class == config.window_class {
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        if let Ok(process_name) = win_utils::get_process_name(process_id) {
            if process_name == config.process_name {
                let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED.0 as i32);
                // SetLayeredWindowAttributes(hwnd, 0, config.transparency, 2);
                match SetLayeredWindowAttributes(
                    hwnd,
                    COLORREF(0),
                    config.transparency,
                    LAYERED_WINDOW_ATTRIBUTES_FLAGS(2),
                ) {
                    Ok(()) => (),
                    Err(err) => println!("gayt {:?} {:?}", err, config),
                }
            }
        }
    }

    true.into()
}

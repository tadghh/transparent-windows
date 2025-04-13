use crate::{
    app_state::AppState, window_config::WindowConfig, MouseInfo, PercentageInput, PercentageWindow,
};

use anyhow::{anyhow, Result};
use core::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
use slint::{ComponentHandle, PhysicalPosition, SharedString};
use std::{
    env::current_exe,
    os::raw::c_void,
    sync::Arc,
    thread::{self, sleep},
    time::Instant,
};
use windows::{
    core::{PCSTR, PWSTR},
    Win32::{
        Foundation::{COLORREF, ERROR_SUCCESS, HANDLE, HWND, MAX_PATH, POINT},
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::{
            Registry::{
                RegCloseKey, RegCreateKeyExA, RegDeleteValueA, RegOpenKeyExA, RegQueryValueExA,
                RegSetValueExA, HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ,
                REG_OPTION_NON_VOLATILE, REG_SZ,
            },
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
                PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
            WindowsAndMessaging::{
                GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowThreadProcessId,
                SetLayeredWindowAttributes, SetWindowLongW, WindowFromPoint, GWL_EXSTYLE,
                LAYERED_WINDOW_ATTRIBUTES_FLAGS, WS_EX_LAYERED,
            },
        },
    },
};

// This is "left click"
const KEY_PRESSED: i16 = 0x8000u16 as i16;

// Aligns the mouse cursor (window scaling will break this)
const MOUSE_OFFSET: i32 = 15;

const MINIMUM_TRANSPARENCY: i32 = 30;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowInfo {
    pub class_name: String,
    pub process_name: String,
}

/*
  This function is called to allow the user to click on a window, the info about the window is returned.

  Note: Should really try to click on the border of the window, clicking inside causes issues
*/
pub fn get_window_under_cursor() -> Result<WindowInfo> {
    let window = MouseInfo::new()?;
    let handle_weak = window.as_weak();
    let (tx, rx): (Sender<WindowInfo>, Receiver<WindowInfo>) = bounded(1);

    let window_thread = thread::spawn(move || {
        let mut click_point = POINT::default();
        let mut click_point_old = POINT::default();
        let mut last_window_check = Instant::now();
        let mut window_info_old = WindowInfo::default();

        let window_check_interval = Duration::from_millis(25);
        let is_admin = is_running_as_admin();

        loop {
            let now = Instant::now();

            unsafe {
                if GetCursorPos(&mut click_point).is_ok() && click_point != click_point_old {
                    click_point_old = click_point;
                    handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.window().set_position(PhysicalPosition {
                            x: click_point.x + MOUSE_OFFSET,
                            y: click_point.y + MOUSE_OFFSET,
                        });
                    })?;
                }
            }

            if now.duration_since(last_window_check) >= window_check_interval {
                last_window_check = now;

                if let Some(window_info) = get_window_info(click_point).ok()
                    && window_info_old != window_info
                {
                    window_info_old = window_info.clone();

                    handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.set_class_name(window_info.class_name.into());
                        handle.set_process_name(window_info.process_name.into());

                        if is_elevated(click_point) && !is_admin {
                            handle.set_opacity_error(1);
                            handle.set_error_string(
                                "Not happening, we need admin rights for this one".into(),
                            );
                        } else {
                            handle.set_opacity_error(0);
                        }
                    })?;
                }
            }

            if is_left_click() {
                let window_io = get_window_info(click_point)?;
                tx.send(window_io)?;

                // Back to main we go!
                break;
            }

            // Smaller sleep to maintain responsiveness
            sleep(Duration::from_micros(125));
        }

        handle_weak.upgrade_in_event_loop(|handle| {
            handle
                .window()
                .hide()
                .expect("Window cursor thread failed.")
        })?;

        Ok::<(), anyhow::Error>(())
    });

    window.run()?;

    // Wait for window thread to clean up
    window_thread
        .join()
        .map_err(|_| anyhow!("Window thread panicked."))??;

    match rx.try_recv() {
        Ok(window_info) => Ok(window_info),
        Err(_) => Ok(WindowInfo::default()),
    }
}

#[inline]
fn is_left_click() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0.into()) & KEY_PRESSED) != 0 }
}

fn get_window_info(point: POINT) -> Result<WindowInfo> {
    unsafe {
        let hwnd = WindowFromPoint(point);
        if hwnd.0 == std::ptr::null_mut() {
            return Err(anyhow!("No window found at cursor position."));
        }

        let mut class_name = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_name);
        let window_class = String::from_utf16_lossy(&class_name[..class_len as usize]);

        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        Ok(WindowInfo {
            class_name: window_class,
            process_name: get_process_name(process_id)?,
        })
    }
}

/*
  Gets the process name from a provided process id.
*/
pub fn get_process_name(process_id: u32) -> Result<String> {
    unsafe {
        // Stack allocate buffer
        let mut buffer = [0u16; MAX_PATH as usize];
        let mut size = buffer.len() as u32;

        // Use ? operator for early return
        let process_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
            .map_err(|_| anyhow!("Failed to get process handle."))?;

        // Get process name
        QueryFullProcessImageNameW(
            process_handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .map_err(|_| anyhow!("Failed to get process name."))?;

        // Extract filename without extension
        let full_path = String::from_utf16_lossy(&buffer[..size as usize]);
        Ok(full_path
            .rsplit('\\') // rsplit is slightly faster for getting last element
            .next()
            .and_then(|s| s.split('.').next())
            .unwrap_or("")
            .to_string())
    }
}

/*
  Convert a value from 1 - 100 to its u8 (255) equivalent.
*/
pub fn convert_to_full(mut value: i32) -> u8 {
    if value < MINIMUM_TRANSPARENCY {
        value = MINIMUM_TRANSPARENCY;
    }
    if value > 100 {
        return 255;
    }
    ((value as f32 / 100.0) * 255.0).round() as u8
}

/*
  Takes a u8 (255) value and converts it to a measureable format (a percentage of 100)
*/
pub fn convert_to_human(value: u8) -> u8 {
    ((value as f32 / 255.0) * 100.0).round() as u8
}

/*
  Creates the process selection window, this is created after the user selected the frame of a window.
*/
pub async fn create_percentage_window(
    window_info: WindowInfo,
    app_state: Arc<AppState>,
) -> Result<(), anyhow::Error> {
    let window = PercentageWindow::new()?;
    let window_handle = window.as_weak();
    let submit_handle = window_handle.clone();

    {
        let globals = window.global::<PercentageInput>();
        globals.set_name(window_info.process_name.clone().into());
        globals.set_classname(window_info.class_name.clone().into());
    }

    window.on_submit(move |value: SharedString| {
        let value_string = value.to_owned();

        if value_string.is_empty() {
            return;
        }

        let window_info = window_info.clone();
        if let Ok(number) = value_string.parse::<u8>() {
            let value = convert_to_full(number.into());

            let window_config =
                WindowConfig::new(window_info.process_name, window_info.class_name, value);

            let app_state = Arc::clone(&app_state);

            app_state.spawn_update_config(window_config);

            if let Some(window) = submit_handle.upgrade() {
                window.hide().expect("Failed to hide percentage window.");
            }
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().expect("Failed to hide percentage window.");
        }
    });

    window.run()?;
    Ok(())
}

/*
  Sets the transparency of the handles window.
*/
pub fn make_window_transparent(window_handle: HWND, transparency: u8) -> Result<(), anyhow::Error> {
    unsafe {
        SetWindowLongW(
            window_handle,
            GWL_EXSTYLE,
            GetWindowLongW(window_handle, GWL_EXSTYLE) | WS_EX_LAYERED.0 as i32,
        );

        match SetLayeredWindowAttributes(
            window_handle,
            COLORREF(0),
            transparency,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS(2),
        ) {
            Ok(()) => (),
            Err(err) => return Err(anyhow!("Failed to get process handle {}", err)),
        }
    }
    Ok(())
}

/*
  Returns if the window below the cursor is running as admin.
  Used by the UI to make the user aware when a program they want to select a administrator program.
  Hopefully they will realize they need to run WinAlpha to select it.
*/
fn is_elevated(point: POINT) -> bool {
    let mut process_id = 0;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = size_of::<TOKEN_ELEVATION>() as u32;
    let mut token = HANDLE::default();

    unsafe {
        let hwnd = WindowFromPoint(point);
        if hwnd.0 == std::ptr::null_mut() {
            return false;
        }

        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
            .ok()
            .unwrap();

        if !OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok() {
            return false;
        }

        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            size,
            &mut size,
        )
        .ok();
    }

    elevation.TokenIsElevated != 0
}

/*
 Check if we are running as admin.
*/
fn is_running_as_admin() -> bool {
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
    let mut token = HANDLE::default();

    unsafe {
        if !OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            return false;
        }

        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut c_void),
            size,
            &mut size,
        )
        .map_or(false, |_| elevation.TokenIsElevated != 0)
    }
}

/*
 Enables/disables autostart of WinAlpha.
 Done by adding a registry key for the current user under "run" this key is created with the current path WinAlpha was executed with
*/
pub fn change_startup(current_state: bool) -> windows::core::Result<()> {
    let mut startup_key = HKEY::default();

    let path_str = PCSTR::from_raw(b"Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".as_ptr());
    let app_name = PCSTR::from_raw(b"WinAlpha\0".as_ptr());
    let exe_path = current_exe()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    unsafe {
        _ = RegCreateKeyExA(
            HKEY_CURRENT_USER,
            path_str,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            None,
            &mut startup_key,
            None,
        );

        if current_state {
            // State is true so we add the startup key
            _ = RegSetValueExA(
                startup_key,
                app_name,
                Some(0),
                REG_SZ,
                Some(exe_path.as_bytes()),
            );
        } else {
            // It false so remove it
            _ = RegDeleteValueA(startup_key, app_name);
        }
        // close reg key handle
        _ = RegCloseKey(startup_key);
    }

    Ok(())
}

/*
 Returns if autostart is enabled.
 This is done by checking if the startup registry key exists for the current user.
*/
pub fn get_startup_state() -> bool {
    let key: HKEY = HKEY_CURRENT_USER;
    let path_str = PCSTR::from_raw(b"Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".as_ptr());
    let app_name = PCSTR::from_raw(b"WinAlpha\0".as_ptr());

    let mut startup_key = HKEY::default();
    let mut size = 0u32;

    unsafe {
        let result = RegOpenKeyExA(key, path_str, Some(0), KEY_READ, &mut startup_key);

        if result != ERROR_SUCCESS {
            return false;
        }

        // Query the size first
        let result = RegQueryValueExA(startup_key, app_name, None, None, None, Some(&mut size));

        if result != ERROR_SUCCESS {
            return false;
        }

        let result = RegQueryValueExA(
            startup_key,
            app_name,
            None,
            None,
            Some(Vec::with_capacity(size as usize).as_mut_ptr()),
            Some(&mut size),
        );

        return result == ERROR_SUCCESS;
    }
}

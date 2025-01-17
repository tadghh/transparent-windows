use crate::{MouseInfo, PercentageInput, PercentageWindow};
use anyhow::{anyhow, Error, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use slint::{ComponentHandle, PhysicalPosition, SharedString};

use core::time::Duration;
use std::{sync::mpsc, thread, time::Instant};
use windows::{
    core::{PCWSTR, PWSTR},
    Win32::{
        Foundation::{COLORREF, HWND, MAX_PATH, POINT},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
            PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
            WindowsAndMessaging::{
                FindWindowExW, FindWindowW, GetClassNameW, GetCursorPos, GetWindowLongW,
                GetWindowThreadProcessId, SetLayeredWindowAttributes, SetWindowLongW,
                WindowFromPoint, GWL_EXSTYLE, LAYERED_WINDOW_ATTRIBUTES_FLAGS, WS_EX_LAYERED,
            },
        },
    },
};
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    pub class_name: String,
    pub process_name: String,
}

const MOUSE_OFFSET: i32 = 15;

/*
  This function is called to allow the user to click on a window, the info about the window is returned.

  Note: Should really try to click on the border of the window, clicking inside causes issues
*/
pub fn get_window_under_cursor() -> Result<WindowInfo> {
    let window = MouseInfo::new()?;
    let handle_weak = window.as_weak();
    let (tx, rx): (Sender<WindowInfo>, Receiver<WindowInfo>) = bounded(1);

    let window_thread = thread::spawn(move || {
        #[allow(unused_assignments)]
        let mut last_window_info: Option<WindowInfo> = None;
        let mut click_point = POINT::default();
        let mut last_window_check = Instant::now();
        let window_check_interval = Duration::from_millis(50);

        loop {
            unsafe {
                let now = Instant::now();
                if now.duration_since(last_window_check) >= window_check_interval {
                    last_window_check = now;

                    // Looks laggy swapping across windows
                    if GetCursorPos(&mut click_point).is_ok() {
                        handle_weak.upgrade_in_event_loop(move |handle| {
                            handle.window().set_position(PhysicalPosition {
                                x: click_point.x + MOUSE_OFFSET,
                                y: click_point.y + MOUSE_OFFSET,
                            });
                        })?;
                    }
                    let window_info = get_window_info(click_point)?;
                    handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.set_class_name(window_info.class_name.into());
                        handle.set_process_name(window_info.process_name.into());
                    })?;
                }
                if (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 {
                    if GetCursorPos(&mut click_point).is_ok() {
                        let window_io = get_window_info(click_point)?;
                        tx.send(window_io.clone())?;
                        last_window_info = Some(window_io);
                        handle_weak
                            .upgrade_in_event_loop(|handle| handle.window().hide().unwrap())?;
                        // Back to main we go!
                        break;
                    }
                    return Err(anyhow!("Failed to get cursor position"));
                }

                if GetCursorPos(&mut click_point).is_ok() {
                    handle_weak.upgrade_in_event_loop(move |handle| {
                        handle.window().set_position(PhysicalPosition {
                            x: click_point.x + MOUSE_OFFSET,
                            y: click_point.y + MOUSE_OFFSET,
                        });
                    })?;
                }
            }

            // Smaller sleep to maintain responsiveness
            std::thread::sleep(Duration::from_micros(125));
        }

        handle_weak.upgrade_in_event_loop(|handle| handle.window().hide().unwrap())?;
        Ok(last_window_info.unwrap_or_else(|| WindowInfo::default()))
    });

    window.run()?;

    // Wait for window thread to clean up
    let thread_result = window_thread
        .join()
        .map_err(|_| anyhow!("Window thread panicked"))?;

    match rx.try_recv() {
        Ok(window_info) => Ok(window_info),
        Err(_) => thread_result.or_else(|_| get_window_info(POINT::default())),
    }
}

fn get_window_info(point: POINT) -> Result<WindowInfo> {
    unsafe {
        let hwnd = WindowFromPoint(point);
        if hwnd.0 == std::ptr::null_mut() {
            return Err(anyhow!("No window found at cursor position"));
        }

        let mut class_name = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_name);
        let window_class = String::from_utf16_lossy(&class_name[..class_len as usize]);

        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        let process_name = get_process_name(process_id)?;
        Ok(WindowInfo {
            class_name: window_class,
            process_name,
        })
    }
}

/*
  Gets the process name from a provided process id.
*/
pub fn get_process_name(process_id: u32) -> Result<String> {
    unsafe {
        // Open a handle to the process
        let process_handle = match OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            process_id,
        )
        .ok()
        {
            Some(handle) => handle,
            None => return Err(anyhow!("Failed to get process handle")),
        };

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut size = buffer.len() as u32;

        if QueryFullProcessImageNameW(
            process_handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .is_ok()
        {
            // Extract just the file name from the path
            let full_path = String::from_utf16_lossy(&buffer[..size as usize]);
            let file_name = full_path
                .split('\\')
                .last()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("");

            Ok(file_name.to_string())
        } else {
            Err(anyhow!("Failed to get process name"))
        }
    }
}

const MINIMUM_PERCENTAGE: i32 = 30;

/*
  Convert a value from 1 - 100 to its u8 (255) equivalent.
*/
pub fn convert_to_full(mut value: i32) -> u8 {
    if value < MINIMUM_PERCENTAGE {
        value = MINIMUM_PERCENTAGE;
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
pub fn create_percentage_window(window_info: WindowInfo) -> Option<u8> {
    let (sender, receiver) = mpsc::channel();
    let window = PercentageWindow::new().unwrap();
    let window_handle = window.as_weak();
    let submit_handle = window_handle.clone();
    let cancel_sender = sender.clone();
    {
        let globals = window.global::<PercentageInput>();
        globals.set_name(window_info.process_name.into());
        globals.set_classname(window_info.class_name.into());
    }

    window.on_submit(move |value: SharedString| {
        let value_string = value.to_string();

        if value_string.is_empty() {
            return;
        }

        if let Ok(number) = value_string.parse::<u8>() {
            let value = convert_to_full(number.into());

            let _ = sender.send(Some(value));
            if let Some(window) = submit_handle.upgrade() {
                window.hide().unwrap();
            }
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
            cancel_sender.send(None).ok();
        }
    });

    let result = {
        window.run().unwrap();
        receiver.recv().ok()
    };

    result?
}

/*
  Returns all the current handles for the classname
*/
pub fn get_window_hwnds(classname: String) -> Vec<HWND> {
    let mut hwds = Vec::new();
    unsafe {
        let wide_class: Vec<u16> = classname.encode_utf16().chain(std::iter::once(0)).collect();

        match FindWindowW(PCWSTR::from_raw(wide_class.as_ptr()), PCWSTR::null()) {
            Ok(mut new_handle) => {
                if !new_handle.is_invalid() {
                    hwds.push(new_handle);
                }

                while !new_handle.is_invalid() {
                    match FindWindowExW(
                        None,
                        Some(new_handle),
                        PCWSTR::from_raw(wide_class.as_ptr()),
                        PCWSTR::null(),
                    ) {
                        Ok(next_handle) => {
                            if next_handle.is_invalid() {
                                break;
                            }
                            new_handle = next_handle;
                            hwds.push(new_handle);
                        }
                        Err(_) => break,
                    }
                }
            }
            Err(_) => (),
        }
    }
    hwds
}

/*
  Makes the window with the provided handle transparent.
*/
pub fn make_window_transparent(window_handle: HWND, transparency: &u8) -> Result<(), Error> {
    unsafe {
        SetWindowLongW(
            window_handle,
            GWL_EXSTYLE,
            GetWindowLongW(window_handle, GWL_EXSTYLE) | WS_EX_LAYERED.0 as i32,
        );

        match SetLayeredWindowAttributes(
            window_handle,
            COLORREF(0),
            *transparency,
            LAYERED_WINDOW_ATTRIBUTES_FLAGS(2),
        ) {
            Ok(()) => (),
            Err(err) => println!("{:?}", err),
        }
    }
    Ok(())
}

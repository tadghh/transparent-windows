use crate::PercentageTest;
use crate::PercentageWindow;
use anyhow::Error;
use anyhow::Result;
use slint::ComponentHandle;
use slint::SharedString;
use std::io::Write;
use std::sync::mpsc;

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

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub class_name: String,
    pub process_name: String,
}

pub fn get_window_under_cursor() -> Result<WindowInfo> {
    unsafe {
        // Variable to store click position
        let mut click_point = POINT::default();

        // Wait for left mouse button click
        loop {
            let key_state = GetAsyncKeyState(VK_LBUTTON.0 as i32);
            if (key_state as u16 & 0x8000) != 0 {
                // Get cursor position immediately when click is detected
                if !GetCursorPos(&mut click_point).is_ok() {
                    return Err(anyhow::anyhow!("Failed to get cursor position"));
                }
                println!("Click detected at ({}, {})", click_point.x, click_point.y);
                break;
            }
            print!("Waiting for click...\r");
            std::io::stdout().flush().unwrap();
        }

        let hwnd = WindowFromPoint(click_point);
        if hwnd.0 == std::ptr::null_mut() {
            return Err(anyhow::anyhow!("No window found at cursor position"));
        }

        // Get window title

        // Get window class name
        let mut class_name = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_name);
        let window_class = String::from_utf16_lossy(&class_name[..class_len as usize]);

        // Get process ID
        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        // Get process name
        let process_name = get_process_name(process_id)?;
        let info = WindowInfo {
            class_name: window_class,
            process_name,
        };
        println!("{:?}", info);
        Ok(info)
    }
}

pub fn get_process_name(process_id: u32) -> Result<String> {
    unsafe {
        // Open a handle to the process
        let process_handle = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            process_id,
        )
        .ok()
        .unwrap();

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
            Err(anyhow::anyhow!("Failed to get process name"))
        }
    }
}

pub fn create_percentage_window(window_info: WindowInfo) -> Option<u8> {
    let (sender, receiver) = mpsc::channel();
    let window = PercentageWindow::new().unwrap();
    let window_handle = window.as_weak();
    let submit_handle = window_handle.clone();

    {
        let globals = window.global::<PercentageTest>();
        globals.set_name(window_info.process_name.into());
        globals.set_classname(window_info.class_name.into());
    }

    window.on_submit(move |value: SharedString| {
        let value_string = value.to_string();

        if value_string.is_empty() {
            return;
        }

        if let Ok(number) = value_string.parse::<u8>() {
            let value = ((number as f32 / 100.0) * 255.0) as u8;

            let _ = sender.send(value);
            if let Some(window) = submit_handle.upgrade() {
                window.hide().unwrap();
                drop(window);
            }
        }
    });

    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
            drop(window);
        }
    });

    let result = {
        window.run().unwrap();
        receiver.recv().ok()
    };

    drop(window);
    // Receive the value from the channel after the submit
    result
}

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

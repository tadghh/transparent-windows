use core::time::Duration;
use std::io::Write;

use slint::SharedString;
use thread::sleep;
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::MAX_PATH,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
        },
        UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
    },
};

use super::*;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub handle: HWND,
    pub title: String,
    pub class_name: String,
    pub process_name: String,
    pub process_id: u32,
}
pub fn get_window_under_cursor() -> Result<WindowInfo> {
    unsafe {
        // Load cross cursor
        // let cursor = LoadCursorW(None, IDC_CROSS);
        println!(" detected!");

        // Variable to store click position
        let mut click_point = windows::Win32::Foundation::POINT::default();

        // Wait for left mouse button click
        loop {
            let key_state = GetAsyncKeyState(VK_LBUTTON.0 as i32);
            if (key_state as u16 & 0x8000) != 0 {
                // Get cursor position immediately when click is detected
                if !windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut click_point).is_ok()
                {
                    return Err(anyhow::anyhow!("Failed to get cursor position"));
                }
                println!("Click detected at ({}, {})", click_point.x, click_point.y);
                break;
            }
            print!("Waiting for click...\r");
            std::io::stdout().flush().unwrap();
            sleep(Duration::from_millis(10));
        }

        // Small delay to ensure click is complete
        sleep(Duration::from_millis(50));

        // Get window handle using stored click position
        let hwnd = windows::Win32::UI::WindowsAndMessaging::WindowFromPoint(click_point);
        if hwnd.0 == std::ptr::null_mut() {
            return Err(anyhow::anyhow!("No window found at cursor position"));
        }

        // Get window title
        let mut title = [0u16; 512];
        let title_len = GetWindowTextW(hwnd, &mut title);
        let window_title = String::from_utf16_lossy(&title[..title_len as usize]);

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
            handle: hwnd,
            title: window_title,
            class_name: window_class,
            process_name,
            process_id,
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
        )?;

        use windows::Win32::System::Threading::PROCESS_NAME_FORMAT;

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

use std::sync::mpsc;
pub fn create_percentage_window(window2: WindowInfo) -> Option<u8> {
    let (sender, receiver) = mpsc::channel();
    let window = PercentageWindow::new().unwrap();
    window
        .global::<PercentageTest>()
        .set_name(window2.process_name.into());
    window
        .global::<PercentageTest>()
        .set_classname(window2.class_name.into());
    let window_handle = window.as_weak();
    let submit_handle = window_handle.clone();
    // Handle submit
    window.on_submit(move |value: SharedString| {
        let value_string = value.to_string();
        println!("Input received: {}", value_string);

        if value_string.is_empty() {
            println!("No input to send!");
            return;
        }

        // Parse the input string to a number
        if let Ok(number) = value_string.parse::<u8>() {
            // Calculate the value as a fraction of 255
            let result = (number as f32 / 100.0) * 255.0;
            let value = result as u8;
            println!("Calculated value: {}", value);

            // Send the value through the channel
            let _ = sender.send(value);
            if let Some(window) = submit_handle.upgrade() {
                window.hide().unwrap();
            }
        } else {
            println!("Invalid number entered");
        }
    });

    // Handle cancel
    window.on_cancel(move || {
        if let Some(window) = window_handle.upgrade() {
            window.hide().unwrap();
        }
    });

    window.run().unwrap();

    // Receive the value from the channel after the submit
    receiver.recv().ok()
}

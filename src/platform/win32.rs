use super::{CursorPoint, WindowHandle, WindowInfo, WindowManager};
use anyhow::{Result, anyhow};
use core::ffi::c_void;
use std::{env::current_exe, iter::once};
use windows::{
    Win32::{
        Foundation::{
            BOOL, COLORREF, CloseHandle, ERROR_SUCCESS, HANDLE, HWND, LPARAM, MAX_PATH, POINT,
        },
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::{
            ProcessStatus::GetProcessImageFileNameA,
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ, REG_OPTION_NON_VOLATILE, REG_SZ,
                RegCloseKey, RegCreateKeyExA, RegDeleteValueA, RegOpenKeyExA, RegQueryValueExA,
                RegSetValueExA,
            },
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_FORMAT,
                PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                EnumChildWindows, EnumWindows, FindWindowExW, FindWindowW, GWL_EXSTYLE,
                GetClassNameW, GetCursorPos, GetWindowLongW, GetWindowThreadProcessId,
                LAYERED_WINDOW_ATTRIBUTES_FLAGS, SHOW_WINDOW_CMD, SetLayeredWindowAttributes,
                SetWindowLongW, WS_EX_LAYERED, WindowFromPoint,
            },
        },
    },
    core::{PCSTR, PCWSTR, PWSTR, w},
};

// TODO finish abstraction

// This is "left click"
const KEY_PRESSED: i16 = 0x8000u16 as i16;

#[inline]
fn to_hwnd(handle: WindowHandle) -> HWND {
    HWND(handle.0 as *mut c_void)
}

#[inline]
fn from_hwnd(hwnd: HWND) -> WindowHandle {
    WindowHandle(hwnd.0 as u64)
}

pub struct Win32Manager;

impl Win32Manager {
    pub fn new() -> Self {
        Self
    }
}

impl WindowManager for Win32Manager {
    /*
      Sets the transparency of the handles window.
    */
    fn set_window_alpha(&self, handle: WindowHandle, alpha: u8) -> Result<()> {
        let window_handle = to_hwnd(handle);
        unsafe {
            SetWindowLongW(
                window_handle,
                GWL_EXSTYLE,
                GetWindowLongW(window_handle, GWL_EXSTYLE) | WS_EX_LAYERED.0 as i32,
            );

            match SetLayeredWindowAttributes(
                window_handle,
                COLORREF(0),
                alpha,
                LAYERED_WINDOW_ATTRIBUTES_FLAGS(2),
            ) {
                Ok(()) => (),
                Err(err) => return Err(anyhow!("Failed to get process handle {}", err)),
            }
        }
        Ok(())
    }

    /*
      Returns all the current handles for the classname that also match the process.
    */
    fn enumerate_windows(&self, process_name: &str, window_class: &str) -> Vec<WindowHandle> {
        let wide_class: Vec<u16> = window_class.encode_utf16().chain(once(0)).collect();

        let class_ptr = PCWSTR::from_raw(wide_class.as_ptr());
        let mut handles = Vec::new();

        unsafe {
            if let Ok(mut hwnd) = FindWindowW(class_ptr, None) {
                while !hwnd.is_invalid() {
                    let mut process_id = 0;
                    GetWindowThreadProcessId(hwnd, Some(&mut process_id));

                    if let Ok(process_handle) =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)
                    {
                        let mut buffer = [0u8; 260];
                        let len = GetProcessImageFileNameA(process_handle, &mut buffer);
                        _ = CloseHandle(process_handle);

                        if len > 0 {
                            let path_str =
                                String::from_utf8_lossy(&buffer[..len as usize]).to_string();
                            let name = std::path::Path::new(&path_str)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.split('.').next().unwrap_or(s));

                            if let Some(name) = name {
                                if name == process_name {
                                    handles.push(from_hwnd(hwnd));
                                }
                            }
                        }
                    }
                    hwnd = match FindWindowExW(None, Some(hwnd), class_ptr, None) {
                        Ok(next_hwnd) if !next_hwnd.is_invalid() => next_hwnd,
                        _ => break,
                    };
                }
            }
        }

        handles
    }

    fn find_parent_from_child_class(
        &self,
        child_class: &str,
    ) -> Result<Option<(WindowHandle, String)>> {
        let child_hwnd = match find_window_by_class(child_class)? {
            Some(hwnd) => hwnd,
            None => return Ok(None),
        };

        Ok(get_window_class_name(child_hwnd).map(|class_name| (from_hwnd(child_hwnd), class_name)))
    }

    fn get_cursor_pos(&self) -> Result<CursorPoint> {
        unsafe {
            let mut point = POINT::default();
            GetCursorPos(&mut point)
                .map_err(|err| anyhow!("Failed to get cursor position {}", err))?;
            Ok(CursorPoint {
                x: point.x,
                y: point.y,
            })
        }
    }

    #[inline]
    fn is_left_click(&self) -> bool {
        unsafe { (GetAsyncKeyState(VK_LBUTTON.0.into()) & KEY_PRESSED) != 0 }
    }

    /*
      Gets information that will be used to store and identify the window
    */
    fn get_window_info_at(&self, point: CursorPoint) -> Result<WindowInfo> {
        unsafe {
            let hwnd = WindowFromPoint(POINT {
                x: point.x,
                y: point.y,
            });
            if hwnd.0.is_null() {
                return Err(anyhow!("No window found at cursor position."));
            }

            let mut class_name = [0u16; 256];
            let class_len = GetClassNameW(hwnd, &mut class_name);
            let window_class = String::from_utf16_lossy(&class_name[..class_len as usize]);

            let mut process_id = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));

            Ok(WindowInfo {
                class_name: window_class,
                process_name: self.process_name_from_pid(process_id)?,
            })
        }
    }

    /*
      Returns if the window below the cursor is running as admin.
    */
    fn is_elevated_at(&self, point: CursorPoint) -> bool {
        let mut process_id = 0;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = size_of::<TOKEN_ELEVATION>() as u32;
        let mut token = HANDLE::default();

        unsafe {
            let hwnd = WindowFromPoint(POINT {
                x: point.x,
                y: point.y,
            });
            if hwnd.0.is_null() {
                return false;
            }

            GetWindowThreadProcessId(hwnd, Some(&mut process_id));

            let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                Ok(process) => process,
                Err(_) => return false,
            };

            if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_err() {
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
    fn is_running_as_admin(&self) -> bool {
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        let mut token = HANDLE::default();

        unsafe {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }

            GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut c_void),
                size,
                &mut size,
            )
            .is_ok_and(|_| elevation.TokenIsElevated != 0)
        }
    }

    /*
     Enables/disables autostart of WinAlpha via a registry "Run" key.
    */
    fn set_autostart(&self, enabled: bool) -> Result<()> {
        let mut startup_key = HKEY::default();

        let path_str =
            PCSTR::from_raw(b"Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".as_ptr());
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

            if enabled {
                _ = RegSetValueExA(
                    startup_key,
                    app_name,
                    Some(0),
                    REG_SZ,
                    Some(exe_path.as_bytes()),
                );
            } else {
                _ = RegDeleteValueA(startup_key, app_name);
            }
            // Close reg key handle
            _ = RegCloseKey(startup_key);
        }

        Ok(())
    }

    /*
     Returns if autostart is enabled, by checking if the registry key exists.
    */
    fn get_autostart_state(&self) -> bool {
        let key: HKEY = HKEY_CURRENT_USER;
        let path_str =
            PCSTR::from_raw(b"Software\\Microsoft\\Windows\\CurrentVersion\\Run\0".as_ptr());
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

            result == ERROR_SUCCESS
        }
    }

    fn open_path(&self, path: &str) -> Result<()> {
        let wide: Vec<u16> = path.encode_utf16().chain(once(0)).collect();
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR::from_raw(wide.as_ptr()),
                None,
                None,
                SHOW_WINDOW_CMD(1),
            );
        }
        Ok(())
    }

    /*
      Gets the process name from a provided process id.
    */
    fn process_name_from_pid(&self, pid: u32) -> Result<String> {
        unsafe {
            // A holding buffer for the process name.
            let mut buffer = [0u16; MAX_PATH as usize];
            let mut size = buffer.len() as u32;

            let process_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
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
            let buffer_path = String::from_utf16_lossy(&buffer[..size as usize]);
            let split_name = buffer_path
                .rsplit('\\')
                .next()
                .and_then(|s| s.split('.').next());

            if let Some(file_name) = split_name {
                Ok(file_name.to_owned())
            } else {
                Err(anyhow!("Failed to get application name."))
            }
        }
    }
}

fn get_window_class_name(hwnd: HWND) -> Option<String> {
    let mut class_name = [0u16; MAX_PATH as usize];

    unsafe {
        let length = GetClassNameW(hwnd, &mut class_name);

        if length == 0 {
            return None;
        }

        String::from_utf16_lossy(&class_name[..length as usize])
            .trim_end_matches('\0')
            .to_string()
            .into()
    }
}

fn find_window_by_class(target_class: &str) -> Result<Option<HWND>> {
    struct SearchState<'a> {
        target_class: &'a str,
        found_hwnd: Option<HWND>,
    }

    unsafe extern "system" fn enum_child_windows_proc(child_hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        if let Some(class_name) = get_window_class_name(child_hwnd) {
            if class_name == state.target_class {
                state.found_hwnd = Some(child_hwnd);

                return false.into();
            }
        }
        true.into()
    }

    unsafe extern "system" fn enum_windows_proc(parent_hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut SearchState);
        let _ = EnumChildWindows(Some(parent_hwnd), Some(enum_child_windows_proc), lparam);
        (state.found_hwnd.is_none()).into()
    }

    fn find_topmost_parent(hwnd: HWND) -> Option<HWND> {
        unsafe {
            let current_hwnd = windows::Win32::UI::WindowsAndMessaging::GetParent(hwnd).ok()?;

            Some(current_hwnd)
        }
    }

    let mut state = SearchState {
        target_class,
        found_hwnd: None,
    };

    unsafe {
        _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut state as *mut _ as isize),
        );
    }

    if let Some(found_hwnd) = state.found_hwnd {
        Ok(find_topmost_parent(found_hwnd))
    } else {
        Ok(None)
    }
}
